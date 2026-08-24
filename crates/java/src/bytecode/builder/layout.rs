//! Fixed-point layout and relaxation for symbolic JVM instructions.

use super::super::encode::encoded_size_at;
use super::super::{Instruction, Opcode, Operand, encode};
use super::model::{
    BranchForm, BuiltCode, CatchTarget, PendingExceptionHandler, PendingInstruction,
    PendingInstructionKind,
};
use crate::classfile::{CATCH_ALL_EXCEPTION_INDEX, ExceptionHandler, MAX_CODE_LENGTH};
use crate::{Error, Result};

const SHORT_BRANCH_WIDTH: usize = 3;
const WIDE_BRANCH_WIDTH: usize = 5;
const EXPANDED_CONDITIONAL_WIDTH: usize = SHORT_BRANCH_WIDTH + WIDE_BRANCH_WIDTH;
const SWITCH_ALIGNMENT: usize = size_of::<i32>();
const TABLE_SWITCH_FIXED_WIDTH: usize = 1 + 3 * size_of::<i32>();
const LOOKUP_SWITCH_FIXED_WIDTH: usize = 1 + 2 * size_of::<i32>();
const LOOKUP_SWITCH_PAIR_WIDTH: usize = 2 * size_of::<i32>();

pub(super) fn finish(
    scope: u64,
    mut pending: Vec<PendingInstruction>,
    bindings: &[Option<usize>],
    handlers: &[PendingExceptionHandler],
) -> Result<BuiltCode> {
    if pending.is_empty() {
        return Err(Error::invalid_assembly(
            "symbolic method body must contain at least one instruction",
        ));
    }
    ensure_all_labels_bound(bindings)?;
    relax(&mut pending, bindings)?;
    let (item_offsets, code_length) = item_offsets(&pending)?;
    if code_length > MAX_CODE_LENGTH {
        return Err(Error::invalid_assembly(format!(
            "method bytecode length {code_length} exceeds {MAX_CODE_LENGTH} bytes"
        )));
    }
    let label_offsets = resolve_labels(bindings, &item_offsets, code_length)?;
    let instructions = materialize(&pending, &item_offsets, &label_offsets)?;
    let code = encode(&instructions)?;
    let exception_table = resolve_handlers(handlers, &label_offsets, code_length)?;
    Ok(BuiltCode {
        scope,
        code,
        instructions,
        instruction_offsets: item_offsets,
        label_offsets,
        exception_table,
    })
}

fn ensure_all_labels_bound(bindings: &[Option<usize>]) -> Result<()> {
    if let Some(index) = bindings.iter().position(Option::is_none) {
        Err(Error::invalid_assembly(format!(
            "symbolic bytecode label {index} was never bound"
        )))
    } else {
        Ok(())
    }
}

fn relax(pending: &mut [PendingInstruction], bindings: &[Option<usize>]) -> Result<()> {
    loop {
        let (offsets, code_length) = item_offsets(pending)?;
        let labels = resolve_labels(bindings, &offsets, code_length)?;
        let mut changed = false;
        for (position, instruction) in pending.iter_mut().enumerate() {
            let PendingInstructionKind::Branch {
                opcode,
                target,
                form,
            } = &mut instruction.kind
            else {
                continue;
            };
            let delta = branch_delta(offsets[position], labels[target.index])?;
            match *form {
                BranchForm::Short if i16::try_from(delta).is_err() => {
                    *form = if opcode.is_conditional_branch() {
                        BranchForm::ExpandedConditional
                    } else {
                        BranchForm::Wide
                    };
                    changed = true;
                }
                BranchForm::Short | BranchForm::Wide | BranchForm::ExpandedConditional => {}
            }
        }
        if !changed {
            return Ok(());
        }
    }
}

fn item_offsets(pending: &[PendingInstruction]) -> Result<(Vec<usize>, usize)> {
    let mut offsets = Vec::with_capacity(pending.len());
    let mut offset = 0_usize;
    for instruction in pending {
        offsets.push(offset);
        let width = pending_width(instruction, offset)?;
        offset = offset
            .checked_add(width)
            .ok_or_else(|| Error::invalid_assembly("method bytecode layout overflows usize"))?;
    }
    Ok((offsets, offset))
}

fn pending_width(instruction: &PendingInstruction, offset: usize) -> Result<usize> {
    match &instruction.kind {
        PendingInstructionKind::Plain { opcode, operand } => {
            let instruction = normalize_plain(offset, *opcode, operand.clone())?;
            encoded_size_at(&instruction, offset)
        }
        PendingInstructionKind::Branch { form, .. } => Ok(match form {
            BranchForm::Short => SHORT_BRANCH_WIDTH,
            BranchForm::Wide => WIDE_BRANCH_WIDTH,
            BranchForm::ExpandedConditional => EXPANDED_CONDITIONAL_WIDTH,
        }),
        PendingInstructionKind::TableSwitch { targets, .. } => switch_width(
            offset,
            TABLE_SWITCH_FIXED_WIDTH,
            size_of::<i32>(),
            targets.len(),
        ),
        PendingInstructionKind::LookupSwitch { pairs, .. } => switch_width(
            offset,
            LOOKUP_SWITCH_FIXED_WIDTH,
            LOOKUP_SWITCH_PAIR_WIDTH,
            pairs.len(),
        ),
    }
}

fn switch_width(offset: usize, fixed: usize, entry: usize, count: usize) -> Result<usize> {
    let after_opcode = offset
        .checked_add(1)
        .ok_or_else(|| Error::invalid_assembly("switch offset overflows usize"))?;
    let padding = (SWITCH_ALIGNMENT - after_opcode % SWITCH_ALIGNMENT) % SWITCH_ALIGNMENT;
    count
        .checked_mul(entry)
        .and_then(|entries| fixed.checked_add(padding)?.checked_add(entries))
        .ok_or_else(|| Error::invalid_assembly("switch layout overflows usize"))
}

fn resolve_labels(
    bindings: &[Option<usize>],
    item_offsets: &[usize],
    code_length: usize,
) -> Result<Vec<usize>> {
    bindings
        .iter()
        .enumerate()
        .map(|(label, binding)| {
            let position = binding.ok_or_else(|| {
                Error::invalid_assembly(format!("symbolic bytecode label {label} is unbound"))
            })?;
            if position == item_offsets.len() {
                Ok(code_length)
            } else {
                item_offsets.get(position).copied().ok_or_else(|| {
                    Error::invalid_assembly(format!(
                        "symbolic bytecode label {label} has invalid item position {position}"
                    ))
                })
            }
        })
        .collect()
}

fn materialize(
    pending: &[PendingInstruction],
    offsets: &[usize],
    labels: &[usize],
) -> Result<Vec<Instruction>> {
    let mut output = Vec::with_capacity(pending.len());
    for (position, pending) in pending.iter().enumerate() {
        let offset = offsets[position];
        match &pending.kind {
            PendingInstructionKind::Plain { opcode, operand } => {
                let mut instruction = normalize_plain(offset, *opcode, operand.clone())?;
                instruction.size = encoded_size_at(&instruction, offset)?;
                output.push(instruction);
            }
            PendingInstructionKind::Branch {
                opcode,
                target,
                form,
            } => materialize_branch(&mut output, offset, *opcode, labels[target.index], *form)?,
            PendingInstructionKind::TableSwitch {
                default,
                low,
                targets,
            } => {
                let mut instruction = Instruction::new(
                    offset,
                    Opcode::TableSwitch,
                    Operand::TableSwitch {
                        default: target_i32(labels[default.index])?,
                        low: *low,
                        targets: targets
                            .iter()
                            .map(|target| target_i32(labels[target.index]))
                            .collect::<Result<_>>()?,
                    },
                );
                instruction.size = pending_width(pending, offset)?;
                output.push(instruction);
            }
            PendingInstructionKind::LookupSwitch { default, pairs } => {
                let mut instruction = Instruction::new(
                    offset,
                    Opcode::LookupSwitch,
                    Operand::LookupSwitch {
                        default: target_i32(labels[default.index])?,
                        pairs: pairs
                            .iter()
                            .map(|(key, target)| Ok((*key, target_i32(labels[target.index])?)))
                            .collect::<Result<_>>()?,
                    },
                );
                instruction.size = pending_width(pending, offset)?;
                output.push(instruction);
            }
        }
    }
    Ok(output)
}

fn materialize_branch(
    output: &mut Vec<Instruction>,
    offset: usize,
    opcode: Opcode,
    target: usize,
    form: BranchForm,
) -> Result<()> {
    match form {
        BranchForm::Short => output.push(sized_branch(offset, opcode, target, SHORT_BRANCH_WIDTH)?),
        BranchForm::Wide => {
            let opcode = match opcode {
                Opcode::Goto | Opcode::GotoW => Opcode::GotoW,
                Opcode::Jsr | Opcode::JsrW => Opcode::JsrW,
                _ => {
                    return Err(Error::invalid_assembly(format!(
                        "{} has no wide branch encoding",
                        opcode.mnemonic()
                    )));
                }
            };
            output.push(sized_branch(offset, opcode, target, WIDE_BRANCH_WIDTH)?);
        }
        BranchForm::ExpandedConditional => {
            let inverted = opcode.inverted_conditional().ok_or_else(|| {
                Error::invalid_assembly(format!(
                    "{} cannot be expanded as a conditional branch",
                    opcode.mnemonic()
                ))
            })?;
            let fallthrough = offset
                .checked_add(EXPANDED_CONDITIONAL_WIDTH)
                .ok_or_else(|| {
                    Error::invalid_assembly("conditional fallthrough overflows usize")
                })?;
            output.push(sized_branch(
                offset,
                inverted,
                fallthrough,
                SHORT_BRANCH_WIDTH,
            )?);
            output.push(sized_branch(
                offset + SHORT_BRANCH_WIDTH,
                Opcode::GotoW,
                target,
                WIDE_BRANCH_WIDTH,
            )?);
        }
    }
    Ok(())
}

fn sized_branch(offset: usize, opcode: Opcode, target: usize, size: usize) -> Result<Instruction> {
    let mut instruction = Instruction::new(offset, opcode, Operand::Branch(target_i32(target)?));
    instruction.size = size;
    Ok(instruction)
}

fn normalize_plain(offset: usize, mut opcode: Opcode, operand: Operand) -> Result<Instruction> {
    if matches!(
        operand,
        Operand::Branch(_) | Operand::TableSwitch { .. } | Operand::LookupSwitch { .. }
    ) {
        return Err(Error::invalid_assembly(
            "raw control-flow operands cannot be emitted through CodeBuilder::emit",
        ));
    }
    if opcode == Opcode::Wide
        || opcode.is_conditional_branch()
        || opcode.is_unconditional_branch()
        || opcode.is_switch()
    {
        return Err(Error::invalid_assembly(format!(
            "{} requires a symbolic CodeBuilder control-flow method",
            opcode.mnemonic()
        )));
    }
    if opcode == Opcode::Ldc
        && matches!(operand, Operand::Constant(index) if index > u16::from(u8::MAX))
    {
        opcode = Opcode::LdcW;
    }
    let wide = match operand {
        Operand::Local(index) => index > u16::from(u8::MAX),
        Operand::Increment { index, value } => {
            index > u16::from(u8::MAX) || i8::try_from(value).is_err()
        }
        _ => false,
    };
    Ok(if wide {
        Instruction::new_wide(offset, opcode, operand)
    } else {
        Instruction::new(offset, opcode, operand)
    })
}

fn resolve_handlers(
    handlers: &[PendingExceptionHandler],
    labels: &[usize],
    code_length: usize,
) -> Result<Vec<ExceptionHandler>> {
    handlers
        .iter()
        .map(|handler| {
            let start = labels[handler.start.index];
            let end = labels[handler.end.index];
            let target = labels[handler.handler.index];
            if start >= end {
                return Err(Error::invalid_assembly(format!(
                    "exception range {start}..{end} is empty or reversed"
                )));
            }
            if end > code_length || target >= code_length {
                return Err(Error::invalid_assembly(
                    "exception range or handler lies outside method bytecode",
                ));
            }
            Ok(ExceptionHandler {
                start_pc: offset_u16(start)?,
                end_pc: offset_u16(end)?,
                handler_pc: offset_u16(target)?,
                catch_type: match handler.catch {
                    CatchTarget::Any => CATCH_ALL_EXCEPTION_INDEX,
                    CatchTarget::Class(index) => index,
                },
            })
        })
        .collect()
}

fn branch_delta(source: usize, target: usize) -> Result<i64> {
    let source = i64::try_from(source)
        .map_err(|_| Error::invalid_assembly("branch source exceeds signed address space"))?;
    let target = i64::try_from(target)
        .map_err(|_| Error::invalid_assembly("branch target exceeds signed address space"))?;
    Ok(target - source)
}

fn target_i32(target: usize) -> Result<i32> {
    i32::try_from(target)
        .map_err(|_| Error::invalid_assembly("branch target exceeds signed 32-bit address space"))
}

fn offset_u16(offset: usize) -> Result<u16> {
    u16::try_from(offset)
        .map_err(|_| Error::invalid_assembly("metadata offset exceeds unsigned 16-bit range"))
}

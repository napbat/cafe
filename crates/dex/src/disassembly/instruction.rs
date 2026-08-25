//! DEX instruction, operand, reference, payload, and flow lifting.

use std::collections::BTreeMap;

use disassembler::{
    CodeAddress, CodeSize, ExactText, ExceptionBehavior, Immediate,
    Instruction as SharedInstruction, InstructionFlow, Operand as SharedOperand, Reference,
    ReferenceKind, ReferenceSymbol, SwitchCase, SwitchTable,
};

use crate::file::{
    CallSiteIndex, DexFile, FieldIndex, MethodHandleIndex, MethodIndex, PrototypeIndex,
    StringIndex, TypeIndex,
};
use crate::instruction::{
    ArrayDataPayload, IndexKind, Instruction, InstructionData, Opcode, Operands,
    PackedSwitchPayload, PayloadKind, SparseSwitchPayload,
};
use crate::{Error, Result};

pub(super) struct Payloads<'a> {
    packed: BTreeMap<u32, &'a PackedSwitchPayload>,
    sparse: BTreeMap<u32, &'a SparseSwitchPayload>,
}

impl<'a> Payloads<'a> {
    pub(super) fn new(instructions: &'a [Instruction]) -> Result<Self> {
        let mut payloads = Self {
            packed: BTreeMap::new(),
            sparse: BTreeMap::new(),
        };
        for instruction in instructions {
            let duplicate = match instruction.data() {
                InstructionData::PackedSwitchPayload(payload) => payloads
                    .packed
                    .insert(instruction.offset(), payload)
                    .is_some(),
                InstructionData::SparseSwitchPayload(payload) => payloads
                    .sparse
                    .insert(instruction.offset(), payload)
                    .is_some(),
                InstructionData::Operation { .. } | InstructionData::ArrayDataPayload(_) => false,
            };
            if duplicate {
                return Err(Error::invalid_instruction(
                    instruction.offset(),
                    "duplicate payload address",
                ));
            }
        }
        Ok(payloads)
    }

    fn switch_table(&self, opcode: Opcode, source: u32, target: u32) -> Result<SwitchTable> {
        let default = source
            .checked_add(opcode.format().code_units())
            .ok_or_else(|| {
                Error::invalid_instruction(source, "switch fallthrough address overflowed")
            })?;
        let cases = match opcode {
            Opcode::PackedSwitch => {
                let payload = self.packed.get(&target).ok_or_else(|| {
                    Error::invalid_instruction(source, "packed-switch payload is missing")
                })?;
                payload
                    .targets
                    .iter()
                    .enumerate()
                    .map(|(position, target)| {
                        let position = i64::try_from(position).map_err(|_| {
                            Error::invalid_instruction(source, "packed-switch key overflowed")
                        })?;
                        Ok(SwitchCase {
                            key: i64::from(payload.first_key) + position,
                            target: CodeAddress::from(*target),
                        })
                    })
                    .collect::<Result<Vec<_>>>()?
            }
            Opcode::SparseSwitch => {
                let payload = self.sparse.get(&target).ok_or_else(|| {
                    Error::invalid_instruction(source, "sparse-switch payload is missing")
                })?;
                if payload.keys.len() != payload.targets.len() {
                    return Err(Error::invalid_instruction(
                        source,
                        "sparse-switch key and target counts differ",
                    ));
                }
                payload
                    .keys
                    .iter()
                    .zip(&payload.targets)
                    .map(|(key, target)| SwitchCase {
                        key: i64::from(*key),
                        target: CodeAddress::from(*target),
                    })
                    .collect()
            }
            _ => {
                return Err(Error::invalid_instruction(
                    source,
                    "non-switch opcode requested a switch table",
                ));
            }
        };
        Ok(SwitchTable {
            default: CodeAddress::from(default),
            cases,
        })
    }
}

pub(super) fn lift_instruction(
    instruction: &Instruction,
    file: &DexFile,
    payloads: &Payloads<'_>,
) -> Result<SharedInstruction> {
    let address = CodeAddress::from(instruction.offset());
    let size = CodeSize::new(instruction.code_units().ok_or_else(|| {
        Error::invalid_instruction(instruction.offset(), "instruction width overflowed")
    })?);
    match instruction.data() {
        InstructionData::Operation { opcode, operands } => {
            let lifted_operands =
                lift_operands(*opcode, operands, instruction.offset(), file, payloads)?;
            let flow = lift_flow(*opcode, operands, instruction.offset(), payloads)?;
            let exception_behavior = ExceptionBehavior::from_may_throw(
                crate::analysis::instruction_semantics(instruction)?.may_throw,
            );
            Ok(SharedInstruction::new(
                address,
                size,
                u32::from(opcode.byte()),
                opcode.mnemonic(),
                lifted_operands,
                flow,
            )
            .with_exception_behavior(exception_behavior))
        }
        InstructionData::PackedSwitchPayload(payload) => Ok(SharedInstruction::new(
            address,
            size,
            u32::from(PayloadKind::PackedSwitch.identifier()),
            PayloadKind::PackedSwitch.mnemonic(),
            packed_payload_operands(payload),
            InstructionFlow::IndirectBranch,
        )
        .with_exception_behavior(ExceptionBehavior::CannotThrow)),
        InstructionData::SparseSwitchPayload(payload) => Ok(SharedInstruction::new(
            address,
            size,
            u32::from(PayloadKind::SparseSwitch.identifier()),
            PayloadKind::SparseSwitch.mnemonic(),
            sparse_payload_operands(payload),
            InstructionFlow::IndirectBranch,
        )
        .with_exception_behavior(ExceptionBehavior::CannotThrow)),
        InstructionData::ArrayDataPayload(payload) => Ok(SharedInstruction::new(
            address,
            size,
            u32::from(PayloadKind::ArrayData.identifier()),
            PayloadKind::ArrayData.mnemonic(),
            array_payload_operands(payload),
            InstructionFlow::IndirectBranch,
        )
        .with_exception_behavior(ExceptionBehavior::CannotThrow)),
    }
}

fn lift_operands(
    opcode: Opcode,
    operands: &Operands,
    source: u32,
    file: &DexFile,
    payloads: &Payloads<'_>,
) -> Result<Vec<SharedOperand>> {
    let lifted = match operands {
        Operands::None => Vec::new(),
        Operands::Register(register) => vec![register_operand(*register)],
        Operands::Registers { first, second } => {
            vec![register_operand(*first), register_operand(*second)]
        }
        Operands::ThreeRegisters {
            first,
            second,
            third,
        } => vec![
            register_operand(*first),
            register_operand(*second),
            register_operand(*third),
        ],
        Operands::RegisterLiteral { register, literal } => {
            vec![register_operand(*register), signed(*literal)]
        }
        Operands::RegistersLiteral {
            first,
            second,
            literal,
        } => vec![
            register_operand(*first),
            register_operand(*second),
            signed(*literal),
        ],
        Operands::Branch { target } => vec![branch_operand(*target)],
        Operands::RegisterBranch { register, target } if opcode.is_switch() => {
            let table = payloads.switch_table(opcode, source, *target)?;
            vec![
                register_operand(*register),
                branch_operand(*target),
                SharedOperand::Switch(table),
            ]
        }
        Operands::RegisterBranch { register, target } => {
            vec![register_operand(*register), branch_operand(*target)]
        }
        Operands::RegistersBranch {
            first,
            second,
            target,
        } => vec![
            register_operand(*first),
            register_operand(*second),
            branch_operand(*target),
        ],
        Operands::RegisterIndex { register, index } => vec![
            register_operand(*register),
            lift_reference(opcode, *index, source, file)?,
        ],
        Operands::RegistersIndex {
            first,
            second,
            index,
        } => vec![
            register_operand(*first),
            register_operand(*second),
            lift_reference(opcode, *index, source, file)?,
        ],
        Operands::RegisterListIndex {
            registers,
            index,
            secondary_index,
        } => {
            let mut values = registers
                .iter()
                .copied()
                .map(register_operand)
                .collect::<Vec<_>>();
            values.push(lift_reference(opcode, *index, source, file)?);
            if let Some(secondary) = secondary_index {
                values.push(prototype_reference(*secondary, file)?);
            }
            values
        }
        Operands::RegisterRangeIndex {
            start,
            count,
            index,
            secondary_index,
        } => {
            let mut values = vec![SharedOperand::RegisterRange {
                start: u32::from(*start),
                count: u32::from(*count),
            }];
            values.push(lift_reference(opcode, *index, source, file)?);
            if let Some(secondary) = secondary_index {
                values.push(prototype_reference(*secondary, file)?);
            }
            values
        }
    };
    Ok(lifted)
}

fn lift_flow(
    opcode: Opcode,
    operands: &Operands,
    source: u32,
    payloads: &Payloads<'_>,
) -> Result<InstructionFlow> {
    if opcode.is_conditional_branch() {
        return Ok(InstructionFlow::ConditionalBranch {
            target: CodeAddress::from(branch_target(operands, source)?),
        });
    }
    if opcode.is_unconditional_branch() {
        return Ok(InstructionFlow::UnconditionalBranch {
            target: CodeAddress::from(branch_target(operands, source)?),
        });
    }
    if opcode.is_switch() {
        let payload = branch_target(operands, source)?;
        let table = payloads.switch_table(opcode, source, payload)?;
        return Ok(InstructionFlow::Switch {
            default: table.default,
            cases: table.cases,
        });
    }
    if opcode.is_return() {
        return Ok(InstructionFlow::Return);
    }
    if opcode == Opcode::Throw {
        return Ok(InstructionFlow::Throw);
    }
    Ok(InstructionFlow::FallThrough)
}

fn branch_target(operands: &Operands, source: u32) -> Result<u32> {
    match operands {
        Operands::Branch { target }
        | Operands::RegisterBranch { target, .. }
        | Operands::RegistersBranch { target, .. } => Ok(*target),
        _ => Err(Error::invalid_instruction(
            source,
            "branch opcode lacks a branch operand",
        )),
    }
}

fn lift_reference(
    opcode: Opcode,
    index: u32,
    source: u32,
    file: &DexFile,
) -> Result<SharedOperand> {
    let kind = opcode
        .index_kind()
        .ok_or_else(|| Error::invalid_instruction(source, "opcode lacks an index kind"))?;
    let reference_kind = match kind {
        IndexKind::String => ReferenceKind::String,
        IndexKind::Type => ReferenceKind::Type,
        IndexKind::Field => ReferenceKind::Field,
        IndexKind::Method if opcode.is_interface_invoke() => ReferenceKind::InterfaceMethod,
        IndexKind::Method => ReferenceKind::Method,
        IndexKind::Prototype => ReferenceKind::MethodPrototype,
        IndexKind::CallSite => ReferenceKind::DynamicCallSite,
        IndexKind::MethodHandle => ReferenceKind::MethodHandle,
    };
    let display = match kind {
        IndexKind::String => Some(file.resolve_string(StringIndex::new(index))?.text.clone()),
        IndexKind::Type => Some(file.type_descriptor(TypeIndex::new(index))?.to_owned()),
        IndexKind::Field => Some(file.resolve_field(FieldIndex::new(index))?.to_string()),
        IndexKind::Method => Some(file.resolve_method(MethodIndex::new(index))?.to_string()),
        IndexKind::Prototype => Some(file.prototype_descriptor(PrototypeIndex::new(index))?),
        IndexKind::CallSite => {
            file.resolve_call_site(CallSiteIndex::new(index))?;
            None
        }
        IndexKind::MethodHandle => Some(
            file.resolve_method_handle(MethodHandleIndex::new(index))?
                .to_string(),
        ),
    };
    let reference = match display {
        Some(display) => Reference::resolved(reference_kind, index, display),
        None => Reference::unresolved(reference_kind, index),
    };
    Ok(SharedOperand::Reference(
        reference_symbol(kind, index, file)?
            .map_or(reference.clone(), |symbol| reference.with_symbol(symbol)),
    ))
}

fn prototype_reference(index: u32, file: &DexFile) -> Result<SharedOperand> {
    let descriptor = file.prototype_descriptor(PrototypeIndex::new(index))?;
    Ok(SharedOperand::Reference(
        Reference::resolved(ReferenceKind::MethodPrototype, index, descriptor.clone())
            .with_symbol(ReferenceSymbol::MethodPrototype(descriptor)),
    ))
}

fn reference_symbol(
    kind: IndexKind,
    index: u32,
    file: &DexFile,
) -> Result<Option<ReferenceSymbol>> {
    let symbol = match kind {
        IndexKind::String => {
            let value = file.resolve_string(StringIndex::new(index))?;
            Some(ReferenceSymbol::String(ExactText {
                text: value.text.clone(),
                utf16_units: value.utf16_units.clone(),
            }))
        }
        IndexKind::Type => Some(ReferenceSymbol::Type(
            file.type_descriptor(TypeIndex::new(index))?.to_owned(),
        )),
        IndexKind::Field => {
            let field = file.resolve_field_id(FieldIndex::new(index))?;
            let name = file.resolve_string(field.name)?;
            Some(ReferenceSymbol::Field {
                owner: file.type_descriptor(field.class)?.to_owned(),
                name: ExactText {
                    text: name.text.clone(),
                    utf16_units: name.utf16_units.clone(),
                },
                descriptor: file.type_descriptor(field.field_type)?.to_owned(),
            })
        }
        IndexKind::Method => {
            let method = file.resolve_method_id(MethodIndex::new(index))?;
            let name = file.resolve_string(method.name)?;
            Some(ReferenceSymbol::Method {
                owner: file.type_descriptor(method.class)?.to_owned(),
                name: ExactText {
                    text: name.text.clone(),
                    utf16_units: name.utf16_units.clone(),
                },
                descriptor: file.prototype_descriptor(method.prototype)?,
            })
        }
        IndexKind::Prototype => Some(ReferenceSymbol::MethodPrototype(
            file.prototype_descriptor(PrototypeIndex::new(index))?,
        )),
        IndexKind::CallSite | IndexKind::MethodHandle => None,
    };
    Ok(symbol)
}

fn packed_payload_operands(payload: &PackedSwitchPayload) -> Vec<SharedOperand> {
    std::iter::once(signed(i64::from(payload.first_key)))
        .chain(payload.targets.iter().copied().map(branch_operand))
        .collect()
}

fn sparse_payload_operands(payload: &SparseSwitchPayload) -> Vec<SharedOperand> {
    payload
        .keys
        .iter()
        .copied()
        .zip(payload.targets.iter().copied())
        .flat_map(|(key, target)| [signed(i64::from(key)), branch_operand(target)])
        .collect()
}

fn array_payload_operands(payload: &ArrayDataPayload) -> Vec<SharedOperand> {
    vec![
        unsigned(u64::from(payload.element_width)),
        unsigned(u64::from(payload.element_count)),
        SharedOperand::Data(payload.data.clone()),
    ]
}

fn register_operand(register: u16) -> SharedOperand {
    SharedOperand::Register(u32::from(register))
}

fn branch_operand(target: u32) -> SharedOperand {
    SharedOperand::BranchTarget(CodeAddress::from(target))
}

fn signed(value: i64) -> SharedOperand {
    SharedOperand::Immediate(Immediate::Signed(value))
}

fn unsigned(value: u64) -> SharedOperand {
    SharedOperand::Immediate(Immediate::Unsigned(value))
}

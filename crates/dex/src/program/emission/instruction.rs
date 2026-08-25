//! Reverse lowering from shared instructions to native Dalvik operations.

use std::collections::HashMap;

use disassembler::{
    Immediate, Instruction as SharedInstruction, Operand as SharedOperand, Reference,
};

use super::{DexEmissionError, DexReferenceHandle};
use crate::file::DexIndices;
use crate::instruction::{
    ArrayDataPayload, Instruction, InstructionFormat, Opcode, Operands, PackedSwitchPayload,
    PayloadKind, SparseSwitchPayload,
};

pub(super) fn lower_instructions(
    instructions: &[SharedInstruction],
    references: &HashMap<Reference, DexReferenceHandle>,
    indices: &DexIndices,
    class: &str,
    method: &str,
    descriptor: &str,
) -> Result<Vec<Instruction>, DexEmissionError> {
    instructions
        .iter()
        .map(|instruction| {
            lower_instruction(instruction, references, indices).map_err(|message| {
                DexEmissionError::Instruction {
                    class: class.to_owned(),
                    method: method.to_owned(),
                    descriptor: descriptor.to_owned(),
                    address: instruction.address,
                    message,
                }
            })
        })
        .collect()
}

fn lower_instruction(
    instruction: &SharedInstruction,
    references: &HashMap<Reference, DexReferenceHandle>,
    indices: &DexIndices,
) -> Result<Instruction, String> {
    let offset = u32::try_from(instruction.address.get())
        .map_err(|_| "address exceeds the DEX code-unit range".to_owned())?;
    if instruction.opcode == u32::from(PayloadKind::PackedSwitch.identifier()) {
        return packed_payload(instruction, offset);
    }
    if instruction.opcode == u32::from(PayloadKind::SparseSwitch.identifier()) {
        return sparse_payload(instruction, offset);
    }
    if instruction.opcode == u32::from(PayloadKind::ArrayData.identifier()) {
        return array_payload(instruction, offset);
    }
    let opcode = u8::try_from(instruction.opcode)
        .ok()
        .and_then(Opcode::from_byte)
        .ok_or_else(|| "unknown DEX opcode".to_owned())?;
    let operands = operation_operands(instruction, opcode, references, indices)?;
    let native = Instruction::operation(offset, opcode, operands);
    require_size(instruction, &native)?;
    Ok(native)
}

fn operation_operands(
    instruction: &SharedInstruction,
    opcode: Opcode,
    references: &HashMap<Reference, DexReferenceHandle>,
    indices: &DexIndices,
) -> Result<Operands, String> {
    Ok(match opcode.format() {
        InstructionFormat::F10x => {
            count(instruction, 0)?;
            Operands::None
        }
        InstructionFormat::F12x | InstructionFormat::F22x | InstructionFormat::F32x => {
            Operands::Registers {
                first: register(instruction, 0)?,
                second: register(instruction, 1)?,
            }
        }
        InstructionFormat::F11n
        | InstructionFormat::F21s
        | InstructionFormat::F21h
        | InstructionFormat::F31i
        | InstructionFormat::F51l => Operands::RegisterLiteral {
            register: register(instruction, 0)?,
            literal: signed(instruction, 1)?,
        },
        InstructionFormat::F11x => Operands::Register(register(instruction, 0)?),
        InstructionFormat::F10t | InstructionFormat::F20t | InstructionFormat::F30t => {
            Operands::Branch {
                target: target(instruction, 0)?,
            }
        }
        InstructionFormat::F21t | InstructionFormat::F31t => Operands::RegisterBranch {
            register: register(instruction, 0)?,
            target: target(instruction, 1)?,
        },
        InstructionFormat::F21c | InstructionFormat::F31c => Operands::RegisterIndex {
            register: register(instruction, 0)?,
            index: reference_index(instruction, 1, opcode, references, indices)?,
        },
        InstructionFormat::F23x => Operands::ThreeRegisters {
            first: register(instruction, 0)?,
            second: register(instruction, 1)?,
            third: register(instruction, 2)?,
        },
        InstructionFormat::F22t => Operands::RegistersBranch {
            first: register(instruction, 0)?,
            second: register(instruction, 1)?,
            target: target(instruction, 2)?,
        },
        InstructionFormat::F22s | InstructionFormat::F22b => Operands::RegistersLiteral {
            first: register(instruction, 0)?,
            second: register(instruction, 1)?,
            literal: signed(instruction, 2)?,
        },
        InstructionFormat::F22c => Operands::RegistersIndex {
            first: register(instruction, 0)?,
            second: register(instruction, 1)?,
            index: reference_index(instruction, 2, opcode, references, indices)?,
        },
        InstructionFormat::F35c | InstructionFormat::F45cc => {
            let reference_position = instruction
                .operands
                .iter()
                .position(|operand| matches!(operand, SharedOperand::Reference(_)))
                .ok_or_else(|| "register-list instruction lacks a reference".to_owned())?;
            let registers = instruction.operands[..reference_position]
                .iter()
                .map(register_operand)
                .collect::<Result<Vec<_>, _>>()?;
            let index =
                reference_index(instruction, reference_position, opcode, references, indices)?;
            let secondary_index = if opcode.format() == InstructionFormat::F45cc {
                Some(prototype_index(
                    instruction,
                    reference_position + 1,
                    references,
                    indices,
                )?)
            } else {
                None
            };
            Operands::RegisterListIndex {
                registers,
                index,
                secondary_index,
            }
        }
        InstructionFormat::F3rc | InstructionFormat::F4rcc => {
            let (start, count) = range(instruction, 0)?;
            let index = reference_index(instruction, 1, opcode, references, indices)?;
            let secondary_index = if opcode.format() == InstructionFormat::F4rcc {
                Some(prototype_index(instruction, 2, references, indices)?)
            } else {
                None
            };
            Operands::RegisterRangeIndex {
                start,
                count,
                index,
                secondary_index,
            }
        }
    })
}

fn reference_index(
    instruction: &SharedInstruction,
    position: usize,
    opcode: Opcode,
    references: &HashMap<Reference, DexReferenceHandle>,
    indices: &DexIndices,
) -> Result<u32, String> {
    let reference = reference(instruction, position)?;
    let handle = references
        .get(reference)
        .ok_or_else(|| "reference was not interned during the emission prepass".to_owned())?;
    handle.index_for(opcode.index_kind(), indices)
}

fn prototype_index(
    instruction: &SharedInstruction,
    position: usize,
    references: &HashMap<Reference, DexReferenceHandle>,
    indices: &DexIndices,
) -> Result<u32, String> {
    let reference = reference(instruction, position)?;
    let handle = references
        .get(reference)
        .ok_or_else(|| "prototype was not interned during the emission prepass".to_owned())?;
    handle.index_for(Some(crate::instruction::IndexKind::Prototype), indices)
}

fn packed_payload(instruction: &SharedInstruction, offset: u32) -> Result<Instruction, String> {
    let first_key = i32::try_from(signed(instruction, 0)?)
        .map_err(|_| "packed-switch first key exceeds i32".to_owned())?;
    let targets = instruction.operands[1..]
        .iter()
        .map(target_operand)
        .collect::<Result<Vec<_>, _>>()?;
    let native = Instruction::packed_switch(offset, PackedSwitchPayload { first_key, targets });
    require_size(instruction, &native)?;
    Ok(native)
}

fn sparse_payload(instruction: &SharedInstruction, offset: u32) -> Result<Instruction, String> {
    let (pairs, remainder) = instruction.operands.as_chunks::<2>();
    if !remainder.is_empty() {
        return Err("sparse-switch payload requires key/target pairs".to_owned());
    }
    let mut keys = Vec::new();
    let mut targets = Vec::new();
    for pair in pairs {
        keys.push(
            i32::try_from(immediate_operand(&pair[0])?)
                .map_err(|_| "sparse-switch key exceeds i32".to_owned())?,
        );
        targets.push(target_operand(&pair[1])?);
    }
    let native = Instruction::sparse_switch(offset, SparseSwitchPayload { keys, targets });
    require_size(instruction, &native)?;
    Ok(native)
}

fn array_payload(instruction: &SharedInstruction, offset: u32) -> Result<Instruction, String> {
    count(instruction, 3)?;
    let element_width = u16::try_from(unsigned(instruction, 0)?)
        .map_err(|_| "array payload element width exceeds u16".to_owned())?;
    let element_count = u32::try_from(unsigned(instruction, 1)?)
        .map_err(|_| "array payload element count exceeds u32".to_owned())?;
    let SharedOperand::Data(data) = &instruction.operands[2] else {
        return Err("array payload lacks inline data".to_owned());
    };
    let native = Instruction::array_data(
        offset,
        ArrayDataPayload {
            element_width,
            element_count,
            data: data.clone(),
        },
    );
    require_size(instruction, &native)?;
    Ok(native)
}

fn register(instruction: &SharedInstruction, position: usize) -> Result<u16, String> {
    instruction
        .operands
        .get(position)
        .ok_or_else(|| "missing register operand".to_owned())
        .and_then(register_operand)
}

fn register_operand(operand: &SharedOperand) -> Result<u16, String> {
    let SharedOperand::Register(register) = operand else {
        return Err("expected register operand".to_owned());
    };
    u16::try_from(*register).map_err(|_| "register index exceeds u16".to_owned())
}

fn range(instruction: &SharedInstruction, position: usize) -> Result<(u16, u8), String> {
    let Some(SharedOperand::RegisterRange { start, count }) = instruction.operands.get(position)
    else {
        return Err("expected register-range operand".to_owned());
    };
    Ok((
        u16::try_from(*start).map_err(|_| "range start exceeds u16".to_owned())?,
        u8::try_from(*count).map_err(|_| "range count exceeds u8".to_owned())?,
    ))
}

fn signed(instruction: &SharedInstruction, position: usize) -> Result<i64, String> {
    instruction
        .operands
        .get(position)
        .ok_or_else(|| "missing immediate operand".to_owned())
        .and_then(immediate_operand)
}

fn immediate_operand(operand: &SharedOperand) -> Result<i64, String> {
    match operand {
        SharedOperand::Immediate(Immediate::Signed(value)) => Ok(*value),
        SharedOperand::Immediate(Immediate::Unsigned(value)) => {
            i64::try_from(*value).map_err(|_| "immediate exceeds i64".to_owned())
        }
        _ => Err("expected immediate operand".to_owned()),
    }
}

fn unsigned(instruction: &SharedInstruction, position: usize) -> Result<u64, String> {
    match instruction.operands.get(position) {
        Some(SharedOperand::Immediate(Immediate::Unsigned(value))) => Ok(*value),
        Some(SharedOperand::Immediate(Immediate::Signed(value))) => {
            u64::try_from(*value).map_err(|_| "negative unsigned immediate".to_owned())
        }
        _ => Err("expected unsigned immediate operand".to_owned()),
    }
}

fn target(instruction: &SharedInstruction, position: usize) -> Result<u32, String> {
    instruction
        .operands
        .get(position)
        .ok_or_else(|| "missing branch target".to_owned())
        .and_then(target_operand)
}

fn target_operand(operand: &SharedOperand) -> Result<u32, String> {
    let SharedOperand::BranchTarget(target) = operand else {
        return Err("expected branch target".to_owned());
    };
    u32::try_from(target.get()).map_err(|_| "branch target exceeds u32".to_owned())
}

fn reference(instruction: &SharedInstruction, position: usize) -> Result<&Reference, String> {
    match instruction.operands.get(position) {
        Some(SharedOperand::Reference(reference)) => Ok(reference),
        _ => Err("expected symbolic reference operand".to_owned()),
    }
}

fn count(instruction: &SharedInstruction, expected: usize) -> Result<(), String> {
    if instruction.operands.len() == expected {
        Ok(())
    } else {
        Err(format!(
            "expected {expected} operands, found {}",
            instruction.operands.len()
        ))
    }
}

fn require_size(shared: &SharedInstruction, native: &Instruction) -> Result<(), String> {
    let actual = native
        .code_units()
        .ok_or_else(|| "native instruction width overflowed".to_owned())?;
    if u64::from(actual) == u64::from(shared.size.get()) {
        Ok(())
    } else {
        Err(format!(
            "shared size {} differs from native size {actual}",
            shared.size.get()
        ))
    }
}

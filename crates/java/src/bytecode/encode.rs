//! Encoder for typed JVM method bytecode.

use super::{
    INFERRED_INSTRUCTION_SIZE, Instruction, MIN_INVOKE_INTERFACE_COUNT, MIN_MULTI_ARRAY_DIMENSIONS,
    Opcode, Operand, RESERVED_OPERAND_BYTE, RESERVED_OPERAND_WORD, SWITCH_ALIGNMENT, decode,
};
use crate::{Error, Result};

const FIRST_ENCODED_BYTE_INDEX: usize = 0;
const INCLUSIVE_KEY_COUNT_ADJUSTMENT: i64 = 1;

/// Encodes a sequence of typed instructions into JVM bytecode.
///
/// Instruction offsets and absolute branch targets are checked. A nonzero
/// [`Instruction::size`] must equal the resulting encoded size; setting it to
/// zero allows callers constructing new instructions to request inference.
///
/// # Errors
///
/// Returns an error when an operand is incompatible with its opcode, an offset
/// or branch width cannot be represented, or the resulting bytecode is invalid.
pub fn encode(instructions: &[Instruction]) -> Result<Vec<u8>> {
    let mut code = Vec::new();
    for instruction in instructions {
        if instruction.offset != code.len() {
            return Err(Error::invalid_assembly(format!(
                "instruction {} has offset {}, expected {}",
                instruction.opcode.mnemonic(),
                instruction.offset,
                code.len()
            )));
        }
        let start = code.len();
        if instruction.wide {
            code.push(Opcode::Wide.byte());
        }
        code.push(instruction.opcode.byte());
        encode_operand(&mut code, instruction, start)?;
        let encoded_size = code.len() - start;
        if instruction.size != INFERRED_INSTRUCTION_SIZE && instruction.size != encoded_size {
            return Err(Error::invalid_assembly(format!(
                "instruction at offset {start} records size {}, encoded size is {encoded_size}",
                instruction.size
            )));
        }
    }
    decode(&code)?;
    Ok(code)
}

#[allow(clippy::too_many_lines)]
fn encode_operand(code: &mut Vec<u8>, instruction: &Instruction, start: usize) -> Result<()> {
    let opcode = instruction.opcode;
    let invalid = || {
        Error::invalid_assembly(format!(
            "operand {:?} is incompatible with {}{} at offset {start}",
            instruction.operand,
            if instruction.wide { "wide " } else { "" },
            opcode.mnemonic()
        ))
    };

    if instruction.wide
        && !matches!(
            opcode,
            Opcode::ILoad
                | Opcode::LLoad
                | Opcode::FLoad
                | Opcode::DLoad
                | Opcode::ALoad
                | Opcode::IStore
                | Opcode::LStore
                | Opcode::FStore
                | Opcode::DStore
                | Opcode::AStore
                | Opcode::Ret
                | Opcode::IInc
        )
    {
        return Err(invalid());
    }

    match (&instruction.operand, opcode) {
        (Operand::None, opcode) if opcode_has_no_operand(opcode) && !instruction.wide => {}
        (Operand::Byte(value), Opcode::BiPush) if !instruction.wide => {
            code.push(value.to_be_bytes()[FIRST_ENCODED_BYTE_INDEX]);
        }
        (Operand::Short(value), Opcode::SiPush) if !instruction.wide => {
            code.extend_from_slice(&value.to_be_bytes());
        }
        (Operand::Constant(index), Opcode::Ldc) if !instruction.wide => {
            code.push(u8::try_from(*index).map_err(|_| {
                Error::invalid_assembly(format!("ldc constant index #{index} exceeds u8"))
            })?);
        }
        (Operand::Constant(index), opcode)
            if !instruction.wide && opcode_has_u16_constant(opcode) =>
        {
            code.extend_from_slice(&index.to_be_bytes());
        }
        (Operand::Local(index), opcode) if opcode_has_local(opcode) => {
            if instruction.wide {
                code.extend_from_slice(&index.to_be_bytes());
            } else {
                code.push(u8::try_from(*index).map_err(|_| {
                    Error::invalid_assembly(format!("local index {index} requires the wide prefix"))
                })?);
            }
        }
        (Operand::Increment { index, value }, Opcode::IInc) => {
            if instruction.wide {
                code.extend_from_slice(&index.to_be_bytes());
                code.extend_from_slice(&value.to_be_bytes());
            } else {
                code.push(u8::try_from(*index).map_err(|_| {
                    Error::invalid_assembly(format!(
                        "iinc local index {index} requires the wide prefix"
                    ))
                })?);
                let value = i8::try_from(*value).map_err(|_| {
                    Error::invalid_assembly(format!("iinc value {value} requires the wide prefix"))
                })?;
                code.push(value.to_be_bytes()[FIRST_ENCODED_BYTE_INDEX]);
            }
        }
        (Operand::Branch(target), opcode) if opcode_has_short_branch(opcode) => {
            let delta = branch_delta(*target, start)?;
            let delta = i16::try_from(delta).map_err(|_| {
                Error::invalid_assembly(format!(
                    "branch from {start} to {target} exceeds a signed 16-bit offset"
                ))
            })?;
            code.extend_from_slice(&delta.to_be_bytes());
        }
        (Operand::Branch(target), Opcode::GotoW | Opcode::JsrW) => {
            code.extend_from_slice(&branch_delta(*target, start)?.to_be_bytes());
        }
        (
            Operand::TableSwitch {
                default,
                low,
                targets,
            },
            Opcode::TableSwitch,
        ) if !instruction.wide => {
            if targets.is_empty() {
                return Err(Error::invalid_assembly(
                    "tableswitch requires at least one target",
                ));
            }
            write_switch_padding(code);
            code.extend_from_slice(&branch_delta(*default, start)?.to_be_bytes());
            code.extend_from_slice(&low.to_be_bytes());
            let high = i64::from(*low) + i64::try_from(targets.len()).unwrap_or(i64::MAX)
                - INCLUSIVE_KEY_COUNT_ADJUSTMENT;
            let high = i32::try_from(high)
                .map_err(|_| Error::invalid_assembly("tableswitch high key overflows i32"))?;
            code.extend_from_slice(&high.to_be_bytes());
            for &target in targets {
                code.extend_from_slice(&branch_delta(target, start)?.to_be_bytes());
            }
        }
        (Operand::LookupSwitch { default, pairs }, Opcode::LookupSwitch) if !instruction.wide => {
            write_switch_padding(code);
            code.extend_from_slice(&branch_delta(*default, start)?.to_be_bytes());
            let pair_count = i32::try_from(pairs.len()).map_err(|_| {
                Error::invalid_assembly("lookupswitch pair count exceeds signed i32")
            })?;
            code.extend_from_slice(&pair_count.to_be_bytes());
            for &(key, target) in pairs {
                code.extend_from_slice(&key.to_be_bytes());
                code.extend_from_slice(&branch_delta(target, start)?.to_be_bytes());
            }
        }
        (Operand::ArrayType(array_type), Opcode::NewArray) if !instruction.wide => {
            code.push(array_type.byte());
        }
        (Operand::InvokeInterface { index, count }, Opcode::InvokeInterface)
            if !instruction.wide && *count >= MIN_INVOKE_INTERFACE_COUNT =>
        {
            code.extend_from_slice(&index.to_be_bytes());
            code.push(*count);
            code.push(RESERVED_OPERAND_BYTE);
        }
        (Operand::InvokeDynamic(index), Opcode::InvokeDynamic) if !instruction.wide => {
            code.extend_from_slice(&index.to_be_bytes());
            code.extend_from_slice(&RESERVED_OPERAND_WORD.to_be_bytes());
        }
        (Operand::MultiArray { index, dimensions }, Opcode::MultiANewArray)
            if !instruction.wide && *dimensions >= MIN_MULTI_ARRAY_DIMENSIONS =>
        {
            code.extend_from_slice(&index.to_be_bytes());
            code.push(*dimensions);
        }
        _ => return Err(invalid()),
    }
    Ok(())
}

const fn opcode_has_u16_constant(opcode: Opcode) -> bool {
    matches!(
        opcode,
        Opcode::LdcW
            | Opcode::Ldc2W
            | Opcode::GetStatic
            | Opcode::PutStatic
            | Opcode::GetField
            | Opcode::PutField
            | Opcode::InvokeVirtual
            | Opcode::InvokeSpecial
            | Opcode::InvokeStatic
            | Opcode::New
            | Opcode::ANewArray
            | Opcode::CheckCast
            | Opcode::InstanceOf
    )
}

const fn opcode_has_local(opcode: Opcode) -> bool {
    matches!(
        opcode,
        Opcode::ILoad
            | Opcode::LLoad
            | Opcode::FLoad
            | Opcode::DLoad
            | Opcode::ALoad
            | Opcode::IStore
            | Opcode::LStore
            | Opcode::FStore
            | Opcode::DStore
            | Opcode::AStore
            | Opcode::Ret
    )
}

const fn opcode_has_short_branch(opcode: Opcode) -> bool {
    matches!(
        opcode,
        Opcode::IfEq
            | Opcode::IfNe
            | Opcode::IfLt
            | Opcode::IfGe
            | Opcode::IfGt
            | Opcode::IfLe
            | Opcode::IfICmpEq
            | Opcode::IfICmpNe
            | Opcode::IfICmpLt
            | Opcode::IfICmpGe
            | Opcode::IfICmpGt
            | Opcode::IfICmpLe
            | Opcode::IfACmpEq
            | Opcode::IfACmpNe
            | Opcode::Goto
            | Opcode::Jsr
            | Opcode::IfNull
            | Opcode::IfNonNull
    )
}

const fn opcode_has_no_operand(opcode: Opcode) -> bool {
    !matches!(
        opcode,
        Opcode::BiPush
            | Opcode::SiPush
            | Opcode::Ldc
            | Opcode::LdcW
            | Opcode::Ldc2W
            | Opcode::ILoad
            | Opcode::LLoad
            | Opcode::FLoad
            | Opcode::DLoad
            | Opcode::ALoad
            | Opcode::IStore
            | Opcode::LStore
            | Opcode::FStore
            | Opcode::DStore
            | Opcode::AStore
            | Opcode::IInc
            | Opcode::IfEq
            | Opcode::IfNe
            | Opcode::IfLt
            | Opcode::IfGe
            | Opcode::IfGt
            | Opcode::IfLe
            | Opcode::IfICmpEq
            | Opcode::IfICmpNe
            | Opcode::IfICmpLt
            | Opcode::IfICmpGe
            | Opcode::IfICmpGt
            | Opcode::IfICmpLe
            | Opcode::IfACmpEq
            | Opcode::IfACmpNe
            | Opcode::Goto
            | Opcode::Jsr
            | Opcode::Ret
            | Opcode::TableSwitch
            | Opcode::LookupSwitch
            | Opcode::GetStatic
            | Opcode::PutStatic
            | Opcode::GetField
            | Opcode::PutField
            | Opcode::InvokeVirtual
            | Opcode::InvokeSpecial
            | Opcode::InvokeStatic
            | Opcode::InvokeInterface
            | Opcode::InvokeDynamic
            | Opcode::New
            | Opcode::NewArray
            | Opcode::ANewArray
            | Opcode::CheckCast
            | Opcode::InstanceOf
            | Opcode::Wide
            | Opcode::MultiANewArray
            | Opcode::IfNull
            | Opcode::IfNonNull
            | Opcode::GotoW
            | Opcode::JsrW
    )
}

fn branch_delta(target: i32, start: usize) -> Result<i32> {
    let start = i64::try_from(start).unwrap_or(i64::MAX);
    i32::try_from(i64::from(target) - start).map_err(|_| {
        Error::invalid_assembly(format!(
            "branch from {start} to {target} exceeds a signed 32-bit offset"
        ))
    })
}

fn write_switch_padding(code: &mut Vec<u8>) {
    let padding = (SWITCH_ALIGNMENT - (code.len() % SWITCH_ALIGNMENT)) % SWITCH_ALIGNMENT;
    code.resize(code.len() + padding, RESERVED_OPERAND_BYTE);
}

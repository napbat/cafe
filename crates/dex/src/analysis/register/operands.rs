//! Checked register extraction from Dalvik operand variants.

use crate::instruction::Operands;
use crate::{Error, Result};

pub(super) fn first_register(operands: &Operands, offset: u32) -> Result<u16> {
    match operands {
        Operands::Register(register)
        | Operands::RegisterLiteral { register, .. }
        | Operands::RegisterBranch { register, .. }
        | Operands::RegisterIndex { register, .. } => Ok(*register),
        Operands::Registers { first, .. }
        | Operands::ThreeRegisters { first, .. }
        | Operands::RegistersLiteral { first, .. }
        | Operands::RegistersBranch { first, .. }
        | Operands::RegistersIndex { first, .. } => Ok(*first),
        _ => Err(Error::invalid_instruction(
            offset,
            "first register is missing",
        )),
    }
}

pub(super) fn two_registers(operands: &Operands, offset: u32) -> Result<(u16, u16)> {
    match operands {
        Operands::Registers { first, second }
        | Operands::RegistersLiteral { first, second, .. }
        | Operands::RegistersBranch { first, second, .. }
        | Operands::RegistersIndex { first, second, .. } => Ok((*first, *second)),
        _ => Err(Error::invalid_instruction(
            offset,
            "two registers are missing",
        )),
    }
}

pub(super) fn three_registers(operands: &Operands, offset: u32) -> Result<(u16, u16, u16)> {
    let Operands::ThreeRegisters {
        first,
        second,
        third,
    } = operands
    else {
        return Err(Error::invalid_instruction(
            offset,
            "three register operands are missing",
        ));
    };
    Ok((*first, *second, *third))
}

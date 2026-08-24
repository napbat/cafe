//! One instruction in the shared disassembly representation.

use std::borrow::Cow;
use std::fmt;

use cfglib::DisplayInstr;

use super::{CodeAddress, CodeSize, InstructionFlow, Operand};

/// Decoded instruction retaining its native opcode and normalized operands.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Instruction {
    /// Address relative to the start of the function body.
    pub address: CodeAddress,
    /// Encoded width in the function body's address unit.
    pub size: CodeSize,
    /// Native numeric opcode.
    pub opcode: u32,
    /// Native instruction mnemonic.
    pub mnemonic: String,
    /// Structured operands in source order.
    pub operands: Vec<Operand>,
    /// Intraprocedural control-flow effect.
    pub flow: InstructionFlow,
}

impl Instruction {
    /// Creates a shared instruction from format-native decoded values.
    #[must_use]
    pub fn new(
        address: CodeAddress,
        size: CodeSize,
        opcode: u32,
        mnemonic: impl Into<String>,
        operands: Vec<Operand>,
        flow: InstructionFlow,
    ) -> Self {
        Self {
            address,
            size,
            opcode,
            mnemonic: mnemonic.into(),
            operands,
            flow,
        }
    }

    /// Returns the first address after this instruction, if representable.
    #[must_use]
    pub fn checked_end(&self) -> Option<CodeAddress> {
        self.address.checked_add(self.size)
    }
}

impl fmt::Display for Instruction {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.address, self.mnemonic)?;
        for (position, operand) in self.operands.iter().enumerate() {
            if position == 0 {
                formatter.write_str(" ")?;
            } else {
                formatter.write_str(", ")?;
            }
            operand.fmt(formatter)?;
        }
        Ok(())
    }
}

impl DisplayInstr for Instruction {
    fn mnemonic(&self) -> Cow<'_, str> {
        Cow::Owned(self.to_string())
    }
}

//! One instruction in the shared disassembly representation.

use std::borrow::Cow;
use std::fmt;

use cfglib::DisplayInstr;

use super::{CodeAddress, CodeSize, InstructionFlow, Operand};

/// Whether ordinary execution of an instruction can transfer to an exception handler.
///
/// Native frontends should report [`Self::MayThrow`] or [`Self::CannotThrow`]
/// when their instruction semantics are known. [`Self::Unknown`] keeps custom
/// [`crate::DisassemblySource`] implementations conservative: protected
/// instructions with unknown behavior retain exceptional CFG edges.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ExceptionBehavior {
    /// The source did not classify this instruction's exceptional behavior.
    #[default]
    Unknown,
    /// The instruction can transfer to an enclosing exception handler.
    MayThrow,
    /// The instruction cannot transfer to an enclosing exception handler.
    CannotThrow,
}

impl ExceptionBehavior {
    /// Converts an exact native may-throw fact into the shared classification.
    #[must_use]
    pub const fn from_may_throw(may_throw: bool) -> Self {
        if may_throw {
            Self::MayThrow
        } else {
            Self::CannotThrow
        }
    }

    /// Whether CFG construction must conservatively retain an exception edge.
    #[must_use]
    pub const fn retains_exception_edge(self) -> bool {
        !matches!(self, Self::CannotThrow)
    }
}

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
    /// Exceptional control-flow behavior supplied by the native frontend.
    pub exception_behavior: ExceptionBehavior,
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
            exception_behavior: ExceptionBehavior::Unknown,
        }
    }

    /// Sets this instruction's exceptional control-flow behavior.
    #[must_use]
    pub const fn with_exception_behavior(mut self, behavior: ExceptionBehavior) -> Self {
        self.exception_behavior = behavior;
        self
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

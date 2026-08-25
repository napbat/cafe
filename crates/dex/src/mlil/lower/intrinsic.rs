//! Explicit target policy for implementation-defined MLIL intrinsics.

use std::ops::Range;

use ::mlil::{InstructionId, ValueType};

use crate::file::DexFile;
use crate::instruction::{Opcode, Operands};

/// One straight-line Dalvik instruction selected for an MLIL intrinsic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DexIntrinsicInstruction {
    /// Dalvik opcode to emit.
    pub opcode: Opcode,
    /// Typed operands belonging to `opcode`.
    pub operands: Operands,
}

impl DexIntrinsicInstruction {
    /// Creates one policy-selected Dalvik instruction.
    #[must_use]
    pub const fn new(opcode: Opcode, operands: Operands) -> Self {
        Self { opcode, operands }
    }
}

/// Allocated target signature presented to a Dalvik intrinsic policy.
#[derive(Debug, Clone)]
pub struct DexIntrinsicRequest<'a> {
    /// Stable MLIL instruction identity.
    pub instruction: InstructionId,
    /// Stable implementation-defined intrinsic name.
    pub name: &'a str,
    /// Physical registers containing uses in semantic operand order.
    pub use_registers: &'a [u16],
    /// Types paired with `use_registers`.
    pub use_types: &'a [ValueType],
    /// Physical registers that must receive definitions in semantic order.
    pub definition_registers: &'a [u16],
    /// Types paired with `definition_registers`.
    pub definition_types: &'a [ValueType],
    /// Low temporary registers reserved for the expansion and dead afterward.
    pub scratch_registers: Range<u16>,
}

/// Structured failure returned by a Dalvik intrinsic policy.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
pub struct DexIntrinsicLoweringError {
    message: String,
}

impl DexIntrinsicLoweringError {
    /// Creates an intrinsic-policy failure.
    #[must_use]
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

/// Maps implementation-defined MLIL intrinsics to straight-line Dalvik code.
///
/// The request exposes final variable registers and a clobberable low scratch
/// window. The finished body is decoded, analyzed, and verified, so invalid
/// register widths, operand forms, or definition effects are rejected.
/// Control-flow opcodes are rejected because the semantic successors remain
/// owned by the MLIL graph; a may-throw intrinsic must return at least one
/// instruction so its protected native interval is non-empty.
pub trait DexMlilIntrinsicLowerer {
    /// Selects a Dalvik expansion against the explicit target identifier tables.
    ///
    /// # Errors
    ///
    /// Returns an explanation when this policy does not support the intrinsic or
    /// cannot construct its target operands.
    fn lower(
        &mut self,
        request: DexIntrinsicRequest<'_>,
        file: &DexFile,
    ) -> std::result::Result<Vec<DexIntrinsicInstruction>, DexIntrinsicLoweringError>;
}

/// Default policy that requires callers to opt into every intrinsic meaning.
#[derive(Debug, Clone, Copy, Default)]
pub struct RejectDexIntrinsics;

impl DexMlilIntrinsicLowerer for RejectDexIntrinsics {
    fn lower(
        &mut self,
        request: DexIntrinsicRequest<'_>,
        _file: &DexFile,
    ) -> std::result::Result<Vec<DexIntrinsicInstruction>, DexIntrinsicLoweringError> {
        Err(DexIntrinsicLoweringError::new(format!(
            "Dalvik intrinsic policy does not define `{}`",
            request.name
        )))
    }
}

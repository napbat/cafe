//! Explicit target policy for implementation-defined MLIL intrinsics.

use ::mlil::ValueType;

use crate::bytecode::{Opcode, Operand};
use crate::classfile::ConstantPool;

/// One straight-line JVM instruction selected for an MLIL intrinsic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JavaIntrinsicInstruction {
    /// JVM opcode to emit.
    pub opcode: Opcode,
    /// Typed operand belonging to `opcode`.
    pub operand: Operand,
}

impl JavaIntrinsicInstruction {
    /// Creates one policy-selected JVM instruction.
    #[must_use]
    pub const fn new(opcode: Opcode, operand: Operand) -> Self {
        Self { opcode, operand }
    }
}

/// Target-independent signature presented to a JVM intrinsic policy.
#[derive(Debug, Clone, Copy)]
pub struct JavaIntrinsicRequest<'a> {
    /// Stable implementation-defined intrinsic name.
    pub name: &'a str,
    /// Stack values loaded before the expansion, in operand order.
    pub use_types: &'a [ValueType],
    /// Stack values that the expansion must leave behind, in definition order.
    pub definition_types: &'a [ValueType],
}

/// Structured failure returned by a JVM intrinsic policy.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
pub struct JavaIntrinsicLoweringError {
    message: String,
}

impl JavaIntrinsicLoweringError {
    /// Creates an intrinsic-policy failure.
    #[must_use]
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

/// Maps implementation-defined MLIL intrinsics to straight-line JVM bytecode.
///
/// Cafe loads every MLIL use onto the operand stack before emitting the returned
/// instructions and stores every resulting definition afterward. The finished
/// JVM body is analyzed and verified, so an expansion with an incompatible stack
/// effect or operand encoding is rejected transactionally. Control-flow opcodes
/// are rejected because the semantic successors remain owned by the MLIL graph;
/// a may-throw intrinsic must return at least one instruction so its protected
/// native interval is non-empty.
pub trait JavaMlilIntrinsicLowerer {
    /// Selects a JVM expansion and may intern target constants into `pool`.
    ///
    /// # Errors
    ///
    /// Returns an explanation when this policy does not support the intrinsic or
    /// cannot construct its target operands.
    fn lower(
        &mut self,
        request: JavaIntrinsicRequest<'_>,
        pool: &mut ConstantPool,
    ) -> std::result::Result<Vec<JavaIntrinsicInstruction>, JavaIntrinsicLoweringError>;
}

/// Default policy that requires callers to opt into every intrinsic meaning.
#[derive(Debug, Clone, Copy, Default)]
pub struct RejectJavaIntrinsics;

impl JavaMlilIntrinsicLowerer for RejectJavaIntrinsics {
    fn lower(
        &mut self,
        request: JavaIntrinsicRequest<'_>,
        _pool: &mut ConstantPool,
    ) -> std::result::Result<Vec<JavaIntrinsicInstruction>, JavaIntrinsicLoweringError> {
        Err(JavaIntrinsicLoweringError::new(format!(
            "JVM intrinsic policy does not define `{}`",
            request.name
        )))
    }
}

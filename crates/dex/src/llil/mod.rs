//! Dalvik-specific low-level intermediate language.
//!
//! DEX LLIL normalizes opcode encoding variants and makes typed register uses,
//! definitions, references, literals, and targets explicit. Every instruction
//! also retains its exact native encoding for checked reconstruction.

mod body;
mod lift;
mod model;

pub use self::body::{Body, lift_code, lower_code};
pub use self::lift::{lift_instructions, lower_instructions};
pub use self::model::{
    ArithmeticOperator, ArrayAccess, ArrayElementKind, Comparison, ConstantKind, Conversion,
    FieldAccess, Instruction, InstructionKind, Invocation, MonitorAction, NativeEncoding, Operand,
    Operation, OperationKind, Payload, Relation, UnaryOperator,
};

#[cfg(test)]
mod tests;

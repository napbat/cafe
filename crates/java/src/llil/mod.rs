//! JVM-specific low-level intermediate language.
//!
//! JVM LLIL normalizes encoding aliases into semantic operations while retaining
//! an exact native encoding beside every instruction. This makes decoded method
//! bodies suitable for analysis without sacrificing byte-for-byte reconstruction.

mod body;
mod lift;
mod model;

pub use self::body::{Body, lift_code, lower_code};
pub use self::lift::{lift_instructions, lower_instructions};
pub use self::model::{
    ArithmeticOperator, ArrayAccess, ArrayElementKind, BitwiseOperator, BranchCondition,
    Comparison, Constant, ConstantWidth, Conversion, FieldAccess, Instruction, Intrinsic,
    Invocation, LocalAccess, MonitorAction, NativeEncoding, Operation, Relation, ShiftOperator,
    StackOperation, Switch, SwitchCase, ValueKind,
};

#[cfg(test)]
mod tests;

//! Owned resolution of JVM instruction references.

mod model;
mod resolve;

pub use self::model::{
    ClassSymbol, DynamicSymbol, ExactString, FieldSymbol, InstructionReference, LoadableConstant,
    MethodHandleSymbol, MethodHandleTargetSymbol, MethodReferenceKind, MethodSymbol,
};
pub use self::resolve::resolve_instruction_reference;

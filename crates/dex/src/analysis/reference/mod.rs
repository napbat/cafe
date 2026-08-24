//! Owned resolution of instruction references and recursively encoded values.

mod model;
mod resolve;

pub use self::model::{
    AnnotationElementSymbol, AnnotationSymbol, CallSiteSymbol, ExactString, FieldSymbol,
    InstructionReference, InstructionReferences, MethodHandleSymbol, MethodHandleTargetSymbol,
    MethodSymbol, PrototypeSymbol, ResolvedValue, TypeSymbol,
};
pub use self::resolve::{resolve_instruction_references, resolve_value};

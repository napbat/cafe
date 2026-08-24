//! Semantic analysis utilities over typed Dalvik instructions.

mod body;
mod flow;
mod hierarchy;
mod model;
mod reference;
mod register;
mod semantics;

pub use self::body::analyze_body;
pub use self::flow::control_flow;
pub use self::hierarchy::{
    DexHierarchy, JAVA_IO_SERIALIZABLE_DESCRIPTOR, JAVA_LANG_CLONEABLE_DESCRIPTOR,
    JAVA_LANG_OBJECT_DESCRIPTOR, ReferenceHierarchy,
};
pub use self::model::{
    AnalyzedInstruction, BodyAnalysis, ControlFlow, FlowEdge, FlowEdgeKind, InstructionSemantics,
    PayloadKind, PayloadLink, ProducedValue, RegisterOperand, ValueKind,
};
pub use self::reference::{
    AnnotationElementSymbol, AnnotationSymbol, CallSiteSymbol, ExactString, FieldSymbol,
    InstructionReference, InstructionReferences, MethodHandleSymbol, MethodHandleTargetSymbol,
    MethodSymbol, PrototypeSymbol, ResolvedValue, TypeSymbol, resolve_instruction_references,
    resolve_value,
};
pub use self::register::{
    ReferenceType, RegisterAnalysis, RegisterFrame, RegisterType, analyze_method_registers,
    analyze_method_registers_with_hierarchy,
};
pub use self::semantics::instruction_semantics;

#[cfg(test)]
mod tests;

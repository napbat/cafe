//! JVM stack/local verification, maxima computation, and stack-map generation.
//!
//! Native instruction flow implements cfglib's borrowed graph contracts, and
//! frame propagation uses its seeded fallible edge-sensitive solver so normal
//! transfers consume post-state while exception handlers consume pre-state.

mod flow;
mod frame;
mod hierarchy;
mod model;
mod reference;
mod stack_map;
mod stack_ops;
mod transfer;

pub(crate) use self::frame::analyze_decoded_code_with_hierarchy;
pub use self::frame::{
    analyze_code, analyze_code_with_hierarchy, analyze_method, analyze_method_with_hierarchy,
};
pub use self::hierarchy::{
    ClassHierarchy, JAVA_IO_SERIALIZABLE_NAME, JAVA_LANG_CLONEABLE_NAME, JAVA_LANG_OBJECT_NAME,
    ReferenceHierarchy,
};
pub use self::model::{
    ControlFlow, FlowEdge, FlowEdgeKind, FrameState, FrameValue, MethodAnalysis,
};
pub use self::reference::{
    ClassSymbol, DynamicSymbol, ExactString, FieldSymbol, InstructionReference, LoadableConstant,
    MethodHandleSymbol, MethodHandleTargetSymbol, MethodReferenceKind, MethodSymbol,
    resolve_instruction_reference,
};

#[cfg(test)]
mod tests;

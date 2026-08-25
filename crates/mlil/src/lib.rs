//! Shared typed semantic intermediate language for Java-ecosystem bytecode.
//!
//! MLIL removes JVM operand-stack and Dalvik register-encoding mechanics while
//! retaining explicit variables, types, effects, control flow, exception
//! provenance, and format-qualified native origins. Frontend-owned adapters
//! lift JVM and Dalvik LLIL into this crate. Target-specific code generation
//! remains outside this boundary.

mod builder;
mod descriptor;
mod error;
mod model;
mod verify;

pub use self::builder::FunctionBuilder;
pub use self::error::{Error, Result, VerificationIssue, VerificationReport};
pub use self::model::{
    AllocationKind, AllocationSite, ArrayAccess, BinaryOperator, BranchOperandKind,
    BranchPredicate, CallKind, Constant, Conversion, EdgeMetadata, EdgeRole, Effect, ElementType,
    EntityId, FieldAccess, Function, Instruction, InstructionId, MonitorAction, NativeVariable,
    Operation, ProvenanceEntry, ProvenanceMap, Relation, SourceStorage, ThreeWayComparison,
    TypedVariable, UnaryOperator, ValueType, Variable, VariableId, VariableRole,
};

/// Graph algorithms and SSA/data-flow facilities used by MLIL.
pub use disassembler::cfglib;

#[cfg(test)]
mod tests;

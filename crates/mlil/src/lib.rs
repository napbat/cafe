//! Java-managed semantic dialect for cfglib's generic medium-level IR.
//!
//! [`cfglib::ir::mlil`] owns generic MLIL storage, stable identities, provenance,
//! verification scaffolding, and reusable analyses. This crate supplies the
//! shared Java-ecosystem operation, type, effect, edge, and source vocabulary
//! used by JVM and Dalvik frontends. Target-specific lowering remains in those
//! frontends so this layer never owns stack or register allocation, native
//! reference tables, or encoding policy.

mod analysis;
mod descriptor;
mod dialect;
mod hlil;
mod model;
pub mod rtl;
mod verify;

pub use self::analysis::ExpressionOperator;
pub use self::dialect::JavaDialect;
pub use self::model::{
    AllocationKind, AllocationSite, ArrayAccess, ArrayType, BinaryOperator, BranchOperandKind,
    BranchPredicate, CallKind, Constant, Conversion, EdgeMetadata, EdgeRole, Effect, ElementType,
    EntityId, FieldAccess, Function, Instruction, InstructionId, MonitorAction, NativeVariable,
    Operation, ProvenanceEntry, ProvenanceMap, Relation, SourceStorage, ThreeWayComparison,
    TypedVariable, UnaryOperator, ValueType, Variable, VariableId, VariableRole,
};
pub use cfglib::ir::mlil::{Error, Result, VerificationIssue, VerificationReport};

/// Checked builder for a Java-managed semantic function.
pub type FunctionBuilder = cfglib::ir::mlil::FunctionBuilder<JavaDialect>;

/// Graph algorithms and SSA/data-flow facilities used by MLIL.
pub use cfglib;

#[cfg(test)]
mod tests;

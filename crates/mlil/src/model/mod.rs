//! MLIL public model facades.

mod edge;
mod function;
mod instruction;
mod operation;
mod provenance;
mod types;

pub use self::edge::{EdgeMetadata, EdgeRole};
pub use self::function::Function;
pub use self::instruction::{Effect, Instruction, InstructionId};
pub(crate) use self::operation::ControlClass;
pub use self::operation::{
    AllocationKind, ArrayAccess, BinaryOperator, BranchOperandKind, BranchPredicate, CallKind,
    Constant, Conversion, ElementType, FieldAccess, MonitorAction, Operation, Relation,
    ThreeWayComparison, UnaryOperator,
};
pub use self::provenance::{EntityId, ProvenanceEntry, ProvenanceMap};
pub use self::types::{
    AllocationSite, NativeVariable, SourceStorage, TypedVariable, ValueType, Variable, VariableId,
    VariableRole,
};

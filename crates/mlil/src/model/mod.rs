//! MLIL public model facades.

mod edge;
mod instruction;
mod operation;
mod types;

pub use self::edge::{EdgeMetadata, EdgeRole};
pub use self::instruction::Effect;
pub(crate) use self::operation::ControlClass;
pub use self::operation::{
    AllocationKind, ArrayAccess, ArrayType, BinaryOperator, BranchOperandKind, BranchPredicate,
    CallKind, Constant, Conversion, ElementType, FieldAccess, MonitorAction, Operation, Relation,
    ThreeWayComparison, UnaryOperator,
};
pub use self::types::{AllocationSite, NativeVariable, SourceStorage, ValueType, VariableRole};
pub use cfglib::ir::mlil::{EntityId, InstructionId, VariableId};

/// One Java-managed semantic function backed by cfglib control flow.
pub type Function = cfglib::ir::mlil::Function<crate::JavaDialect>;

/// One typed Java-managed semantic instruction.
pub type Instruction = cfglib::ir::mlil::Instruction<crate::JavaDialect>;

/// One native source span mapped to one Java-managed MLIL entity.
pub type ProvenanceEntry = cfglib::ir::mlil::ProvenanceEntry<crate::JavaDialect>;

/// Deterministic native-source-to-MLIL provenance.
pub type ProvenanceMap = cfglib::ir::mlil::ProvenanceMap<crate::JavaDialect>;

/// One typed variable occurrence in Java-managed MLIL.
pub type TypedVariable = cfglib::ir::mlil::TypedVariable<crate::JavaDialect>;

/// One mutable pre-SSA variable in Java-managed MLIL.
pub type Variable = cfglib::ir::mlil::Variable<crate::JavaDialect>;

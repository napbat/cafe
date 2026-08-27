//! Java-managed specialization of cfglib's generic MLIL contracts.

use std::collections::BTreeMap;

use cfglib::ir::mlil::{
    AnalysisDialect, Dialect, InstructionMetadata, VerificationIssue, VerifyDialect,
};
use cfglib::{EdgeKind, FlowEffect, Vocabulary};
use disassembler::{AddressRange, CodeAddress, FunctionCoordinate};

use crate::model::{
    ArrayAccess, ControlClass, EdgeMetadata, EdgeRole, Effect, FieldAccess, NativeVariable,
    Operation, ValueType, VariableRole,
};
use crate::{Constant, ExpressionOperator, Function, Instruction, VariableId};

/// Shared managed-language semantics used by the JVM and Dalvik frontends.
///
/// This is deliberately a semantic dialect rather than a source-ISA marker:
/// both frontends erase stack/register encoding mechanics into the same
/// operations while retaining their exact native identities in provenance.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct JavaDialect;

impl Vocabulary for JavaDialect {
    type ValueType = ValueType;
    type Effect = Effect;
    type Source = FunctionCoordinate;
    type SourceSpan = AddressRange;
    type SourcePoint = CodeAddress;
    type VariableRole = VariableRole;
    type NativeVariable = NativeVariable;

    fn span_is_empty(span: &Self::SourceSpan) -> bool {
        span.is_empty()
    }

    fn span_contains(span: &Self::SourceSpan, point: &Self::SourcePoint) -> bool {
        span.contains(*point)
    }
}

impl Dialect for JavaDialect {
    type Operation = Operation;
    type Edge = EdgeMetadata;

    fn instruction_metadata(
        operation: &Self::Operation,
        may_throw: bool,
    ) -> InstructionMetadata<Self::Effect> {
        let may_throw = may_throw || matches!(operation, Operation::Throw);
        let flow = match operation.control_class() {
            ControlClass::Normal if may_throw => FlowEffect::MayThrow,
            ControlClass::Normal => FlowEffect::Fallthrough,
            ControlClass::Branch => FlowEffect::ConditionalJump,
            ControlClass::Jump => FlowEffect::Jump,
            ControlClass::Switch => FlowEffect::IndirectJump,
            ControlClass::Return => FlowEffect::Return,
            ControlClass::Throw => FlowEffect::Terminate,
        };
        InstructionMetadata::new(operation_effects(operation, may_throw), flow, may_throw)
    }

    fn mnemonic(operation: &Self::Operation) -> &str {
        operation.mnemonic()
    }

    fn edge_kind(edge: &Self::Edge) -> EdgeKind {
        edge.role.cfglib_kind()
    }

    fn is_entry_edge(edge: &Self::Edge) -> bool {
        edge.role == EdgeRole::Entry
    }
}

impl AnalysisDialect for JavaDialect {
    type Constant = Constant;
    type ExpressionOperator = ExpressionOperator;
    type Callee = String;

    fn is_copy(operation: &Self::Operation) -> bool {
        crate::analysis::is_copy(operation)
    }

    fn is_value_alias(operation: &Self::Operation) -> bool {
        matches!(
            operation,
            Operation::Copy | Operation::ParallelCopy | Operation::TypeRefine
        )
    }

    fn expression_operator(operation: &Self::Operation) -> Option<Self::ExpressionOperator> {
        crate::analysis::expression_operator(operation)
    }

    fn constant(operation: &Self::Operation) -> Option<Self::Constant> {
        crate::analysis::constant(operation)
    }

    fn fold_constant(
        instruction: &Instruction,
        known: &BTreeMap<VariableId, Self::Constant>,
    ) -> Option<(VariableId, Self::Constant)> {
        crate::analysis::fold_constant(instruction, known)
    }

    fn callee(operation: &Self::Operation) -> Option<Self::Callee> {
        crate::analysis::callee(operation)
    }
}

impl VerifyDialect for JavaDialect {
    fn verify(function: &Function, issues: &mut Vec<VerificationIssue>) {
        crate::verify::verify_function(function, issues);
    }
}

fn operation_effects(operation: &Operation, may_throw: bool) -> Vec<Effect> {
    let mut effects = match operation {
        Operation::Array {
            access: ArrayAccess::Get,
            ..
        }
        | Operation::ArrayLength
        | Operation::Field {
            access: FieldAccess::GetInstance | FieldAccess::GetStatic,
            ..
        } => vec![Effect::ReadMemory],
        Operation::Array {
            access: ArrayAccess::Put,
            ..
        }
        | Operation::InitializeArray { .. }
        | Operation::Field {
            access: FieldAccess::PutInstance | FieldAccess::PutStatic,
            ..
        } => vec![Effect::WriteMemory],
        Operation::Call { .. } => vec![Effect::Call, Effect::ReadMemory, Effect::WriteMemory],
        Operation::Allocate(_) => vec![Effect::Allocate],
        Operation::Monitor(_) => vec![Effect::Synchronize],
        Operation::Branch(_)
        | Operation::Jump
        | Operation::Switch(_)
        | Operation::Return
        | Operation::Throw => vec![Effect::Control],
        Operation::Intrinsic(_) => vec![Effect::ReadMemory, Effect::WriteMemory],
        Operation::Nop
        | Operation::Copy
        | Operation::ParallelCopy
        | Operation::Discard
        | Operation::TypeRefine
        | Operation::Constant(_)
        | Operation::Unary(_)
        | Operation::Binary(_)
        | Operation::Convert(_)
        | Operation::Compare(_)
        | Operation::CheckCast(_)
        | Operation::InstanceOf(_)
        | Operation::CaughtException(_)
        | Operation::Select => Vec::new(),
    };
    if may_throw {
        effects.push(Effect::Throw);
    }
    effects
}

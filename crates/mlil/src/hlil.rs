//! Java-managed specialization of cfglib's generic HLIL contracts.
//!
//! The MLIL operation vocabulary is reused verbatim as the HLIL expression
//! vocabulary: every value-producing or effectful operation becomes an
//! expression node, stores become assignments to place expressions, and
//! flat control transfers dissolve into cfglib's structured statements.

use cfglib::ir::hlil::{
    Dialect, ExpressionKind, Function, LiftDialect, Lifted, RecoverDialect, VerificationIssue,
    VerifyDialect,
};

use crate::model::{
    ArrayAccess, EdgeMetadata, EdgeRole, Effect, FieldAccess, MonitorAction, Operation, ValueType,
    VariableRole,
};
use crate::{Constant, JavaDialect};

impl Dialect for JavaDialect {
    type Operation = Operation;
    type Constant = Constant;

    fn mnemonic(operation: &Self::Operation) -> &str {
        operation.mnemonic()
    }
}

impl VerifyDialect for JavaDialect {
    fn verify(function: &Function<Self>, issues: &mut Vec<VerificationIssue>) {
        // Flat control transfers have no expression form: branches survive
        // only as fused condition operations, everything else dissolves
        // into structured statements.
        for expression in function.expressions() {
            let ExpressionKind::Operation { operation, .. } = expression.kind() else {
                continue;
            };
            if matches!(
                operation,
                Operation::Nop
                    | Operation::Copy
                    | Operation::ParallelCopy
                    | Operation::Discard
                    | Operation::TypeRefine
                    | Operation::Jump
                    | Operation::Switch(_)
                    | Operation::Return
            ) {
                issues.push(VerificationIssue::new(format!(
                    "expression {} applies flat MLIL operation {}",
                    expression.id(),
                    operation.mnemonic()
                )));
            }
        }
    }
}

impl RecoverDialect for JavaDialect {
    fn select() -> Option<Operation> {
        Some(Operation::Select)
    }

    fn single_expression_assignment(value_type: &ValueType) -> bool {
        // The Dalvik zero pattern writes both slot views and cannot stand
        // in a `for` header or a selection arm.
        !matches!(value_type, ValueType::Zero)
    }

    fn region_enter(operation: &Operation) -> Option<Operation> {
        matches!(operation, Operation::Monitor(MonitorAction::Enter)).then(|| operation.clone())
    }

    fn releases(enter: &Operation, exit: &Operation) -> bool {
        matches!(enter, Operation::Monitor(MonitorAction::Enter))
            && matches!(exit, Operation::Monitor(MonitorAction::Exit))
    }

    fn is_exception_materialization(operation: &Operation) -> bool {
        matches!(operation, Operation::CaughtException(_))
    }

    fn is_throw(operation: &Operation) -> bool {
        matches!(operation, Operation::Throw)
    }
}

impl LiftDialect for JavaDialect {
    fn negate_operation(operation: &Operation) -> Option<Operation> {
        // Branch predicates negate exactly by relation inversion over the
        // same operands — `while (a < b)` instead of `!(a >= b)`.
        let Operation::Branch(predicate) = operation else {
            return None;
        };
        Some(Operation::Branch(predicate.inverted()))
    }

    fn lift_operation(operation: &Operation) -> Lifted<Operation> {
        match operation {
            Operation::Nop | Operation::Jump | Operation::Discard => Lifted::ControlFlow,
            Operation::ParallelCopy | Operation::TypeRefine => Lifted::ParallelCopy,
            Operation::Branch(_) => Lifted::BranchOperation(operation.clone()),
            Operation::Switch(_) => Lifted::Switch,
            Operation::Return => Lifted::Return,
            Operation::Array {
                access: ArrayAccess::Put,
                element,
            } => Lifted::Store {
                location: Operation::Array {
                    access: ArrayAccess::Get,
                    element: *element,
                },
            },
            Operation::Field {
                access: FieldAccess::PutInstance,
                field,
            } => Lifted::Store {
                location: Operation::Field {
                    access: FieldAccess::GetInstance,
                    field: field.clone(),
                },
            },
            Operation::Field {
                access: FieldAccess::PutStatic,
                field,
            } => Lifted::Store {
                location: Operation::Field {
                    access: FieldAccess::GetStatic,
                    field: field.clone(),
                },
            },
            other => Lifted::Operation(other.clone()),
        }
    }

    fn case_values(edge: &EdgeMetadata) -> Vec<Constant> {
        match edge.role {
            EdgeRole::SwitchCase(value) => {
                vec![i32::try_from(value).map_or(Constant::Long(value), Constant::Integer)]
            }
            _ => Vec::new(),
        }
    }

    fn void_type() -> ValueType {
        ValueType::Unknown
    }

    fn temporary_role() -> Option<VariableRole> {
        Some(VariableRole::Temporary)
    }

    fn evaluation_commutes(
        moved_effects: &[Effect],
        moved_may_throw: bool,
        crossed_effects: &[Effect],
        crossed_may_throw: bool,
    ) -> bool {
        // Java fixes evaluation and exception order exactly, so only
        // non-throwing memory reads may move across each other.
        if moved_may_throw || crossed_may_throw {
            return false;
        }
        let read_only =
            |effects: &[Effect]| effects.iter().all(|effect| *effect == Effect::ReadMemory);
        read_only(moved_effects) && read_only(crossed_effects)
    }
}

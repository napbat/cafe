//! Statement-tree analyses over the HLIL view: JLS reachability for the
//! recompilation scaffold, and single-read return forwarding.

use std::collections::{BTreeMap, BTreeSet};

use mlil::cfglib::ir::hlil::{ExpressionId, ExpressionKind, StatementId, StatementKind};
use mlil::{Operation, VariableId};

use super::{HlilFunction, mlil_variable};

/// Single-read assignments forwarded into their immediately following
/// return.
pub(super) struct Forwardings {
    pub(super) assigns: BTreeSet<StatementId>,
    pub(super) returns: BTreeMap<StatementId, ExpressionId>,
    pub(super) variables: BTreeSet<VariableId>,
}

/// Finds `v = value; return v;` pairs where the return's read is the
/// variable's only read in the function: the value renders inside the
/// return and the variable disappears. Adjacency makes the move
/// order-exact — no statement runs between the assignment and the read.
pub(super) fn return_forwardings(function: &HlilFunction) -> Forwardings {
    use mlil::cfglib::ir::hlil::VariableId as HlilVariableId;

    let mut occurrences: BTreeMap<HlilVariableId, usize> = BTreeMap::new();
    for expression in function.expressions() {
        if let ExpressionKind::Variable(variable) = expression.kind() {
            *occurrences.entry(*variable).or_default() += 1;
        }
    }
    let mut forwarding = Forwardings {
        assigns: BTreeSet::new(),
        returns: BTreeMap::new(),
        variables: BTreeSet::new(),
    };
    let mut lists: Vec<&[StatementId]> = vec![function.body()];
    while let Some(list) = lists.pop() {
        for window in list.windows(2) {
            let [assign, ret] = window else {
                continue;
            };
            let (Some(assign_kind), Some(return_kind)) = (
                function
                    .statement(*assign)
                    .map(mlil::cfglib::ir::hlil::Statement::kind),
                function
                    .statement(*ret)
                    .map(mlil::cfglib::ir::hlil::Statement::kind),
            ) else {
                continue;
            };
            let (StatementKind::Assign { target, value }, StatementKind::Return { values }) =
                (assign_kind, return_kind)
            else {
                continue;
            };
            let ([read], Some(target)) = (values.as_slice(), function.expression(*target)) else {
                continue;
            };
            let Some(read_expression) = function.expression(*read) else {
                continue;
            };
            let (ExpressionKind::Variable(written), ExpressionKind::Variable(observed)) =
                (target.kind(), read_expression.kind())
            else {
                continue;
            };
            if written != observed || occurrences.get(written) != Some(&2) {
                continue;
            }
            let role = function
                .variables()
                .get(written.index())
                .map(|declared| declared.role);
            if !matches!(
                role,
                Some(
                    mlil::VariableRole::Local
                        | mlil::VariableRole::Temporary
                        | mlil::VariableRole::Condition
                )
            ) {
                continue;
            }
            forwarding.assigns.insert(*assign);
            forwarding.returns.insert(*ret, *value);
            forwarding.variables.insert(mlil_variable(*written));
        }
        for &id in list {
            push_child_lists(function, id, &mut lists);
        }
    }
    forwarding
}

/// Pushes every nested statement list of one statement.
fn push_child_lists<'f>(
    function: &'f HlilFunction,
    id: StatementId,
    lists: &mut Vec<&'f [StatementId]>,
) {
    let Some(kind) = function
        .statement(id)
        .map(mlil::cfglib::ir::hlil::Statement::kind)
    else {
        return;
    };
    match kind {
        StatementKind::If {
            then_body,
            else_body,
            ..
        } => {
            lists.push(then_body);
            lists.push(else_body);
        }
        StatementKind::While { body, .. }
        | StatementKind::DoWhile { body, .. }
        | StatementKind::Loop { body }
        | StatementKind::Labeled { body, .. }
        | StatementKind::Region { body, .. } => lists.push(body),
        StatementKind::For {
            initializer,
            update,
            body,
            ..
        } => {
            lists.push(initializer);
            lists.push(update);
            lists.push(body);
        }
        StatementKind::Switch {
            cases,
            default_body,
            ..
        } => {
            for case in cases {
                lists.push(&case.body);
            }
            lists.push(default_body);
        }
        StatementKind::Try {
            body,
            handlers,
            finally_body,
        } => {
            lists.push(body);
            for handler in handlers {
                lists.push(&handler.body);
            }
            lists.push(finally_body);
        }
        _ => {}
    }
}

/// Whether a statement list can complete normally — a conservative shape
/// of JLS §14.21 reachability, so `true` means javac may demand more code
/// after it. The recompilation scaffold (the constant guard and trailing
/// `AssertionError`) is needed only for bodies that can run off their
/// end; a body sealed by a return or throw on every path renders bare.
/// Every `false` here is one javac itself can prove, so an elided
/// scaffold never turns into a missing-return error.
pub(super) fn completes_normally(function: &HlilFunction, statements: &[StatementId]) -> bool {
    statements
        .last()
        .is_none_or(|&last| statement_completes(function, last))
}

fn statement_completes(function: &HlilFunction, statement: StatementId) -> bool {
    let Some(kind) = function
        .statement(statement)
        .map(mlil::cfglib::ir::hlil::Statement::kind)
    else {
        return true;
    };
    match kind {
        StatementKind::Return { .. }
        | StatementKind::Break { .. }
        | StatementKind::Continue { .. } => false,
        StatementKind::Expression(expression) => !throws(function, *expression),
        StatementKind::If {
            then_body,
            else_body,
            ..
        } => {
            else_body.is_empty()
                || completes_normally(function, then_body)
                || completes_normally(function, else_body)
        }
        StatementKind::Try {
            body,
            handlers,
            finally_body,
        } => {
            (completes_normally(function, body)
                || handlers
                    .iter()
                    .any(|handler| completes_normally(function, &handler.body)))
                && completes_normally(function, finally_body)
        }
        StatementKind::Region { body, .. } => completes_normally(function, body),
        // A switch with a default seals only when no group completes and
        // no unlabeled break escapes it (JLS §14.21); without a default
        // it always completes.
        StatementKind::Switch {
            cases,
            default_body,
            ..
        } => {
            default_body.is_empty()
                || completes_normally(function, default_body)
                || cases
                    .iter()
                    .any(|arm| completes_normally(function, &arm.body))
                || cases
                    .iter()
                    .map(|arm| arm.body.as_slice())
                    .chain(core::iter::once(default_body.as_slice()))
                    .any(|body| reaches_unlabeled_break(function, body))
        }
        // Loops, labels, and residue assume completion: the rendered
        // loop conditions are method calls javac cannot prove constant,
        // so it judges them the same way.
        _ => true,
    }
}

/// Whether an unlabeled `break` in this statement list escapes to the
/// nearest enclosing breakable construct — the walk stops at loops and
/// switches, which rebind it.
fn reaches_unlabeled_break(function: &HlilFunction, statements: &[StatementId]) -> bool {
    statements.iter().any(|&statement| {
        let Some(kind) = function
            .statement(statement)
            .map(mlil::cfglib::ir::hlil::Statement::kind)
        else {
            return false;
        };
        match kind {
            StatementKind::Break { label: None } => true,
            StatementKind::If {
                then_body,
                else_body,
                ..
            } => {
                reaches_unlabeled_break(function, then_body)
                    || reaches_unlabeled_break(function, else_body)
            }
            StatementKind::Try {
                body,
                handlers,
                finally_body,
            } => {
                reaches_unlabeled_break(function, body)
                    || reaches_unlabeled_break(function, finally_body)
                    || handlers
                        .iter()
                        .any(|handler| reaches_unlabeled_break(function, &handler.body))
            }
            StatementKind::Region { body, .. } | StatementKind::Labeled { body, .. } => {
                reaches_unlabeled_break(function, body)
            }
            _ => false,
        }
    })
}

/// Whether the expression statement is a `throw`.
pub(super) fn throws(function: &HlilFunction, expression: ExpressionId) -> bool {
    matches!(
        function
            .expression(expression)
            .map(mlil::cfglib::ir::hlil::Expression::kind),
        Some(ExpressionKind::Operation {
            operation: Operation::Throw,
            ..
        })
    )
}

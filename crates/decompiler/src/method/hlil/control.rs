//! Structured-statement emission: conditionals, loops, switches,
//! returns, labels, and exception dispatch.

use disassembler::CatchType;
use java::descriptor::ReturnType;
use mlil::cfglib::ir::hlil::{
    Expression, ExpressionId, ExpressionKind, Handler, HandlerKind, StatementId, StatementKind,
};
use mlil::{Constant, JavaDialect, Operation};

use super::super::instruction::RenderFailure;
use super::{HlilRenderer, java_label, statement_expressions};

/// One arm of a trailing loop-exit chain.
#[derive(Clone, Copy, PartialEq, Eq)]
enum ChainArm {
    /// Falls through to the rest of the chain (or the back edge).
    Fall,
    /// An unlabeled `break`.
    Break,
    /// An unlabeled `continue`.
    Continue,
}

impl HlilRenderer<'_> {
    pub(super) fn emit_if(
        &mut self,
        condition: ExpressionId,
        then_body: &[StatementId],
        else_body: &[StatementId],
    ) -> Result<(), RenderFailure> {
        let (text, _) = self.condition(condition, false)?;
        let launder = self.launder_required(condition);
        self.open_wrapper(launder);
        self.writer.line(&format!("if ({text}) {{"));
        self.writer.indent();
        self.emit_statements(then_body)?;
        self.writer.dedent();
        if else_body.is_empty() {
            self.writer.line("}");
        } else {
            self.writer.line("} else {");
            self.writer.indent();
            self.emit_statements(else_body)?;
            self.writer.dedent();
            self.writer.line("}");
        }
        self.close_wrapper(launder);
        Ok(())
    }

    pub(super) fn open_labeled(&mut self, label: Option<&str>, opening: &str) {
        match label {
            Some(label) => self.writer.line(&format!("{label}: {opening}")),
            None => self.writer.line(opening),
        }
        self.writer.indent();
    }

    pub(super) fn emit_while(
        &mut self,
        condition: ExpressionId,
        body: &[StatementId],
        label: Option<&str>,
    ) -> Result<(), RenderFailure> {
        let (text, _) = self.condition(condition, false)?;
        if self.launder_required(condition) {
            // A test needing the checked-exception launder cannot live in
            // the loop header; state it as an explicit laundered exit
            // test.
            let (exit, _) = self.condition(condition, true)?;
            self.open_labeled(label, "while (java.lang.Boolean.TRUE.booleanValue()) {");
            self.open_wrapper(true);
            self.writer.line(&format!("if ({exit}) {{"));
            self.writer.indent();
            self.writer.line("break;");
            self.writer.dedent();
            self.writer.line("}");
            self.close_wrapper(true);
            self.emit_statements(body)?;
            self.writer.dedent();
            self.writer.line("}");
            return Ok(());
        }
        self.open_labeled(label, &format!("while ({text}) {{"));
        self.emit_statements(body)?;
        self.writer.dedent();
        self.writer.line("}");
        Ok(())
    }

    pub(super) fn emit_do_while(
        &mut self,
        body: &[StatementId],
        condition: ExpressionId,
        label: Option<&str>,
    ) -> Result<(), RenderFailure> {
        let (text, _) = self.condition(condition, false)?;
        if self.launder_required(condition) {
            let (exit, _) = self.condition(condition, true)?;
            self.open_labeled(label, "while (java.lang.Boolean.TRUE.booleanValue()) {");
            self.emit_statements(body)?;
            self.open_wrapper(true);
            self.writer.line(&format!("if ({exit}) {{"));
            self.writer.indent();
            self.writer.line("break;");
            self.writer.dedent();
            self.writer.line("}");
            self.close_wrapper(true);
            self.writer.dedent();
            self.writer.line("}");
            return Ok(());
        }
        self.open_labeled(label, "do {");
        self.emit_statements(body)?;
        self.writer.dedent();
        self.writer.line(&format!("}} while ({text});"));
        Ok(())
    }

    /// An endless loop; a leading `if (c) break;` recovers a `while` header
    /// with the inverted relation when the test cannot throw.
    pub(super) fn emit_loop(
        &mut self,
        body: &[StatementId],
        label: Option<&str>,
    ) -> Result<(), RenderFailure> {
        if let Some((&first, rest)) = body.split_first()
            && let StatementKind::If {
                condition,
                then_body,
                else_body,
            } = self.statement_kind(first)?
            && else_body.is_empty()
            && let [only] = then_body.as_slice()
            && matches!(
                self.statement_kind(*only)?,
                StatementKind::Break { label: None }
            )
        {
            let condition = *condition;
            let (text, calls) = self.condition(condition, true)?;
            if !calls {
                let start = self.writer.position();
                self.open_labeled(label, &format!("while ({text}) {{"));
                self.map_statement(first, start);
                self.emit_statements(rest)?;
                self.writer.dedent();
                self.writer.line("}");
                return Ok(());
            }
        }
        if let Some((prefix, condition)) = self.do_while_tail(body)? {
            self.open_labeled(label, "do {");
            self.emit_statements(prefix)?;
            self.writer.dedent();
            self.writer.line(&format!("}} while ({condition});"));
            return Ok(());
        }
        self.open_labeled(label, "while (java.lang.Boolean.TRUE.booleanValue()) {");
        self.emit_statements(body)?;
        self.writer.dedent();
        self.writer.line("}");
        Ok(())
    }

    /// The trailing exit chain of an endless loop as one `do`/`while`
    /// condition. A run of trailing `if` statements whose arms are
    /// single unlabeled transfers decides, in evaluation order, between
    /// the back edge and the exit — falling off the body's end is the
    /// back edge — which is exactly `do { … } while (c)` with the chain
    /// composed by short-circuit operators (javac compiles a compound
    /// `do`/`while` latch this way). Refused when the remaining body
    /// carries its own `continue`, which would rebind to the condition.
    fn do_while_tail<'b>(
        &self,
        body: &'b [StatementId],
    ) -> Result<Option<(&'b [StatementId], String)>, RenderFailure> {
        let mut start = body.len();
        while start > 0 && self.chain_link(body[start - 1])?.is_some() {
            start -= 1;
        }
        let (prefix, chain) = body.split_at(start);
        if chain.is_empty() || self.contains_continue(prefix)? {
            return Ok(None);
        }
        // Compose the continue condition back to front; `None` means the
        // remaining chain always takes the back edge.
        let mut condition: Option<String> = None;
        for &statement in chain.iter().rev() {
            let Some((test, then_arm, else_arm)) = self.chain_link(statement)? else {
                return Ok(None);
            };
            let stated = |negated: bool| -> Result<String, RenderFailure> {
                Ok(HlilRenderer::guard(&self.condition(test, negated)?.0))
            };
            condition = match (then_arm, else_arm, condition.take()) {
                (ChainArm::Continue | ChainArm::Fall, ChainArm::Break, None) => {
                    Some(stated(false)?)
                }
                (ChainArm::Break, ChainArm::Continue | ChainArm::Fall, None) => Some(stated(true)?),
                (ChainArm::Continue, ChainArm::Fall, None)
                | (ChainArm::Fall, ChainArm::Continue, None) => None,
                (ChainArm::Continue, ChainArm::Fall, Some(rest))
                | (ChainArm::Fall, ChainArm::Continue, Some(rest)) => {
                    Some(format!("{} || {rest}", stated(false)?))
                }
                (ChainArm::Break, ChainArm::Fall, Some(rest)) => {
                    Some(format!("{} && {rest}", stated(true)?))
                }
                (ChainArm::Fall, ChainArm::Break, Some(rest)) => {
                    Some(format!("{} && {rest}", stated(false)?))
                }
                // Anything else — decisive arms followed by unreachable
                // chain, both arms transferring the same way, or no
                // transfer at all — is not this shape.
                _ => return Ok(None),
            };
        }
        Ok(condition.map(|condition| (prefix, condition)))
    }

    /// One chain-eligible `if`: unlabeled single-transfer (or empty)
    /// arms and a header-safe condition.
    fn chain_link(
        &self,
        statement: StatementId,
    ) -> Result<Option<(ExpressionId, ChainArm, ChainArm)>, RenderFailure> {
        let StatementKind::If {
            condition,
            then_body,
            else_body,
        } = self.statement_kind(statement)?
        else {
            return Ok(None);
        };
        let (Some(then_arm), Some(else_arm)) =
            (self.chain_arm(then_body)?, self.chain_arm(else_body)?)
        else {
            return Ok(None);
        };
        if matches!(
            (then_arm, else_arm),
            (ChainArm::Fall, ChainArm::Fall)
                | (ChainArm::Break, ChainArm::Break)
                | (ChainArm::Continue, ChainArm::Continue)
        ) || self.launder_required(*condition)
        {
            return Ok(None);
        }
        Ok(Some((*condition, then_arm, else_arm)))
    }

    fn chain_arm(&self, arm: &[StatementId]) -> Result<Option<ChainArm>, RenderFailure> {
        match arm {
            [] => Ok(Some(ChainArm::Fall)),
            [single] => Ok(match self.statement_kind(*single)? {
                StatementKind::Break { label: None } => Some(ChainArm::Break),
                StatementKind::Continue { label: None } => Some(ChainArm::Continue),
                _ => None,
            }),
            _ => Ok(None),
        }
    }

    pub(super) fn emit_switch(
        &mut self,
        scrutinee: ExpressionId,
        cases: &[mlil::cfglib::ir::hlil::SwitchArm<JavaDialect>],
        default_body: &[StatementId],
    ) -> Result<(), RenderFailure> {
        let rendered = self.render(scrutinee)?;
        let launder = self.launder_required(scrutinee);
        self.open_wrapper(launder);
        self.writer.line(&format!("switch ({}) {{", rendered.int()));
        self.writer.indent();
        for case in cases {
            let labels = case
                .values
                .iter()
                .map(|value| match value {
                    Constant::Integer(key) => Ok(format!("case {key}:")),
                    _ => Err(RenderFailure::new("switch case key is not a Java int")),
                })
                .collect::<Result<Vec<_>, _>>()?
                .join(" ");
            self.writer.line(&format!("{labels} {{"));
            self.writer.indent();
            self.emit_statements(&case.body)?;
            if self.completes_normally(&case.body) {
                self.writer.line("break;");
            }
            self.writer.dedent();
            self.writer.line("}");
        }
        if !default_body.is_empty() {
            self.writer.line("default: {");
            self.writer.indent();
            self.emit_statements(default_body)?;
            if self.completes_normally(default_body) {
                self.writer.line("break;");
            }
            self.writer.dedent();
            self.writer.line("}");
        }
        self.writer.dedent();
        self.writer.line("}");
        self.close_wrapper(launder);
        Ok(())
    }

    pub(super) fn emit_return(
        &mut self,
        id: StatementId,
        values: &[ExpressionId],
    ) -> Result<(), RenderFailure> {
        if self.class_initializer {
            // JLS 8.7 rejects `return` in a static initializer; the lift
            // places returns only where the initializer completes.
            return Ok(());
        }
        match self.return_type {
            ReturnType::Void => {
                self.writer.line("return;");
                Ok(())
            }
            ReturnType::Type(value_type) => {
                let value = match self.forwarded_returns.get(&id) {
                    Some(&forwarded) => forwarded,
                    None => *values
                        .first()
                        .ok_or_else(|| RenderFailure::new("typed return has no value"))?,
                };
                let rendered = self.render(value)?;
                let line = format!("return {};", self.as_java_type(&rendered, value_type));
                let launder = self.launder_required(value);
                self.write_lines(&[line], launder);
                Ok(())
            }
        }
    }

    pub(super) fn emit_labeled(
        &mut self,
        label: &str,
        body: &[StatementId],
    ) -> Result<(), RenderFailure> {
        let java = java_label(label);
        if let [only] = body {
            let start = self.writer.position();
            match self.statement_kind(*only)? {
                StatementKind::While { condition, body } => {
                    let (condition, body) = (*condition, body.clone());
                    self.emit_while(condition, &body, Some(&java))?;
                    self.map_statement(*only, start);
                    return Ok(());
                }
                StatementKind::DoWhile { body, condition } => {
                    let (condition, body) = (*condition, body.clone());
                    self.emit_do_while(&body, condition, Some(&java))?;
                    self.map_statement(*only, start);
                    return Ok(());
                }
                StatementKind::Loop { body } => {
                    let body = body.clone();
                    self.emit_loop(&body, Some(&java))?;
                    self.map_statement(*only, start);
                    return Ok(());
                }
                _ => {}
            }
        }
        self.writer.line(&format!("{java}: {{"));
        self.writer.indent();
        self.emit_statements(body)?;
        self.writer.dedent();
        self.writer.line("}");
        Ok(())
    }

    /// Renders one exception region as a single `catch (Throwable)` with
    /// its handlers as an ordered `instanceof` dispatch chain — the exact
    /// JVM semantics, and always-compilable Java.
    pub(super) fn emit_try(
        &mut self,
        body: &[StatementId],
        handlers: &[Handler<JavaDialect>],
        finally_body: &[StatementId],
    ) -> Result<(), RenderFailure> {
        if !finally_body.is_empty() {
            return Err(RenderFailure::new(
                "a finally region the JVM model cannot produce",
            ));
        }
        if let Some(finally) = self.finally_shape(handlers) {
            let mut copies = std::collections::BTreeSet::new();
            if !super::completes_normally(self.function, body)
                && self.finally_copies(body, finally, &mut copies)
            {
                self.finally_skips.extend(copies);
                let finally = finally.to_vec();
                self.writer.line("try {");
                self.writer.indent();
                self.emit_statements(body)?;
                self.writer.dedent();
                self.writer.line("} finally {");
                self.writer.indent();
                self.emit_statements(&finally)?;
                self.writer.dedent();
                self.writer.line("}");
                return Ok(());
            }
        }
        let caught = format!("cafe_caught_{}", self.caught_counter);
        self.caught_counter += 1;
        self.writer.line("try {");
        self.writer.indent();
        self.emit_statements(body)?;
        self.writer.dedent();
        self.writer
            .line(&format!("}} catch (java.lang.Throwable {caught}) {{"));
        self.writer.indent();
        self.caught_names.push(caught.clone());
        let rendered = self.emit_dispatch_chain(handlers, &caught);
        self.caught_names.pop();
        self.writer.dedent();
        self.writer.line("}");
        rendered
    }

    /// The finally body carried by a lone catch-all handler that ends by
    /// rethrowing the delivered exception — javac's `finally` encoding.
    fn finally_shape<'h>(&self, handlers: &'h [Handler<JavaDialect>]) -> Option<&'h [StatementId]> {
        let [handler] = handlers else {
            return None;
        };
        if !matches!(handler.kind, mlil::cfglib::ir::hlil::HandlerKind::CatchAll) {
            return None;
        }
        let (&rethrow, finally) = handler.body.split_last()?;
        if finally.is_empty() {
            return None;
        }
        let StatementKind::Expression(expression) = self.statement_kind(rethrow).ok()? else {
            return None;
        };
        let ExpressionKind::Operation {
            operation: Operation::Throw,
            operands,
        } = self.expression_kind(*expression).ok()?.kind()
        else {
            return None;
        };
        let [delivered] = operands.as_slice() else {
            return None;
        };
        matches!(
            self.expression_kind(*delivered).ok()?.kind(),
            ExpressionKind::Operation {
                operation: Operation::CaughtException(_),
                ..
            }
        )
        .then_some(finally)
    }

    /// Collects the duplicated finally copy before every abrupt exit of
    /// the try body — javac runs the finally on each `return` path — or
    /// refuses when any exit lacks one or the body holds shapes this
    /// recovery does not model (loops, nested regions, transfers).
    fn finally_copies(
        &self,
        statements: &[StatementId],
        finally: &[StatementId],
        out: &mut std::collections::BTreeSet<StatementId>,
    ) -> bool {
        for (index, &statement) in statements.iter().enumerate() {
            let Ok(kind) = self.statement_kind(statement) else {
                return false;
            };
            match kind {
                StatementKind::Return { .. } => {
                    if index < finally.len()
                        || !statements[index - finally.len()..index]
                            .iter()
                            .zip(finally)
                            .all(|(&copy, &original)| self.statements_equal(copy, original))
                    {
                        return false;
                    }
                    out.extend(&statements[index - finally.len()..index]);
                }
                StatementKind::If {
                    then_body,
                    else_body,
                    ..
                } => {
                    if !self.finally_copies(then_body, finally, out)
                        || !self.finally_copies(else_body, finally, out)
                    {
                        return false;
                    }
                }
                StatementKind::Expression(_) | StatementKind::Assign { .. } => {}
                _ => return false,
            }
        }
        true
    }

    /// Structural statement equality, conservative over the shapes a
    /// javac finally body can carry.
    fn statements_equal(&self, left: StatementId, right: StatementId) -> bool {
        let (Ok(left), Ok(right)) = (self.statement_kind(left), self.statement_kind(right)) else {
            return false;
        };
        match (left, right) {
            (StatementKind::Expression(a), StatementKind::Expression(b)) => {
                self.expressions_equal(*a, *b)
            }
            (
                StatementKind::Assign { target, value },
                StatementKind::Assign {
                    target: other_target,
                    value: other_value,
                },
            ) => {
                self.expressions_equal(*target, *other_target)
                    && self.expressions_equal(*value, *other_value)
            }
            (
                StatementKind::If {
                    condition,
                    then_body,
                    else_body,
                },
                StatementKind::If {
                    condition: other_condition,
                    then_body: other_then,
                    else_body: other_else,
                },
            ) => {
                self.expressions_equal(*condition, *other_condition)
                    && self.statement_lists_equal(then_body, other_then)
                    && self.statement_lists_equal(else_body, other_else)
            }
            _ => false,
        }
    }

    fn statement_lists_equal(&self, left: &[StatementId], right: &[StatementId]) -> bool {
        left.len() == right.len()
            && left
                .iter()
                .zip(right)
                .all(|(&a, &b)| self.statements_equal(a, b))
    }

    /// Structural expression equality over identical vocabulary.
    pub(super) fn expressions_equal(&self, left: ExpressionId, right: ExpressionId) -> bool {
        let (Ok(left), Ok(right)) = (self.expression_kind(left), self.expression_kind(right))
        else {
            return false;
        };
        match (left.kind(), right.kind()) {
            (ExpressionKind::Variable(a), ExpressionKind::Variable(b)) => a == b,
            (ExpressionKind::Constant(a), ExpressionKind::Constant(b)) => a == b,
            (
                ExpressionKind::Operation {
                    operation,
                    operands,
                },
                ExpressionKind::Operation {
                    operation: other_operation,
                    operands: other_operands,
                },
            ) => {
                operation == other_operation
                    && operands.len() == other_operands.len()
                    && operands
                        .iter()
                        .zip(other_operands)
                        .all(|(&a, &b)| self.expressions_equal(a, b))
            }
            _ => false,
        }
    }

    /// The ordered handler dispatch inside one rendered catch-all.
    pub(super) fn emit_dispatch_chain(
        &mut self,
        handlers: &[Handler<JavaDialect>],
        caught: &str,
    ) -> Result<(), RenderFailure> {
        if handlers.is_empty() {
            self.writer
                .line(&format!("throw {}({caught});", self.rethrow));
            return Ok(());
        }
        let mut open_arms = 0usize;
        let mut exhaustive = false;
        for (position, handler) in handlers.iter().enumerate() {
            match self.handler_catch_type(handler)? {
                CatchType::Any => {
                    if position != 0 {
                        self.writer.line("} else {");
                    }
                    exhaustive = true;
                }
                CatchType::Type(descriptor) => {
                    let catch_type = self
                        .names
                        .type_descriptor(&descriptor)
                        .map_err(|source| RenderFailure::new(source.to_string()))?;
                    let keyword = if position == 0 { "if" } else { "} else if" };
                    self.writer
                        .line(&format!("{keyword} ({caught} instanceof {catch_type}) {{"));
                }
            }
            let single_any = exhaustive && position == 0;
            if !single_any {
                self.writer.indent();
                open_arms += 1;
            }
            self.emit_statements(&handler.body)?;
            if !single_any {
                self.writer.dedent();
            }
            if exhaustive {
                break;
            }
        }
        if !exhaustive {
            self.writer.line("} else {");
            self.writer.indent();
            self.writer
                .line(&format!("throw {}({caught});", self.rethrow));
            self.writer.dedent();
        }
        if open_arms > 0 || !exhaustive {
            self.writer.line("}");
        }
        Ok(())
    }

    /// The catch type materialized at one handler's landing statement.
    pub(super) fn handler_catch_type(
        &self,
        handler: &Handler<JavaDialect>,
    ) -> Result<CatchType, RenderFailure> {
        match handler.kind {
            HandlerKind::CatchAll => return Ok(CatchType::Any),
            HandlerKind::Catch => {}
            HandlerKind::Fault | HandlerKind::Filter { .. } => {
                return Err(RenderFailure::new(
                    "a handler kind the JVM model cannot produce",
                ));
            }
        }
        let first = handler
            .body
            .first()
            .ok_or_else(|| RenderFailure::new("a typed handler has an empty body"))?;
        self.find_caught(*first)?
            .ok_or_else(|| RenderFailure::new("a structured handler does not begin at its landing"))
    }

    /// The [`Operation::CaughtException`] type inside one statement's
    /// expression trees, if any.
    pub(super) fn find_caught(
        &self,
        statement: StatementId,
    ) -> Result<Option<CatchType>, RenderFailure> {
        let mut stack = Vec::new();
        statement_expressions(self.statement_kind(statement)?, &mut stack);
        while let Some(id) = stack.pop() {
            if let ExpressionKind::Operation {
                operation,
                operands,
            } = self.expression_kind(id)?.kind()
            {
                if let Operation::CaughtException(catch) = operation {
                    return Ok(Some(catch.clone()));
                }
                stack.extend(operands.iter().copied());
            }
        }
        Ok(None)
    }

    /// Whether execution can reach the end of `ids` (JLS normal
    /// completion); decides whether a switch arm needs its closing
    /// `break;`.
    pub(super) fn completes_normally(&self, ids: &[StatementId]) -> bool {
        let Some(&last) = ids.last() else {
            return true;
        };
        let Ok(kind) = self.statement_kind(last) else {
            return true;
        };
        match kind {
            StatementKind::Break { .. }
            | StatementKind::Continue { .. }
            | StatementKind::Return { .. }
            | StatementKind::Goto { .. } => false,
            StatementKind::Expression(expression) => !matches!(
                self.function.expression(*expression).map(Expression::kind),
                Some(ExpressionKind::Operation {
                    operation: Operation::Throw,
                    ..
                })
            ),
            StatementKind::If {
                then_body,
                else_body,
                ..
            } => {
                else_body.is_empty()
                    || self.completes_normally(then_body)
                    || self.completes_normally(else_body)
            }
            _ => true,
        }
    }
}

impl HlilRenderer<'_> {
    /// Renders one recovered counted loop. Headers render inline when each
    /// section is one call-free simple assignment; otherwise the loop
    /// desugars to its exact `while` form with the update trailing the
    /// body — refused when a `continue` would bypass that update.
    pub(super) fn emit_for(
        &mut self,
        initializer: &[StatementId],
        condition: Option<ExpressionId>,
        update: &[StatementId],
        body: &[StatementId],
    ) -> Result<(), RenderFailure> {
        let init = self.header_fragment(initializer)?;
        let step = self.header_fragment(update)?;
        let test = condition
            .map(|condition| self.condition(condition, false))
            .transpose()?;
        if let (Some((init_text, false)), Some((step_text, false))) = (&init, &step)
            && !test.as_ref().is_some_and(|(_, calls)| *calls)
        {
            let test_text = test.as_ref().map_or("", |(text, _)| text.as_str());
            self.writer
                .line(&format!("for ({init_text}; {test_text}; {step_text}) {{"));
            self.writer.indent();
            self.emit_statements(body)?;
            self.writer.dedent();
            self.writer.line("}");
            return Ok(());
        }
        // Desugared form: exact only when the update cannot be bypassed.
        if self.contains_continue(body)? {
            return Err(RenderFailure::new(
                "a continue would bypass the desugared for update",
            ));
        }
        self.emit_statements(initializer)?;
        self.open_labeled(None, "while (java.lang.Boolean.TRUE.booleanValue()) {");
        if let Some(condition) = condition {
            let (exit, _) = self.condition(condition, true)?;
            let exit_calls = self.launder_required(condition);
            self.open_wrapper(exit_calls);
            self.writer.line(&format!("if ({exit}) {{"));
            self.writer.indent();
            self.writer.line("break;");
            self.writer.dedent();
            self.writer.line("}");
            self.close_wrapper(exit_calls);
        }
        self.emit_statements(body)?;
        self.emit_statements(update)?;
        self.writer.dedent();
        self.writer.line("}");
        Ok(())
    }

    /// One `for`-header section: empty, or a single call-free simple
    /// assignment rendered without its semicolon.
    fn header_fragment(
        &self,
        ids: &[StatementId],
    ) -> Result<Option<(String, bool)>, RenderFailure> {
        match ids {
            [] => Ok(Some((String::new(), false))),
            [only] => {
                let StatementKind::Assign { target, value } = self.statement_kind(*only)? else {
                    return Ok(None);
                };
                self.variable_assignment(*target, *value)
            }
            _ => Ok(None),
        }
    }

    /// Whether any statement of the subtree is a `continue`; nested loops
    /// are included, keeping the check conservative.
    fn contains_continue(&self, ids: &[StatementId]) -> Result<bool, RenderFailure> {
        let mut stack: Vec<StatementId> = ids.to_vec();
        while let Some(id) = stack.pop() {
            let kind = self.statement_kind(id)?;
            if matches!(kind, StatementKind::Continue { .. }) {
                return Ok(true);
            }
            match kind {
                StatementKind::If {
                    then_body,
                    else_body,
                    ..
                } => {
                    stack.extend(then_body.iter().copied());
                    stack.extend(else_body.iter().copied());
                }
                StatementKind::While { body, .. }
                | StatementKind::DoWhile { body, .. }
                | StatementKind::Loop { body }
                | StatementKind::Labeled { body, .. }
                | StatementKind::Region { body, .. } => stack.extend(body.iter().copied()),
                StatementKind::For {
                    initializer,
                    update,
                    body,
                    ..
                } => {
                    stack.extend(initializer.iter().copied());
                    stack.extend(update.iter().copied());
                    stack.extend(body.iter().copied());
                }
                StatementKind::Switch {
                    cases,
                    default_body,
                    ..
                } => {
                    for case in cases {
                        stack.extend(case.body.iter().copied());
                    }
                    stack.extend(default_body.iter().copied());
                }
                StatementKind::Try {
                    body,
                    handlers,
                    finally_body,
                } => {
                    stack.extend(body.iter().copied());
                    for handler in handlers {
                        stack.extend(handler.body.iter().copied());
                    }
                    stack.extend(finally_body.iter().copied());
                }
                _ => {}
            }
        }
        Ok(false)
    }

    /// Renders one recovered paired region; the JVM model's only region
    /// protocol is the monitor pair, spelled `synchronized`.
    pub(super) fn emit_region(
        &mut self,
        operation: &Operation,
        operands: &[ExpressionId],
        body: &[StatementId],
    ) -> Result<(), RenderFailure> {
        if !matches!(operation, Operation::Monitor(mlil::MonitorAction::Enter)) {
            return Err(RenderFailure::new(
                "region operation has no Java rendering policy",
            ));
        }
        let [subject] = operands else {
            return Err(RenderFailure::new("monitor region expects one subject"));
        };
        let subject = self.render(*subject)?;
        self.writer
            .line(&format!("synchronized ({}) {{", subject.object()));
        self.writer.indent();
        self.emit_statements(body)?;
        self.writer.dedent();
        self.writer.line("}");
        Ok(())
    }
}

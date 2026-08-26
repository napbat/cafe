//! Structured Java rendering over cfglib's lifted AST: loops with real
//! conditions, keyed switches with explicit defaults, and labeled breaks.

use disassembler::CatchType;
use mlil::cfglib::{AstNode, BlockId, CatchHandler, HandlerKind, LoopKind};
use mlil::{EdgeRole, Instruction, InstructionId, Operation};

use super::super::instruction::RenderFailure;
use super::RenderContext;

pub(super) fn structured_java(node: &AstNode<Instruction>) -> bool {
    match node {
        AstNode::Block { .. }
        | AstNode::Return { .. }
        | AstNode::Break { .. }
        | AstNode::Continue { .. } => true,
        AstNode::Sequence { body } | AstNode::Loop { body, .. } => body.iter().all(structured_java),
        AstNode::IfThenElse {
            then_body,
            else_body,
            ..
        } => then_body.iter().all(structured_java) && else_body.iter().all(structured_java),
        AstNode::Switch {
            cases,
            default_body,
            ..
        } => cases
            .iter()
            .flat_map(|case| &case.body)
            .chain(default_body)
            .all(structured_java),
        // A label is Java-representable only as a labeled loop (the shape
        // cfglib emits for multi-level breaks); free labels mean gotos.
        AstNode::Label { body, .. } => {
            matches!(body.as_slice(), [AstNode::Loop { .. }]) && body.iter().all(structured_java)
        }
        // Exception regions render as one catch-all with ordered
        // `instanceof` dispatch, so any catch-all handler must come last
        // (later arms would be silently unreachable) and the JVM model
        // never produces finally, fault, or filter arms.
        AstNode::TryCatch {
            try_body,
            handlers,
            finally_body,
        } => {
            finally_body.is_empty()
                && handlers.iter().all(|handler| {
                    matches!(handler.kind, HandlerKind::Catch | HandlerKind::CatchAll)
                })
                && handlers
                    .iter()
                    .position(|handler| matches!(handler.kind, HandlerKind::CatchAll))
                    .is_none_or(|position| position + 1 == handlers.len())
                && try_body.iter().all(structured_java)
                && handlers
                    .iter()
                    .flat_map(|handler| &handler.body)
                    .all(structured_java)
        }
        AstNode::Goto { .. } | AstNode::Guarded { .. } => false,
    }
}

/// A cfglib label name as a Java identifier (`.bb3` → `cafe__bb3`).
fn java_label(name: &str) -> String {
    let mut label = String::from("cafe_");
    for character in name.chars() {
        label.push(if character.is_ascii_alphanumeric() {
            character
        } else {
            '_'
        });
    }
    label
}

/// Whether execution can reach the end of `nodes` (JLS normal completion);
/// decides whether a switch arm needs its closing `break;`.
fn completes_normally(nodes: &[AstNode<Instruction>]) -> bool {
    match nodes.last() {
        Some(
            AstNode::Return { .. }
            | AstNode::Break { .. }
            | AstNode::Continue { .. }
            | AstNode::Goto { .. },
        ) => false,
        Some(AstNode::Sequence { body }) => completes_normally(body),
        Some(AstNode::IfThenElse {
            then_body,
            else_body,
            ..
        }) => {
            else_body.is_empty() || completes_normally(then_body) || completes_normally(else_body)
        }
        _ => true,
    }
}

/// Whether a plain `break` occurs inside a switch arm of this loop body
/// before any nested loop. cfglib's plain break exits the innermost loop,
/// while Java would bind it to the switch — such a loop needs a label.
fn switch_break_needs_label(nodes: &[AstNode<Instruction>]) -> bool {
    nodes.iter().any(|node| match node {
        AstNode::Switch {
            cases,
            default_body,
            ..
        } => cases
            .iter()
            .flat_map(|case| &case.body)
            .chain(default_body)
            .any(contains_naked_break),
        AstNode::Sequence { body }
        | AstNode::Label { body, .. }
        | AstNode::Guarded { body, .. } => switch_break_needs_label(body),
        AstNode::IfThenElse {
            then_body,
            else_body,
            ..
        } => switch_break_needs_label(then_body) || switch_break_needs_label(else_body),
        _ => false,
    })
}

fn contains_naked_break(node: &AstNode<Instruction>) -> bool {
    match node {
        AstNode::Break { label: None } => true,
        AstNode::Sequence { body }
        | AstNode::Label { body, .. }
        | AstNode::Guarded { body, .. } => body.iter().any(contains_naked_break),
        AstNode::IfThenElse {
            then_body,
            else_body,
            ..
        } => then_body.iter().chain(else_body).any(contains_naked_break),
        AstNode::Switch {
            cases,
            default_body,
            ..
        } => cases
            .iter()
            .flat_map(|case| &case.body)
            .chain(default_body)
            .any(contains_naked_break),
        // A nested loop captures its own plain breaks.
        _ => false,
    }
}

/// One enclosing construct during structured rendering, resolving how a
/// plain `break` must be spelled.
pub(super) enum Frame {
    Loop { label: Option<String> },
    Switch,
}

impl RenderContext<'_> {
    pub(super) fn render_ast(&mut self, node: &AstNode<Instruction>) -> Result<(), RenderFailure> {
        match node {
            AstNode::Block { instructions, .. } | AstNode::Return { instructions, .. } => {
                for instruction in instructions {
                    self.emit_instruction(instruction, true)?;
                }
            }
            AstNode::Sequence { body } => {
                for child in body {
                    self.render_ast(child)?;
                }
            }
            AstNode::IfThenElse {
                condition_instructions,
                then_body,
                else_body,
                ..
            } => {
                let (branch, prefix) = condition_instructions
                    .split_last()
                    .ok_or_else(|| RenderFailure::new("structured conditional has no branch"))?;
                for instruction in prefix {
                    self.emit_instruction(instruction, true)?;
                }
                let Operation::Branch(predicate) = branch.operation() else {
                    return Err(RenderFailure::new(
                        "structured conditional ends in a non-branch instruction",
                    ));
                };
                let start = self.writer.position();
                self.writer.line(&format!(
                    "if ({}) {{",
                    self.renderer.condition(branch, *predicate)
                ));
                self.map(branch.id(), start, self.writer.position());
                self.writer.indent();
                for child in then_body {
                    self.render_ast(child)?;
                }
                self.writer.dedent();
                if else_body.is_empty() {
                    self.writer.line("}");
                } else {
                    self.writer.line("} else {");
                    self.writer.indent();
                    for child in else_body {
                        self.render_ast(child)?;
                    }
                    self.writer.dedent();
                    self.writer.line("}");
                }
            }
            AstNode::Loop { header, kind, body } => {
                self.render_loop(None, *header, kind, body)?;
            }
            AstNode::Label { name, body } => {
                if let [AstNode::Loop { header, kind, body }] = body.as_slice() {
                    self.render_loop(Some(java_label(name)), *header, kind, body)?;
                } else {
                    return Err(RenderFailure::new(
                        "structured AST contains a label that is not a labeled loop",
                    ));
                }
            }
            AstNode::Switch {
                condition_instructions,
                cases,
                default_body,
                ..
            } => self.render_switch(condition_instructions, cases, default_body)?,
            AstNode::Break { label } => {
                let spelled = match label {
                    Some(name) => Some(java_label(name)),
                    None => self.naked_break_label()?,
                };
                match spelled {
                    Some(target) => self.writer.line(&format!("break {target};")),
                    None => self.writer.line("break;"),
                }
            }
            AstNode::Continue { label } => match label {
                Some(name) => {
                    let target = java_label(name);
                    self.writer.line(&format!("continue {target};"));
                }
                None => self.writer.line("continue;"),
            },
            AstNode::TryCatch {
                try_body,
                handlers,
                finally_body,
            } => self.render_try(try_body, handlers, finally_body)?,
            AstNode::Goto { .. } | AstNode::Guarded { .. } => {
                return Err(RenderFailure::new(
                    "structured AST contains a non-Java control-flow node",
                ));
            }
        }
        Ok(())
    }

    /// Renders one exception region as a single `catch (Throwable)` with
    /// its handlers as an ordered `instanceof` dispatch chain — the exact
    /// JVM semantics, and always-compilable Java (typed catch clauses
    /// would trip javac's checked-exception and subtype-order rules).
    fn render_try(
        &mut self,
        try_body: &[AstNode<Instruction>],
        handlers: &[CatchHandler<Instruction>],
        finally_body: &[AstNode<Instruction>],
    ) -> Result<(), RenderFailure> {
        if !finally_body.is_empty() {
            return Err(RenderFailure::new(
                "structured AST contains a finally region the JVM model cannot produce",
            ));
        }
        let caught = format!("cafe_caught_{}", self.caught_counter);
        self.caught_counter += 1;
        self.writer.line("try {");
        self.writer.indent();
        for child in try_body {
            self.render_ast(child)?;
        }
        self.writer.dedent();
        self.writer
            .line(&format!("}} catch (java.lang.Throwable {caught}) {{"));
        self.writer.indent();
        self.caught_names.push(caught.clone());
        let rendered = self.render_dispatch_chain(handlers, &caught);
        self.caught_names.pop();
        self.writer.dedent();
        self.writer.line("}");
        rendered
    }

    /// The ordered handler dispatch inside one rendered catch-all.
    fn render_dispatch_chain(
        &mut self,
        handlers: &[CatchHandler<Instruction>],
        caught: &str,
    ) -> Result<(), RenderFailure> {
        if handlers.is_empty() {
            self.writer.line(&format!(
                "throw {}({caught});",
                self.renderer.rethrow_name()
            ));
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
                        .renderer
                        .names()
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
            for child in &handler.body {
                self.render_ast(child)?;
            }
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
            self.writer.line(&format!(
                "throw {}({caught});",
                self.renderer.rethrow_name()
            ));
            self.writer.dedent();
        }
        if open_arms > 0 || !exhaustive {
            self.writer.line("}");
        }
        Ok(())
    }

    /// The catch type materialized at one handler's landing block.
    fn handler_catch_type(
        &self,
        handler: &CatchHandler<Instruction>,
    ) -> Result<CatchType, RenderFailure> {
        self.function
            .cfg()
            .block(handler.entry)
            .instructions()
            .first()
            .and_then(|instruction| match instruction.operation() {
                Operation::CaughtException(catch) => Some(catch.clone()),
                _ => None,
            })
            .ok_or_else(|| RenderFailure::new("a structured handler does not begin at its landing"))
    }

    /// How a plain cfglib break must be spelled here: labeled when Java
    /// would otherwise bind it to an intervening switch.
    fn naked_break_label(&self) -> Result<Option<String>, RenderFailure> {
        let mut crossed_switch = false;
        for frame in self.frames.iter().rev() {
            match frame {
                Frame::Switch => crossed_switch = true,
                Frame::Loop { label } => {
                    if !crossed_switch {
                        return Ok(None);
                    }
                    return label.clone().map(Some).ok_or_else(|| {
                        RenderFailure::new("a switch-crossing break found an unlabeled loop")
                    });
                }
            }
        }
        Ok(None)
    }

    fn render_loop(
        &mut self,
        label: Option<String>,
        header: BlockId,
        kind: &LoopKind<Instruction>,
        body: &[AstNode<Instruction>],
    ) -> Result<(), RenderFailure> {
        let label = label.or_else(|| {
            switch_break_needs_label(body).then(|| format!("cafe_loop_{}", header.index()))
        });
        match kind {
            LoopKind::While {
                condition,
                exit_on_true,
                ..
            } => {
                let (branch, rendered) = self.loop_condition(condition)?;
                let live_prefix = condition[..condition.len() - 1]
                    .iter()
                    .any(|instruction| !self.skipped.contains(&instruction.id()));
                if live_prefix {
                    // The condition needs statements every iteration, so it
                    // is evaluated inside the loop as an explicit exit test.
                    self.open_loop(
                        label.as_deref(),
                        "while (java.lang.Boolean.TRUE.booleanValue()) {",
                    );
                    for instruction in &condition[..condition.len() - 1] {
                        self.emit_instruction(instruction, true)?;
                    }
                    let start = self.writer.position();
                    let exit = if *exit_on_true {
                        rendered
                    } else {
                        format!("!({rendered})")
                    };
                    self.writer.line(&format!("if ({exit}) {{"));
                    self.writer.indent();
                    self.writer.line("break;");
                    self.writer.dedent();
                    self.writer.line("}");
                    self.map(branch, start, self.writer.position());
                } else {
                    let condition = if *exit_on_true {
                        format!("!({rendered})")
                    } else {
                        rendered
                    };
                    let start = self.writer.position();
                    self.open_loop(label.as_deref(), &format!("while ({condition}) {{"));
                    self.map(branch, start, self.writer.position());
                }
                self.render_loop_body(label, body)?;
                self.writer.line("}");
            }
            LoopKind::DoWhile {
                condition,
                continue_on_true,
                ..
            } => {
                let (branch, rendered) = self.loop_condition(condition)?;
                self.open_loop(label.as_deref(), "do {");
                self.render_loop_body(label, body)?;
                self.writer.indent();
                for instruction in &condition[..condition.len() - 1] {
                    self.emit_instruction(instruction, true)?;
                }
                self.writer.dedent();
                let repeat = if *continue_on_true {
                    rendered
                } else {
                    format!("!({rendered})")
                };
                let start = self.writer.position();
                self.writer.line(&format!("}} while ({repeat});"));
                self.map(branch, start, self.writer.position());
            }
            LoopKind::Endless => {
                self.open_loop(
                    label.as_deref(),
                    "while (java.lang.Boolean.TRUE.booleanValue()) {",
                );
                self.render_loop_body(label, body)?;
                self.writer.line("}");
            }
        }
        Ok(())
    }

    /// The branch closing a loop-condition witness and its rendered test.
    fn loop_condition(
        &self,
        condition: &[Instruction],
    ) -> Result<(InstructionId, String), RenderFailure> {
        let branch = condition
            .last()
            .ok_or_else(|| RenderFailure::new("structured loop condition has no branch"))?;
        let Operation::Branch(predicate) = branch.operation() else {
            return Err(RenderFailure::new(
                "structured loop condition ends in a non-branch instruction",
            ));
        };
        Ok((branch.id(), self.renderer.condition(branch, *predicate)))
    }

    fn open_loop(&mut self, label: Option<&str>, opening: &str) {
        match label {
            Some(label) => self.writer.line(&format!("{label}: {opening}")),
            None => self.writer.line(opening),
        }
        self.writer.indent();
    }

    /// Renders loop body children inside their loop frame; the caller has
    /// already indented and closes the construct afterwards.
    fn render_loop_body(
        &mut self,
        label: Option<String>,
        body: &[AstNode<Instruction>],
    ) -> Result<(), RenderFailure> {
        self.frames.push(Frame::Loop { label });
        let rendered = body.iter().try_for_each(|child| self.render_ast(child));
        self.frames.pop();
        self.writer.dedent();
        rendered
    }

    fn render_switch(
        &mut self,
        condition_instructions: &[Instruction],
        cases: &[mlil::cfglib::SwitchCase<Instruction>],
        default_body: &[AstNode<Instruction>],
    ) -> Result<(), RenderFailure> {
        let (dispatch, prefix) = condition_instructions
            .split_last()
            .ok_or_else(|| RenderFailure::new("structured switch has no dispatch"))?;
        if !matches!(dispatch.operation(), Operation::Switch(_)) {
            return Err(RenderFailure::new(
                "structured switch ends in a non-dispatch instruction",
            ));
        }
        // Every arm's keys live on its dispatch edges; resolve them before
        // rendering so a keyless arm degrades the whole method cleanly.
        let mut keyed_cases = Vec::new();
        for case in cases {
            let keys = case
                .edges
                .iter()
                .filter_map(
                    |&edge| match self.function.cfg().edge(edge).payload().role {
                        EdgeRole::SwitchCase(value) => Some(value),
                        _ => None,
                    },
                )
                .collect::<Vec<_>>();
            if keys.is_empty() {
                return Err(RenderFailure::new("structured switch arm has no case keys"));
            }
            keyed_cases.push((keys, &case.body));
        }
        for instruction in prefix {
            self.emit_instruction(instruction, true)?;
        }
        let start = self.writer.position();
        self.writer.line(&format!(
            "switch ({}) {{",
            self.renderer.switch_value(dispatch)
        ));
        self.map(dispatch.id(), start, self.writer.position());
        self.writer.indent();
        self.frames.push(Frame::Switch);
        let rendered = self.render_switch_arms(&keyed_cases, default_body);
        self.frames.pop();
        self.writer.dedent();
        self.writer.line("}");
        rendered
    }

    fn render_switch_arms(
        &mut self,
        cases: &[(Vec<i64>, &Vec<AstNode<Instruction>>)],
        default_body: &[AstNode<Instruction>],
    ) -> Result<(), RenderFailure> {
        for (keys, body) in cases {
            let labels = keys
                .iter()
                .map(|key| format!("case {key}:"))
                .collect::<Vec<_>>()
                .join(" ");
            self.writer.line(&format!("{labels} {{"));
            self.writer.indent();
            for child in *body {
                self.render_ast(child)?;
            }
            if completes_normally(body) {
                self.writer.line("break;");
            }
            self.writer.dedent();
            self.writer.line("}");
        }
        if !default_body.is_empty() {
            self.writer.line("default: {");
            self.writer.indent();
            for child in default_body {
                self.render_ast(child)?;
            }
            if completes_normally(default_body) {
                self.writer.line("break;");
            }
            self.writer.dedent();
            self.writer.line("}");
        }
        Ok(())
    }
}

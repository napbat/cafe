//! Structured and state-machine Java control-flow rendering.

use std::collections::BTreeSet;

use disassembler::CatchType;
use java::descriptor::{JavaType, ReturnType};
use mlil::cfglib::{AstNode, BlockId};
use mlil::{EdgeRole, EntityId, Function, Instruction, InstructionId, Operation};

use crate::diagnostic::{Diagnostic, DiagnosticCode, MethodIdentity};
use crate::model::{GeneratedSpan, SourceMapEntry};
use crate::names::{SourceNames, rust_string_literal};
use crate::options::{ControlFlowPreference, DecompilerOptions};
use crate::writer::SourceWriter;

use super::constructor::{ConstructorPrelude, fallback_invocation, recover as recover_constructor};
use super::instruction::{InstructionRenderer, RenderFailure};
use super::variables::VariableLayout;

pub(crate) struct RenderedBody {
    pub(crate) source: String,
    pub(crate) diagnostics: Vec<Diagnostic>,
    pub(crate) source_map: Vec<SourceMapEntry>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum BodyKind {
    InstanceMethod,
    StaticMethod,
    Constructor,
    EnumConstructor,
    ClassInitializer,
}

impl BodyKind {
    pub(crate) fn for_method(name: &str, instance: bool, enum_class: bool) -> Self {
        if name == java::classfile::INSTANCE_INITIALIZER_NAME {
            if enum_class {
                Self::EnumConstructor
            } else {
                Self::Constructor
            }
        } else if name == java::classfile::CLASS_INITIALIZER_NAME {
            Self::ClassInitializer
        } else if instance {
            Self::InstanceMethod
        } else {
            Self::StaticMethod
        }
    }

    const fn instance(self) -> bool {
        matches!(
            self,
            Self::InstanceMethod | Self::Constructor | Self::EnumConstructor
        )
    }

    const fn constructor(self) -> bool {
        matches!(self, Self::Constructor | Self::EnumConstructor)
    }

    const fn enum_constructor(self) -> bool {
        matches!(self, Self::EnumConstructor)
    }

    const fn class_initializer(self) -> bool {
        matches!(self, Self::ClassInitializer)
    }
}

pub(crate) struct BodyRequest<'a> {
    pub(crate) function: &'a Function,
    pub(crate) owner: &'a str,
    pub(crate) method: MethodIdentity,
    pub(crate) parameters: &'a [JavaType],
    pub(crate) parameter_names: &'a [String],
    pub(crate) return_type: &'a ReturnType,
    pub(crate) kind: BodyKind,
    pub(crate) options: &'a DecompilerOptions,
    pub(crate) rethrow: &'a str,
    pub(crate) names: &'a SourceNames,
}

pub(crate) fn render(request: &BodyRequest<'_>) -> RenderedBody {
    match try_render(request) {
        Ok(body) => body,
        Err(failure) => stub(request, &failure.message),
    }
}

fn try_render(request: &BodyRequest<'_>) -> Result<RenderedBody, RenderFailure> {
    let variables = VariableLayout::new(
        request.function,
        request.parameters,
        request.parameter_names,
        request.kind.instance(),
    );
    let renderer = InstructionRenderer::new(
        request.function,
        &variables,
        request.return_type,
        request.owner,
        request.rethrow,
        request.names,
        request.kind.class_initializer(),
    );
    preflight(request.function, &renderer)?;
    let prelude = constructor_prelude(request)?;
    let skipped = skipped_instructions(request.function, prelude.as_ref());

    let ast = request.function.structured_control_flow();
    let exceptional = request
        .function
        .cfg()
        .edges()
        .any(|edge| matches!(edge.payload().role, EdgeRole::Exception { .. }));
    let forced_state = request.options.control_flow == ControlFlowPreference::StateMachine;
    let structured = !forced_state && !exceptional && structured_java(&ast);
    let mut diagnostics = Vec::new();
    if !structured {
        diagnostics.push(Diagnostic::method_warning(
            DiagnosticCode::StateMachineFallback,
            request.owner,
            request.method.clone(),
            if exceptional {
                "exact ordered exception dispatch is represented by a Java state machine"
            } else if forced_state {
                "state-machine control flow was requested"
            } else {
                "cfglib retained an irreducible label/goto or non-Java region"
            },
        ));
    }

    let mut context = RenderContext {
        function: request.function,
        renderer,
        writer: SourceWriter::default(),
        source_map: Vec::new(),
        skipped,
        class_initializer: request.kind.class_initializer(),
    };
    if let Some(prelude) = prelude {
        context.emit_prelude(&prelude);
    }
    for declaration in variables.declarations(request.function, request.parameters) {
        context.writer.line(&declaration);
    }
    if !request.function.variables().is_empty() {
        context.writer.blank();
    }
    let guarded_result = structured && matches!(request.return_type, ReturnType::Type(_));
    let guarded_initializer =
        request.kind.class_initializer() && !has_explicit_return(request.function);
    let guarded_body = guarded_initializer || guarded_result;
    if guarded_body {
        context
            .writer
            .line("if (java.lang.Boolean.TRUE.booleanValue()) {");
        context.writer.indent();
    }
    if structured {
        context.render_ast(&ast)?;
    } else {
        context.render_state_machine()?;
    }
    if guarded_body {
        context.writer.dedent();
        context.writer.line("}");
    }
    if guarded_result {
        context.writer.line(
            "throw new java.lang.AssertionError(\"decompiled control flow completed without a result\");",
        );
    }
    Ok(RenderedBody {
        source: context.writer.finish(),
        diagnostics,
        source_map: context.source_map,
    })
}

fn constructor_prelude(
    request: &BodyRequest<'_>,
) -> Result<Option<ConstructorPrelude>, RenderFailure> {
    request
        .kind
        .constructor()
        .then(|| {
            recover_constructor(
                request.function,
                request.owner,
                request.parameters,
                request.parameter_names,
                request.names,
            )
            .map(|mut prelude| {
                if request.kind.enum_constructor() {
                    "super();".clone_into(&mut prelude.source);
                }
                prelude
            })
        })
        .transpose()
}

fn skipped_instructions(
    function: &Function,
    prelude: Option<&ConstructorPrelude>,
) -> BTreeSet<InstructionId> {
    let mut skipped = prelude.map_or_else(BTreeSet::new, |value| value.skipped.clone());
    for point in function.dead_code().instructions {
        if let Some(instruction) = function
            .cfg()
            .blocks()
            .get(point.block.index())
            .and_then(|block| block.instructions().get(point.inst_idx))
        {
            skipped.insert(instruction.id());
        }
    }
    skipped
}

fn has_explicit_return(function: &Function) -> bool {
    function.cfg().blocks().iter().any(|block| {
        block
            .instructions()
            .iter()
            .any(|instruction| matches!(instruction.operation(), Operation::Return))
    })
}

fn preflight(function: &Function, renderer: &InstructionRenderer<'_>) -> Result<(), RenderFailure> {
    for block in function.cfg().blocks() {
        for instruction in block.instructions() {
            renderer.statements(instruction)?;
            if let Operation::Branch(predicate) = instruction.operation() {
                let _ = renderer.condition(instruction, *predicate);
            }
        }
    }
    Ok(())
}

fn structured_java(node: &AstNode<Instruction>) -> bool {
    match node {
        AstNode::Block { .. } | AstNode::Return { .. } | AstNode::Break | AstNode::Continue => true,
        AstNode::Sequence { body } | AstNode::Loop { body, .. } => body.iter().all(structured_java),
        AstNode::IfThenElse {
            then_body,
            else_body,
            ..
        } => then_body.iter().all(structured_java) && else_body.iter().all(structured_java),
        AstNode::Switch { .. }
        | AstNode::Label { .. }
        | AstNode::Goto { .. }
        | AstNode::TryCatch { .. }
        | AstNode::Guarded { .. } => false,
    }
}

struct RenderContext<'a> {
    function: &'a Function,
    renderer: InstructionRenderer<'a>,
    writer: SourceWriter,
    source_map: Vec<SourceMapEntry>,
    skipped: BTreeSet<InstructionId>,
    class_initializer: bool,
}

impl RenderContext<'_> {
    fn emit_prelude(&mut self, prelude: &ConstructorPrelude) {
        let start = self.writer.position();
        self.writer.line(&prelude.source);
        self.map(prelude.instruction, start, self.writer.position());
    }

    fn render_ast(&mut self, node: &AstNode<Instruction>) -> Result<(), RenderFailure> {
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
            AstNode::Loop { body, .. } => {
                self.writer
                    .line("while (java.lang.Boolean.TRUE.booleanValue()) {");
                self.writer.indent();
                for child in body {
                    self.render_ast(child)?;
                }
                self.writer.dedent();
                self.writer.line("}");
            }
            AstNode::Break => self.writer.line("break;"),
            AstNode::Continue => self.writer.line("continue;"),
            AstNode::Switch { .. }
            | AstNode::Label { .. }
            | AstNode::Goto { .. }
            | AstNode::TryCatch { .. }
            | AstNode::Guarded { .. } => {
                return Err(RenderFailure::new(
                    "structured AST contains a non-Java control-flow node",
                ));
            }
        }
        Ok(())
    }

    fn render_state_machine(&mut self) -> Result<(), RenderFailure> {
        let entry_edge = self
            .function
            .cfg()
            .successor_edges(self.function.cfg().entry())
            .first()
            .copied()
            .ok_or_else(|| RenderFailure::new("MLIL root has no entry edge"))?;
        let first = self.function.cfg().edge(entry_edge).target();
        self.writer.line("java.lang.Throwable cafe_caught = null;");
        self.writer
            .line(&format!("int cafe_state = {};", first.index()));
        self.writer.line("cafe_dispatch: while (true) {");
        self.writer.indent();
        self.writer.line("switch (cafe_state) {");
        self.writer.indent();
        for block in self.function.cfg().blocks() {
            if block.id() == self.function.cfg().entry() {
                continue;
            }
            self.writer
                .line(&format!("case {}: {{", block.id().index()));
            self.writer.indent();
            for instruction in block.instructions() {
                self.emit_instruction(instruction, false)?;
            }
            self.emit_transition(block.id())?;
            self.writer.dedent();
            self.writer.line("}");
        }
        self.writer.line("default:");
        self.writer.indent();
        self.writer
            .line("throw new java.lang.AssertionError(\"invalid decompiler control-flow state\");");
        self.writer.dedent();
        self.writer.dedent();
        self.writer.line("}");
        self.writer.dedent();
        self.writer.line("}");
        Ok(())
    }

    fn emit_instruction(
        &mut self,
        instruction: &Instruction,
        structured: bool,
    ) -> Result<(), RenderFailure> {
        if self.skipped.contains(&instruction.id()) {
            return Ok(());
        }
        if self.class_initializer && matches!(instruction.operation(), Operation::Return) {
            if !structured {
                let start = self.writer.position();
                self.writer.line("break cafe_dispatch;");
                self.map(instruction.id(), start, self.writer.position());
            }
            return Ok(());
        }
        if matches!(
            instruction.operation(),
            Operation::Branch(_) | Operation::Switch(_) | Operation::Jump
        ) && !structured
        {
            return Ok(());
        }
        let statements = self.renderer.statements(instruction)?;
        if statements.is_empty() {
            return Ok(());
        }
        let start = self.writer.position();
        if instruction.may_throw() {
            self.writer.line("try {");
            self.writer.indent();
            for statement in statements {
                self.writer.line(&statement);
            }
            self.writer.dedent();
            let error = format!("cafe_error_{}", instruction.id().raw());
            self.writer
                .line(&format!("}} catch (java.lang.Throwable {error}) {{"));
            self.writer.indent();
            if structured {
                self.writer
                    .line(&format!("throw {}({error});", self.renderer.rethrow_name()));
            } else {
                self.emit_exception_dispatch(instruction, &error)?;
            }
            self.writer.dedent();
            self.writer.line("}");
        } else {
            for statement in statements {
                self.writer.line(&statement);
            }
        }
        self.map(instruction.id(), start, self.writer.position());
        Ok(())
    }

    fn emit_exception_dispatch(
        &mut self,
        instruction: &Instruction,
        error: &str,
    ) -> Result<(), RenderFailure> {
        let mut handlers = self
            .function
            .cfg()
            .successor_edges(
                self.function
                    .instruction_point(instruction.id())
                    .ok_or_else(|| RenderFailure::new("instruction has no graph point"))?
                    .block,
            )
            .iter()
            .filter_map(|&edge_id| {
                let edge = self.function.cfg().edge(edge_id);
                let EdgeRole::Exception {
                    catch,
                    handler_order,
                    ..
                } = &edge.payload().role
                else {
                    return None;
                };
                (edge.payload().throw_site == Some(instruction.id()))
                    .then(|| (*handler_order, catch, edge.target()))
            })
            .collect::<Vec<_>>();
        handlers.sort_by_key(|(order, _, _)| *order);
        for (_, catch, target) in handlers {
            match catch {
                CatchType::Any => {
                    self.writer.line(&format!("cafe_caught = {error};"));
                    self.writer
                        .line(&format!("cafe_state = {};", target.index()));
                    self.writer.line("continue cafe_dispatch;");
                    return Ok(());
                }
                CatchType::Type(descriptor) => {
                    let catch_type = self
                        .renderer
                        .names()
                        .type_descriptor(descriptor)
                        .map_err(|source| RenderFailure::new(source.to_string()))?;
                    self.writer
                        .line(&format!("if ({error} instanceof {catch_type}) {{"));
                    self.writer.indent();
                    self.writer.line(&format!("cafe_caught = {error};"));
                    self.writer
                        .line(&format!("cafe_state = {};", target.index()));
                    self.writer.line("continue cafe_dispatch;");
                    self.writer.dedent();
                    self.writer.line("}");
                }
            }
        }
        self.writer
            .line(&format!("throw {}({error});", self.renderer.rethrow_name()));
        Ok(())
    }

    fn emit_transition(&mut self, block: BlockId) -> Result<(), RenderFailure> {
        let terminator = self.function.cfg().block(block).instructions().last();
        match terminator.map(Instruction::operation) {
            Some(Operation::Return | Operation::Throw) => return Ok(()),
            Some(Operation::Branch(predicate)) => {
                let instruction = terminator.expect("branch terminator exists");
                let true_target =
                    self.target(block, |role| matches!(role, EdgeRole::BranchTrue))?;
                let false_target =
                    self.target(block, |role| matches!(role, EdgeRole::BranchFalse))?;
                let start = self.writer.position();
                self.writer.line(&format!(
                    "cafe_state = ({}) ? {} : {};",
                    self.renderer.condition(instruction, *predicate),
                    true_target.index(),
                    false_target.index()
                ));
                self.writer.line("continue cafe_dispatch;");
                self.map(instruction.id(), start, self.writer.position());
                return Ok(());
            }
            Some(Operation::Switch(keys)) => {
                let instruction = terminator.expect("switch terminator exists");
                let start = self.writer.position();
                self.writer.line(&format!(
                    "switch ({}) {{",
                    self.renderer.switch_value(instruction)
                ));
                self.writer.indent();
                for &key in keys {
                    let target = self.target(
                        block,
                        |role| matches!(role, EdgeRole::SwitchCase(value) if *value == key),
                    )?;
                    self.writer.line(&format!(
                        "case {key}: cafe_state = {}; break;",
                        target.index()
                    ));
                }
                let fallback =
                    self.target(block, |role| matches!(role, EdgeRole::SwitchDefault))?;
                self.writer.line(&format!(
                    "default: cafe_state = {}; break;",
                    fallback.index()
                ));
                self.writer.dedent();
                self.writer.line("}");
                self.writer.line("continue cafe_dispatch;");
                self.map(instruction.id(), start, self.writer.position());
                return Ok(());
            }
            Some(Operation::Jump) => {
                let target = self.target(block, |role| matches!(role, EdgeRole::Jump))?;
                self.writer
                    .line(&format!("cafe_state = {};", target.index()));
                self.writer.line("continue cafe_dispatch;");
                return Ok(());
            }
            _ => {}
        }
        let normal = self
            .function
            .cfg()
            .successor_edges(block)
            .iter()
            .filter_map(|&edge| {
                let edge = self.function.cfg().edge(edge);
                (!edge.payload().role.is_exception()).then_some(edge.target())
            })
            .collect::<Vec<_>>();
        match normal.as_slice() {
            [target] => {
                self.writer
                    .line(&format!("cafe_state = {};", target.index()));
                self.writer.line("continue cafe_dispatch;");
                Ok(())
            }
            [] => Err(RenderFailure::new(
                "non-terminating MLIL block has no ordinary successor",
            )),
            _ => Err(RenderFailure::new(
                "non-control MLIL block has multiple ordinary successors",
            )),
        }
    }

    fn target(
        &self,
        block: BlockId,
        predicate: impl Fn(&EdgeRole) -> bool,
    ) -> Result<BlockId, RenderFailure> {
        self.function
            .cfg()
            .successor_edges(block)
            .iter()
            .find_map(|&edge| {
                let edge = self.function.cfg().edge(edge);
                predicate(&edge.payload().role).then_some(edge.target())
            })
            .ok_or_else(|| RenderFailure::new("control instruction lacks its required edge"))
    }

    fn map(&mut self, instruction: InstructionId, start: usize, end: usize) {
        if start == end {
            return;
        }
        let native_ranges = self
            .function
            .provenance()
            .mappings_to(EntityId::Instruction(instruction))
            .map(|entry| entry.source)
            .collect();
        self.source_map.push(SourceMapEntry {
            generated: GeneratedSpan { start, end },
            function: self.function.source().clone(),
            instruction,
            native_ranges,
        });
    }
}

fn stub(request: &BodyRequest<'_>, message: &str) -> RenderedBody {
    let mut writer = SourceWriter::default();
    if request.kind.constructor() {
        let prelude = if request.kind.enum_constructor() {
            "super();".to_owned()
        } else {
            recover_constructor(
                request.function,
                request.owner,
                request.parameters,
                request.parameter_names,
                request.names,
            )
            .map(|prelude| prelude.source)
            .ok()
            .or_else(|| fallback_invocation(request.function, request.owner, request.names))
            .unwrap_or_else(|| "super();".to_owned())
        };
        writer.line(&prelude);
    }
    let throwing = format!(
        "throw new java.lang.UnsupportedOperationException({});",
        rust_string_literal(message)
    );
    if request.kind.class_initializer() {
        // JLS 8.7 rejects a static initializer that cannot complete normally.
        // This runtime-true guard keeps the conservative throwing stub legal
        // Java without pretending that the unsupported bytecode succeeds.
        writer.line("if (java.lang.Boolean.TRUE.booleanValue()) {");
        writer.indent();
        writer.line(&throwing);
        writer.dedent();
        writer.line("}");
    } else {
        writer.line(&throwing);
    }
    RenderedBody {
        source: writer.finish(),
        diagnostics: vec![Diagnostic::method_error(
            DiagnosticCode::UnsupportedSemantics,
            request.owner,
            request.method.clone(),
            message,
        )],
        source_map: Vec::new(),
    }
}

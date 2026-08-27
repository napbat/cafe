//! Expression-oriented Java rendering over cfglib's HLIL lift.
//!
//! The HLIL path renders inlined expression trees — real conditions,
//! nested arithmetic, argument expressions — instead of one statement per
//! MLIL instruction. Any construct without an exact Java form fails the
//! whole body cleanly, and the caller falls back to the statement-per-
//! instruction structured or state-machine renderers.

mod analysis;
mod control;
mod expression;

use std::collections::{BTreeMap, BTreeSet};

use java::descriptor::ReturnType;
use mlil::cfglib::ir::hlil::{
    EntityId, Expression, ExpressionId, ExpressionKind, LiftedFunction, StatementId, StatementKind,
};
use mlil::{
    AllocationKind, AllocationSite, ArrayAccess, Constant, Function, InstructionId, JavaDialect,
    Operation, ValueType, VariableId,
};

use crate::model::{GeneratedSpan, SourceMapEntry};
use crate::names::SourceNames;
use crate::writer::SourceWriter;

use super::control::{BodyRequest, RenderedBody, has_explicit_return};
use super::instruction::{
    RenderFailure, allocation_aliases, binary_symbol, method_symbol, reference_type_name,
};
use super::variables::VariableLayout;

use self::analysis::{completes_normally, return_forwardings};

/// The HLIL view of one Java-managed function.
pub(super) type HlilFunction = mlil::cfglib::ir::hlil::Function<JavaDialect>;

/// The corresponding MLIL variable of one HLIL variable: identities map
/// one-to-one by index, with HLIL temporaries extending the space.
pub(super) fn mlil_variable(variable: mlil::cfglib::ir::hlil::VariableId) -> VariableId {
    VariableId::from_raw(variable.raw())
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

/// Renders one lifted function as an expression-oriented Java body.
pub(super) fn render_body(
    request: &BodyRequest<'_>,
    lifted: &LiftedFunction<JavaDialect>,
) -> Result<RenderedBody, RenderFailure> {
    let forwarding = return_forwardings(&lifted.function);
    let variables = VariableLayout::new_hlil(&lifted.function, request, &forwarding.variables);
    let mut renderer = HlilRenderer {
        function: &lifted.function,
        mlil: request.function,
        variables: &variables,
        return_type: request.return_type,
        owner: request.owner,
        rethrow: request.rethrow,
        names: request.names,
        class_initializer: request.kind.class_initializer(),
        allocation_aliases: allocation_aliases(request.function),
        statement_instructions: statement_instructions(&lifted.function, lifted),
        writer: SourceWriter::default(),
        source_map: Vec::new(),
        caught_names: Vec::new(),
        caught_counter: 0,
        unchecked_calls: request.unchecked_calls,
        hierarchy: request.hierarchy,
        method_exceptions: request.method_exceptions,
        finally_skips: BTreeSet::new(),
        forwarded_assigns: forwarding.assigns,
        forwarded_returns: forwarding.returns,
        error_counter: 0,
    };
    let mut body = lifted.function.body();
    if request.kind.constructor() {
        // JLS §8.8.7: the explicit constructor invocation must be the
        // first statement, before any declarations.
        let Some((&delegation, rest)) = body.split_first() else {
            return Err(RenderFailure::new("constructor body is empty"));
        };
        if !renderer.is_constructor_delegation(delegation) {
            return Err(RenderFailure::new(
                "constructor body does not lead with its delegation",
            ));
        }
        renderer.emit_statement(delegation)?;
        body = rest;
    }
    if matches!(request.return_type, ReturnType::Void)
        && body
            .last()
            .is_some_and(|&statement| renderer.is_void_return(statement))
    {
        body = &body[..body.len() - 1];
    }
    for declaration in variables.declarations(request.parameters) {
        renderer.writer.line(&declaration);
    }
    if !lifted.function.variables().is_empty() {
        renderer.writer.blank();
    }
    let guarded_result = matches!(request.return_type, ReturnType::Type(_))
        && completes_normally(&lifted.function, lifted.function.body());
    let guarded_initializer =
        request.kind.class_initializer() && !has_explicit_return(request.function);
    let guarded_body = guarded_initializer || guarded_result;
    if guarded_body {
        renderer
            .writer
            .line("if (java.lang.Boolean.TRUE.booleanValue()) {");
        renderer.writer.indent();
    }
    renderer.emit_statements(body)?;
    if guarded_body {
        renderer.writer.dedent();
        renderer.writer.line("}");
    }
    if guarded_result {
        renderer.writer.line(
            "throw new java.lang.AssertionError(\"decompiled control flow completed without a result\");",
        );
    }
    Ok(RenderedBody {
        source: renderer.writer.finish(),
        diagnostics: Vec::new(),
        source_map: renderer.source_map,
    })
}

/// The direct expression roots referenced by one statement.
fn statement_expressions(kind: &StatementKind<JavaDialect>, out: &mut Vec<ExpressionId>) {
    match kind {
        StatementKind::Expression(expression) => out.push(*expression),
        StatementKind::Assign { target, value } => {
            out.push(*target);
            out.push(*value);
        }
        StatementKind::If { condition, .. }
        | StatementKind::While { condition, .. }
        | StatementKind::DoWhile { condition, .. }
        | StatementKind::Switch {
            scrutinee: condition,
            ..
        } => out.push(*condition),
        StatementKind::For { condition, .. } => out.extend(*condition),
        StatementKind::Return { values } => out.extend(values.iter().copied()),
        StatementKind::Region { operands, .. } => out.extend(operands.iter().copied()),
        StatementKind::Loop { .. }
        | StatementKind::Break { .. }
        | StatementKind::Continue { .. }
        | StatementKind::Labeled { .. }
        | StatementKind::Goto { .. }
        | StatementKind::Try { .. } => {}
    }
}

/// MLIL instructions grouped under the statement whose rendering carries
/// them — directly, or through an inlined expression.
fn statement_instructions(
    function: &HlilFunction,
    lifted: &LiftedFunction<JavaDialect>,
) -> BTreeMap<StatementId, Vec<InstructionId>> {
    let mut owner: BTreeMap<ExpressionId, StatementId> = BTreeMap::new();
    for statement in function.statements() {
        let mut stack = Vec::new();
        statement_expressions(statement.kind(), &mut stack);
        while let Some(id) = stack.pop() {
            owner.insert(id, statement.id());
            if let Some(ExpressionKind::Operation { operands, .. }) =
                function.expression(id).map(Expression::kind)
            {
                stack.extend(operands.iter().copied());
            }
        }
    }
    let mut grouped: BTreeMap<StatementId, Vec<InstructionId>> = BTreeMap::new();
    for (&instruction, entity) in &lifted.instructions {
        let statement = match entity {
            EntityId::Statement(statement) => Some(*statement),
            EntityId::Expression(expression) => owner.get(expression).copied(),
            EntityId::Variable(_) => None,
        };
        if let Some(statement) = statement {
            grouped.entry(statement).or_default().push(instruction);
        }
    }
    grouped
}

pub(super) struct HlilRenderer<'a> {
    function: &'a HlilFunction,
    mlil: &'a Function,
    variables: &'a VariableLayout,
    return_type: &'a ReturnType,
    owner: &'a str,
    rethrow: &'a str,
    names: &'a SourceNames,
    class_initializer: bool,
    allocation_aliases: BTreeMap<AllocationSite, BTreeSet<VariableId>>,
    statement_instructions: BTreeMap<StatementId, Vec<InstructionId>>,
    writer: SourceWriter,
    source_map: Vec<SourceMapEntry>,
    caught_names: Vec<String>,
    caught_counter: usize,
    /// Methods of the rendered class declaring no exceptions: calls to
    /// them provably cannot raise checked exceptions.
    unchecked_calls: &'a std::collections::BTreeSet<(String, String)>,
    /// Classpath relationships used to omit casts accepted by Java
    /// reference assignability.
    hierarchy: Option<&'a dyn java::analysis::ReferenceHierarchy>,
    /// Archive or classpath method declarations used to distinguish an
    /// unresolved call from one proven to declare no exceptions.
    method_exceptions: Option<&'a crate::environment::MethodExceptionCatalog>,
    /// Statements consumed by `finally` recovery: matched duplicate
    /// copies of a finally body never emit on their own.
    finally_skips: BTreeSet<StatementId>,
    /// Single-read assignments forwarded into the return that
    /// immediately follows them; the assignment never emits.
    forwarded_assigns: BTreeSet<StatementId>,
    /// Return statements whose value renders from a forwarded
    /// assignment's expression instead of the consumed variable read.
    forwarded_returns: BTreeMap<StatementId, ExpressionId>,
    error_counter: usize,
}

impl<'a> HlilRenderer<'a> {
    /// The compound form of a field store: `f = f <op> x` spells
    /// `f <op>= x` when the read names the same field on an equal
    /// receiver, matching the source construct javac desugared.
    fn field_compound(
        &self,
        field: &disassembler::Reference,
        receiver_operands: &[ExpressionId],
        value: ExpressionId,
    ) -> Result<Option<(mlil::BinaryOperator, ExpressionId)>, RenderFailure> {
        let ExpressionKind::Operation {
            operation: Operation::Binary(operator),
            operands,
        } = self.expression_kind(value)?.kind()
        else {
            return Ok(None);
        };
        if *operator == mlil::BinaryOperator::ReverseSubtract {
            return Ok(None);
        }
        let [read, operand] = operands.as_slice() else {
            return Ok(None);
        };
        let ExpressionKind::Operation {
            operation:
                Operation::Field {
                    access,
                    field: read_field,
                },
            operands: read_operands,
        } = self.expression_kind(*read)?.kind()
        else {
            return Ok(None);
        };
        let matches_place = matches!(
            access,
            mlil::FieldAccess::GetStatic | mlil::FieldAccess::GetInstance
        ) && read_field == field
            && read_operands.len() == receiver_operands.len()
            && read_operands
                .iter()
                .zip(receiver_operands)
                .all(|(&left, &right)| self.expressions_equal(left, right));
        Ok(matches_place.then_some((*operator, *operand)))
    }

    /// Whether the statement is the constructor's own delegation: a
    /// `<init>` invocation on the uninitialized `this`.
    fn is_constructor_delegation(&self, statement: StatementId) -> bool {
        let Ok(StatementKind::Expression(expression)) = self.statement_kind(statement) else {
            return false;
        };
        let Ok(expression) = self.expression_kind(*expression) else {
            return false;
        };
        let ExpressionKind::Operation {
            operation: Operation::Call { target, .. },
            operands,
        } = expression.kind()
        else {
            return false;
        };
        matches!(method_symbol(target), Ok((_, name, _)) if name.text == "<init>")
            && operands.first().is_some_and(|&receiver| {
                self.expression_kind(receiver).is_ok_and(|receiver| {
                    matches!(receiver.value_type(), ValueType::UninitializedThis(_))
                })
            })
    }

    fn is_void_return(&self, statement: StatementId) -> bool {
        matches!(
            self.statement_kind(statement),
            Ok(StatementKind::Return { values }) if values.is_empty()
        )
    }

    /// Whether this expression tree can raise a checked exception javac
    /// would force the caller to handle — the reason the
    /// `$cafe$rethrow` launder exists. Calls resolve conservatively,
    /// except targets on the rendered class itself that declare no
    /// exceptions, which provably cannot throw checked ones.
    fn launder_required(&self, id: ExpressionId) -> bool {
        let Ok(expression) = self.expression_kind(id) else {
            return true;
        };
        let ExpressionKind::Operation {
            operation,
            operands,
        } = expression.kind()
        else {
            return false;
        };
        let own = match operation {
            Operation::Call { target, .. } => !self.declares_no_exceptions(target),
            _ => false,
        };
        own || operands
            .iter()
            .any(|&operand| self.launder_required(operand))
    }

    /// Whether the call target is a method of the rendered class that
    /// declares no exceptions at all.
    fn declares_no_exceptions(&self, target: &disassembler::Reference) -> bool {
        match target.symbol.as_ref() {
            Some(disassembler::ReferenceSymbol::Method {
                owner,
                name,
                descriptor,
            }) => {
                self.method_exceptions.is_some_and(|catalog| {
                    catalog.declares_no_exceptions(owner, &name.text, descriptor)
                }) || (owner == self.owner
                    && self
                        .unchecked_calls
                        .iter()
                        .any(|(method, signature)| method == &name.text && signature == descriptor))
            }
            _ => false,
        }
    }

    /// The in-scope delivered-exception name.
    fn caught_name(&self) -> &str {
        self.caught_names
            .last()
            .map_or("cafe_caught", String::as_str)
    }

    /// Opens a checked-exception laundering wrapper when `wrap` is set; the
    /// wrapper rethrows the delivered throwable unchanged, so it is
    /// semantically transparent and exists only for javac's checked rules.
    fn open_wrapper(&mut self, wrap: bool) {
        if wrap {
            self.writer.line("try {");
            self.writer.indent();
        }
    }

    fn close_wrapper(&mut self, wrap: bool) {
        if !wrap {
            return;
        }
        self.writer.dedent();
        let error = format!("cafe_error_h{}", self.error_counter);
        self.error_counter += 1;
        self.writer
            .line(&format!("}} catch (java.lang.Throwable {error}) {{"));
        self.writer.indent();
        self.writer
            .line(&format!("throw {}({error});", self.rethrow));
        self.writer.dedent();
        self.writer.line("}");
    }

    fn write_lines(&mut self, lines: &[String], wrap: bool) {
        self.open_wrapper(wrap);
        for line in lines {
            self.writer.line(line);
        }
        self.close_wrapper(wrap);
    }

    fn statement_kind(
        &self,
        id: StatementId,
    ) -> Result<&'a StatementKind<JavaDialect>, RenderFailure> {
        self.function
            .statement(id)
            .map(mlil::cfglib::ir::hlil::Statement::kind)
            .ok_or_else(|| RenderFailure::new("HLIL statement identity is unresolvable"))
    }

    fn expression_kind(
        &self,
        id: ExpressionId,
    ) -> Result<&'a Expression<JavaDialect>, RenderFailure> {
        self.function
            .expression(id)
            .ok_or_else(|| RenderFailure::new("HLIL expression identity is unresolvable"))
    }

    fn emit_statements(&mut self, ids: &[StatementId]) -> Result<(), RenderFailure> {
        for &id in ids {
            if self.finally_skips.contains(&id) || self.forwarded_assigns.contains(&id) {
                continue;
            }
            self.emit_statement(id)?;
        }
        Ok(())
    }

    /// One simple `variable = value` statement without its semicolon, with
    /// compound-assignment recovery (`x = x + y` → `x += y`, `x++`); `None`
    /// for the special assignment forms (dual zero, skipped allocations).
    fn variable_assignment(
        &self,
        target: ExpressionId,
        value: ExpressionId,
    ) -> Result<Option<(String, bool)>, RenderFailure> {
        let target_node = self.expression_kind(target)?;
        let ExpressionKind::Variable(variable) = target_node.kind() else {
            return Ok(None);
        };
        let variable = mlil_variable(*variable);
        let target_type = target_node.value_type();
        let value_node = self.expression_kind(value)?;
        if matches!(
            value_node.kind(),
            ExpressionKind::Operation {
                operation: Operation::Allocate(AllocationKind::Object(_)),
                ..
            }
        ) || (matches!(target_type, ValueType::Zero)
            && matches!(value_node.kind(), ExpressionKind::Constant(_)))
        {
            return Ok(None);
        }
        let name = self.variables.value(variable, target_type);
        if let ExpressionKind::Operation {
            operation: Operation::Binary(operator),
            operands,
        } = value_node.kind()
            && !matches!(operator, mlil::BinaryOperator::ReverseSubtract)
            && let [left, right] = operands.as_slice()
            && let left_node = self.expression_kind(*left)?
            && let ExpressionKind::Variable(read) = left_node.kind()
            && self
                .variables
                .value(mlil_variable(*read), left_node.value_type())
                == name
        {
            // The compound form recomputes in the target's own slot type,
            // so it is exact for every slot kind.
            if matches!(
                operator,
                mlil::BinaryOperator::Add | mlil::BinaryOperator::Subtract
            ) && matches!(
                self.expression_kind(*right)?.kind(),
                ExpressionKind::Constant(Constant::Integer(1))
            ) {
                let symbol = if *operator == mlil::BinaryOperator::Add {
                    "++"
                } else {
                    "--"
                };
                return Ok(Some((format!("{name}{symbol}"), false)));
            }
            let rendered = self.render(*right)?;
            return Ok(Some((
                format!("{name} {}= {}", binary_symbol(*operator), rendered.text),
                rendered.calls,
            )));
        }
        let rendered = self.render(value)?;
        Ok(Some((
            format!(
                "{name} = {}",
                self.coerced_to_variable(&rendered, variable, target_type)
            ),
            rendered.calls,
        )))
    }

    fn emit_statement(&mut self, id: StatementId) -> Result<(), RenderFailure> {
        let start = self.writer.position();
        match self.statement_kind(id)? {
            StatementKind::Expression(expression) => self.emit_effect(*expression)?,
            StatementKind::Assign { target, value } => self.emit_assign(*target, *value)?,
            StatementKind::If {
                condition,
                then_body,
                else_body,
            } => self.emit_if(*condition, then_body, else_body)?,
            StatementKind::While { condition, body } => self.emit_while(*condition, body, None)?,
            StatementKind::DoWhile { body, condition } => {
                self.emit_do_while(body, *condition, None)?;
            }
            StatementKind::Loop { body } => self.emit_loop(body, None)?,
            StatementKind::Switch {
                scrutinee,
                cases,
                default_body,
            } => self.emit_switch(*scrutinee, cases, default_body)?,
            StatementKind::Break { label } => match label {
                Some(label) => {
                    let label = java_label(label);
                    self.writer.line(&format!("break {label};"));
                }
                None => self.writer.line("break;"),
            },
            StatementKind::Continue { label } => match label {
                Some(label) => {
                    let label = java_label(label);
                    self.writer.line(&format!("continue {label};"));
                }
                None => self.writer.line("continue;"),
            },
            StatementKind::Return { values } => self.emit_return(id, values)?,
            StatementKind::Labeled { label, body } => self.emit_labeled(label, body)?,
            StatementKind::Goto { .. } => {
                return Err(RenderFailure::new(
                    "irreducible control flow has no Java goto",
                ));
            }
            StatementKind::Try {
                body,
                handlers,
                finally_body,
            } => self.emit_try(body, handlers, finally_body)?,
            StatementKind::For {
                initializer,
                condition,
                update,
                body,
            } => self.emit_for(initializer, *condition, update, body)?,
            StatementKind::Region {
                operation,
                operands,
                body,
            } => self.emit_region(operation, operands, body)?,
        }
        self.map_statement(id, start);
        Ok(())
    }

    fn emit_effect(&mut self, id: ExpressionId) -> Result<(), RenderFailure> {
        let ExpressionKind::Operation {
            operation,
            operands,
        } = self.expression_kind(id)?.kind()
        else {
            return Err(RenderFailure::new(
                "a value expression cannot stand as a Java statement",
            ));
        };
        match operation {
            Operation::Throw => {
                let operand = *operands
                    .first()
                    .ok_or_else(|| RenderFailure::new("throw statement has no operand"))?;
                let value = self.render(operand)?;
                let line = format!(
                    "throw {}({});",
                    self.rethrow,
                    Self::cast_value(&value, "java.lang.Throwable")
                );
                let launder = self.launder_required(operand);
                self.write_lines(&[line], launder);
            }
            Operation::InitializeArray { array_type, values } => {
                self.emit_array_initializer(operands, array_type, values)?;
            }
            Operation::Call {
                kind,
                target,
                descriptor,
            } => {
                let (_, name, _) = method_symbol(target)?;
                if name.text == "<init>" {
                    self.emit_construction(target, descriptor.as_deref(), operands)?;
                } else {
                    let rendered = operands
                        .iter()
                        .map(|&operand| self.render(operand))
                        .collect::<Result<Vec<_>, _>>()?;
                    let (invocation, _) =
                        self.invocation(*kind, target, descriptor.as_deref(), &rendered)?;
                    let launder = !self.declares_no_exceptions(target)
                        || operands
                            .iter()
                            .any(|&operand| self.launder_required(operand));
                    self.write_lines(&[format!("{invocation};")], launder);
                }
            }
            _ => {
                return Err(RenderFailure::new(format!(
                    "HLIL effect `{}` has no Java statement policy",
                    operation.mnemonic()
                )));
            }
        }
        Ok(())
    }

    /// Per-element stores for one semantic array initializer.
    fn emit_array_initializer(
        &mut self,
        operands: &[ExpressionId],
        array_type: &mlil::ArrayType,
        values: &[Constant],
    ) -> Result<(), RenderFailure> {
        let array_operand = *operands
            .first()
            .ok_or_else(|| RenderFailure::new("array initializer has no array operand"))?;
        let array = self.render(array_operand)?;
        let array_type_name = self
            .names
            .type_descriptor(array_type.descriptor())
            .map_err(|error| RenderFailure::new(error.to_string()))?;
        let java::descriptor::JavaType::Array(element_type) =
            java::descriptor::parse_field(array_type.descriptor())
                .map_err(|error| RenderFailure::new(error.to_string()))?
        else {
            return Err(RenderFailure::new(
                "array initializer has a non-array descriptor",
            ));
        };
        let place = format!("(({array_type_name}) {})", array.object());
        let lines = values
            .iter()
            .enumerate()
            .map(|(index, value)| {
                Ok(format!(
                    "{place}[{index}] = {};",
                    super::instruction::constant_as_java_type(value, &element_type, self.names)?
                ))
            })
            .collect::<Result<Vec<_>, RenderFailure>>()?;
        let launder = self.launder_required(array_operand);
        self.write_lines(&lines, launder);
        Ok(())
    }

    /// One `<init>` invocation: the observable construction of the object
    /// allocated for its receiver, plus copies into allocation aliases.
    fn emit_construction(
        &mut self,
        target: &disassembler::Reference,
        descriptor: Option<&str>,
        operands: &[ExpressionId],
    ) -> Result<(), RenderFailure> {
        let descriptor =
            descriptor.ok_or_else(|| RenderFailure::new("constructor has no descriptor"))?;
        let parsed = java::descriptor::parse_method(descriptor)
            .map_err(|error| RenderFailure::new(error.to_string()))?;
        let rendered = operands
            .iter()
            .map(|&operand| self.render(operand))
            .collect::<Result<Vec<_>, _>>()?;
        let arguments = self.call_arguments(&parsed, 1, &rendered)?;
        let (owner, _, _) = method_symbol(target)?;
        let source_owner = reference_type_name(owner, self.names)?;
        let launder = !self.declares_no_exceptions(target)
            || operands
                .iter()
                .skip(1)
                .any(|&operand| self.launder_required(operand));
        let receiver = *operands
            .first()
            .ok_or_else(|| RenderFailure::new("constructor has no receiver"))?;
        let receiver = self.expression_kind(receiver)?;
        if matches!(receiver.value_type(), ValueType::UninitializedThis(_)) {
            // The constructor's own delegation: `this(...)` for the same
            // class, `super(...)` otherwise. javac guarantees it stands
            // first, so the plain statement order is already legal.
            let (owner, _, _) = method_symbol(target)?;
            let delegate = if owner == self.owner { "this" } else { "super" };
            if delegate == "super" && arguments.is_empty() {
                return Ok(());
            }
            // Never laundered: JLS §8.8.7 demands the delegation stand
            // alone as the first statement, and the constructor's own
            // `throws` clause already covers whatever it declares.
            self.write_lines(&[format!("{delegate}({arguments});")], false);
            return Ok(());
        }
        match receiver.kind() {
            ExpressionKind::Variable(variable) => {
                let ValueType::Uninitialized { site, .. } = receiver.value_type() else {
                    return Err(RenderFailure::new(
                        "constructor receiver is not an uninitialized allocation",
                    ));
                };
                let variable = mlil_variable(*variable);
                let mut lines = vec![format!(
                    "{} = new {source_owner}({arguments});",
                    self.variables.object(variable)
                )];
                if let Some(aliases) = self.allocation_aliases.get(site) {
                    for &alias in aliases {
                        // An alias with no surviving occurrence was fully
                        // inlined away: it is undeclared and unread.
                        if alias != variable
                            && self
                                .variables
                                .has_slot(alias, super::variables::SlotKind::Object)
                        {
                            let source = self.variables.object(variable);
                            let value = match (
                                self.variables.object_type(alias),
                                self.variables.object_type(variable),
                            ) {
                                (Some(declared), receiver) if receiver != Some(declared) => {
                                    format!("({declared}) (java.lang.Object) {source}")
                                }
                                _ => source,
                            };
                            lines.push(format!("{} = {value};", self.variables.object(alias)));
                        }
                    }
                }
                self.write_lines(&lines, launder);
                Ok(())
            }
            // A construction whose reference nothing reads afterwards: the
            // allocation inlined into its only consumer.
            ExpressionKind::Operation {
                operation: Operation::Allocate(AllocationKind::Object(_)),
                ..
            } => {
                self.write_lines(&[format!("new {source_owner}({arguments});")], launder);
                Ok(())
            }
            _ => Err(RenderFailure::new(
                "constructor receiver is neither a variable nor its allocation",
            )),
        }
    }

    fn emit_assign(
        &mut self,
        target: ExpressionId,
        value: ExpressionId,
    ) -> Result<(), RenderFailure> {
        let target_node = self.expression_kind(target)?;
        match target_node.kind() {
            ExpressionKind::Variable(variable) => {
                let variable = mlil_variable(*variable);
                let target_type = target_node.value_type().clone();
                if let ExpressionKind::Operation {
                    operation: Operation::Allocate(AllocationKind::Object(_)),
                    ..
                } = self.expression_kind(value)?.kind()
                {
                    // The uninitialized reference is unobservable until its
                    // `<init>`, whose statement assigns the constructed
                    // object to every alias of the allocation site; the
                    // declaration's `null` initializer covers definite
                    // assignment.
                    return Ok(());
                }
                if matches!(target_type, ValueType::Zero)
                    && matches!(
                        self.expression_kind(value)?.kind(),
                        ExpressionKind::Constant(_)
                    )
                {
                    // The Dalvik zero pattern is numeric zero and null at
                    // once; keep both slot views current.
                    let lines = [
                        format!("{} = 0;", self.variables.int(variable)),
                        format!("{} = null;", self.variables.object(variable)),
                    ];
                    self.write_lines(&lines, false);
                    return Ok(());
                }
                let (text, _) = self
                    .variable_assignment(target, value)?
                    .expect("the special assignment forms were handled above");
                let launder = self.launder_required(value);
                self.write_lines(&[format!("{text};")], launder);
                Ok(())
            }
            ExpressionKind::Operation {
                operation,
                operands,
            } => {
                match operation {
                    Operation::Array {
                        access: ArrayAccess::Get,
                        element,
                    } => {
                        let array = self.render(*operands.first().ok_or_else(|| {
                            RenderFailure::new("array store place has no array")
                        })?)?;
                        let index = self.render(*operands.get(1).ok_or_else(|| {
                            RenderFailure::new("array store place has no index")
                        })?)?;
                        let (place, element_type) = self.array_element(&array, &index, *element)?;
                        let rendered = self.render(value)?;
                        let line =
                            format!("{place} = {};", self.as_java_type(&rendered, &element_type));
                        let launder = operands
                            .iter()
                            .take(2)
                            .any(|&operand| self.launder_required(operand))
                            || self.launder_required(value);
                        self.write_lines(&[line], launder);
                        Ok(())
                    }
                    Operation::Field { access, field } => {
                        let receiver = operands
                            .first()
                            .map(|&operand| self.render(operand))
                            .transpose()?;
                        let (place, field_type) =
                            self.field_place(*access, field, receiver.as_ref())?;
                        let line = if let Some((operator, operand)) =
                            self.field_compound(field, operands, value)?
                        {
                            let rendered = self.render(operand)?;
                            format!(
                                "{place} {}= {};",
                                binary_symbol(operator),
                                rendered.nested()
                            )
                        } else {
                            let rendered = self.render(value)?;
                            format!("{place} = {};", self.as_java_type(&rendered, &field_type))
                        };
                        let launder = operands
                            .iter()
                            .any(|&operand| self.launder_required(operand))
                            || self.launder_required(value);
                        self.write_lines(&[line], launder);
                        Ok(())
                    }
                    _ => Err(RenderFailure::new(
                        "assignment target is not a Java place expression",
                    )),
                }
            }
            ExpressionKind::Constant(_) => {
                Err(RenderFailure::new("assignment target cannot be a constant"))
            }
        }
    }

    fn map_statement(&mut self, statement: StatementId, start: usize) {
        let end = self.writer.position();
        if start == end {
            return;
        }
        let Some(instructions) = self.statement_instructions.get(&statement) else {
            return;
        };
        for &instruction in instructions {
            let native_ranges = self
                .mlil
                .provenance()
                .mappings_to(mlil::EntityId::Instruction(instruction))
                .map(|entry| entry.source)
                .collect();
            self.source_map.push(SourceMapEntry {
                generated: GeneratedSpan { start, end },
                function: self.mlil.source().clone(),
                instruction,
                native_ranges,
            });
        }
    }
}

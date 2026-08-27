//! Java expression rendering over HLIL typed expression trees.

use java::descriptor::{JavaType, MethodDescriptor, ReturnType, parse_field, parse_method};
use mlil::cfglib::ir::hlil::{ExpressionId, ExpressionKind};
use mlil::{
    AllocationKind, ArrayAccess, BinaryOperator, BranchOperandKind, CallKind, Constant,
    ElementType, FieldAccess, Operation, Relation, ThreeWayComparison, ValueType,
};

use super::super::instruction::{
    RenderFailure, binary_symbol, constant_expression, element_array_descriptor,
    java_value_to_slot, method_symbol, new_array, reference_type, reference_type_name,
    relation_symbol, unary_symbol,
};
use super::HlilRenderer;
use super::mlil_variable;

/// One select arm classified as a boolean value.
enum BoolArm {
    /// The constant `1`.
    True,
    /// The constant `0`.
    False,
    /// A rendered boolean expression and whether it invokes methods.
    Value(String, bool),
}

/// One rendered expression occurrence: a slot-typed Java value expression
/// plus the alternate slot views a consumer may coerce it through.
pub(super) struct Rendered {
    pub(super) text: String,
    object: Option<String>,
    int: Option<String>,
    /// The raw Java boolean expression when `text` is its `(… ? 1 : 0)`
    /// slot wrap, so boolean consumers avoid a round trip through int.
    boolean: Option<String>,
    atomic: bool,
    /// Evaluation has no effects at all (variable and constant reads), so a
    /// consumer may evaluate it more than once.
    pure: bool,
    /// The tree invokes a method, so the carrying statement must launder
    /// checked exceptions through the rethrow helper.
    pub(super) calls: bool,
    /// The expression's exact static Java reference type, when known: the
    /// declared type for variables, the occurrence type otherwise. `None`
    /// means `java.lang.Object` (or a primitive).
    exact: Option<String>,
    pub(super) value_type: ValueType,
}

impl Rendered {
    pub(super) fn object(&self) -> &str {
        self.object.as_deref().unwrap_or(&self.text)
    }

    pub(super) fn int(&self) -> &str {
        self.int.as_deref().unwrap_or(&self.text)
    }

    pub(super) fn nested(&self) -> String {
        if self.atomic {
            self.text.clone()
        } else {
            format!("({})", self.text)
        }
    }

    fn nested_object(&self) -> String {
        match &self.object {
            Some(object) => object.clone(),
            None => self.nested(),
        }
    }

    fn nested_int(&self) -> String {
        match &self.int {
            Some(int) => int.clone(),
            None => self.nested(),
        }
    }
}

/// The Java spelling of one branch relation with inverted polarity.
pub(super) const fn inverted_relation(relation: Relation) -> Relation {
    match relation {
        Relation::Equal => Relation::NotEqual,
        Relation::NotEqual => Relation::Equal,
        Relation::Less => Relation::GreaterOrEqual,
        Relation::GreaterOrEqual => Relation::Less,
        Relation::Greater => Relation::LessOrEqual,
        Relation::LessOrEqual => Relation::Greater,
    }
}

impl HlilRenderer<'_> {
    pub(super) fn render(&self, id: ExpressionId) -> Result<Rendered, RenderFailure> {
        let expression = self
            .function
            .expression(id)
            .ok_or_else(|| RenderFailure::new("HLIL expression identity is unresolvable"))?;
        let value_type = expression.value_type().clone();
        match expression.kind() {
            ExpressionKind::Variable(variable) => {
                let variable = mlil_variable(*variable);
                Ok(Rendered {
                    text: self.variables.value(variable, &value_type),
                    object: Some(self.variables.object(variable)),
                    int: Some(self.variables.int(variable)),
                    boolean: None,
                    atomic: true,
                    pure: true,
                    calls: false,
                    exact: self.variables.object_type(variable).map(str::to_owned),
                    value_type,
                })
            }
            ExpressionKind::Constant(constant) => {
                if matches!(value_type, ValueType::Zero) {
                    return Ok(Rendered {
                        text: "0".to_owned(),
                        object: Some("null".to_owned()),
                        int: Some("0".to_owned()),
                        boolean: None,
                        atomic: true,
                        pure: true,
                        calls: false,
                        exact: None,
                        value_type,
                    });
                }
                let text = constant_expression(constant, self.names)?;
                let boolean = match constant {
                    Constant::Integer(0) => Some("false".to_owned()),
                    Constant::Integer(1) => Some("true".to_owned()),
                    _ => None,
                };
                Ok(Rendered {
                    atomic: !text.starts_with('-'),
                    text,
                    object: None,
                    int: None,
                    boolean,
                    pure: true,
                    calls: false,
                    exact: self.exact_reference(&value_type),
                    value_type,
                })
            }
            ExpressionKind::Operation {
                operation,
                operands,
            } => self.render_operation(operation, operands, value_type),
        }
    }

    fn render_operands(&self, operands: &[ExpressionId]) -> Result<Vec<Rendered>, RenderFailure> {
        operands
            .iter()
            .map(|&operand| self.render(operand))
            .collect()
    }

    fn render_operation(
        &self,
        operation: &Operation,
        operand_ids: &[ExpressionId],
        value_type: ValueType,
    ) -> Result<Rendered, RenderFailure> {
        if matches!(operation, Operation::Select) {
            return self.select_value(operand_ids, value_type);
        }
        let operands = self.render_operands(operand_ids)?;
        let calls = operands.iter().any(|operand| operand.calls);
        let value = |text: String, atomic: bool, calls: bool| Rendered {
            text,
            object: None,
            int: None,
            boolean: None,
            atomic,
            pure: false,
            calls,
            exact: self.exact_reference(&value_type),
            value_type: value_type.clone(),
        };
        let operand = |index: usize| -> Result<&Rendered, RenderFailure> {
            operands
                .get(index)
                .ok_or_else(|| RenderFailure::new("HLIL operation is missing an operand"))
        };
        Ok(match operation {
            Operation::Unary(operator) => value(
                format!("{}{}", unary_symbol(*operator), operand(0)?.nested()),
                false,
                calls,
            ),
            Operation::Binary(operator) => value(
                Self::binary_text(*operator, operand(0)?, operand(1)?),
                false,
                calls,
            ),
            Operation::Convert(conversion) => value(
                format!(
                    "({}) {}",
                    super::super::instruction::conversion_target(*conversion),
                    operand(0)?.nested()
                ),
                false,
                calls,
            ),
            Operation::Compare(comparison) => Self::compare(*comparison, &operands, value_type)?,
            Operation::CheckCast(reference) => value(
                Self::cast_value(operand(0)?, &reference_type(reference, self.names)?),
                false,
                calls,
            ),
            Operation::InstanceOf(reference) => {
                let tested = reference_type(reference, self.names)?;
                self.slot_value(
                    &format!(
                        "{} instanceof {tested}",
                        Self::comparable_object(operand(0)?, &tested)
                    ),
                    &JavaType::Boolean,
                    calls,
                    value_type.clone(),
                )
            }
            Operation::ArrayLength => value(
                format!(
                    "java.lang.reflect.Array.getLength({})",
                    operand(0)?.object()
                ),
                true,
                calls,
            ),
            Operation::Array {
                access: ArrayAccess::Get,
                element,
            } => {
                let (text, element_type) =
                    self.array_element(operand(0)?, operand(1)?, *element)?;
                self.slot_value(&text, &element_type, calls, value_type.clone())
            }
            Operation::Field {
                access: access @ (FieldAccess::GetStatic | FieldAccess::GetInstance),
                field,
            } => {
                let (place, field_type) = self.field_place(*access, field, operands.first())?;
                self.slot_value(&place, &field_type, calls, value_type.clone())
            }
            Operation::Call {
                kind,
                target,
                descriptor,
            } => self.call_value(*kind, target, descriptor.as_deref(), &operands, value_type)?,
            Operation::Allocate(kind) => self.allocation(kind, &operands, &value_type)?,
            Operation::CaughtException(_) => self.caught_value(value_type),
            _ => {
                return Err(RenderFailure::new(format!(
                    "HLIL operation `{}` has no Java expression form",
                    operation.mnemonic()
                )));
            }
        })
    }

    /// One value coerced from its definite Java type to its slot view,
    /// remembering the raw boolean form when the slot wraps one.
    fn slot_value(
        &self,
        raw: &str,
        java_type: &JavaType,
        calls: bool,
        value_type: ValueType,
    ) -> Rendered {
        let boolean = matches!(java_type, JavaType::Boolean).then(|| raw.to_owned());
        Rendered {
            text: java_value_to_slot(raw, java_type),
            object: None,
            int: None,
            boolean,
            atomic: true,
            pure: false,
            calls,
            exact: self.exact_reference(&value_type),
            value_type,
        }
    }

    /// The exact rendered Java type of one reference occurrence type.
    fn exact_reference(&self, value_type: &ValueType) -> Option<String> {
        let ValueType::Reference(Some(descriptor)) = value_type else {
            return None;
        };
        reference_type_name(descriptor, self.names).ok()
    }

    /// A value-position reference cast: elided when the static type already
    /// matches, direct from `java.lang.Object`, and routed through
    /// `java.lang.Object` between two named types (javac rejects casts
    /// between provably unrelated classes; the double cast never does).
    pub(super) fn cast_value(operand: &Rendered, type_name: &str) -> String {
        match operand.exact.as_deref() {
            Some(exact) if exact == type_name => operand.object().to_owned(),
            None | Some("java.lang.Object") => {
                format!("({type_name}) {}", operand.nested_object())
            }
            Some(_) if type_name == "java.lang.Object" => {
                format!("(java.lang.Object) {}", operand.nested_object())
            }
            Some(_) => format!(
                "({type_name}) (java.lang.Object) {}",
                operand.nested_object()
            ),
        }
    }

    /// One operand as an `instanceof` subject: a named static type may
    /// be provably unrelated to the tested type, so it widens to
    /// `java.lang.Object` first.
    fn comparable_object(operand: &Rendered, tested: &str) -> String {
        match operand.exact.as_deref() {
            None | Some("java.lang.Object") => operand.nested_object(),
            Some(exact) if exact == tested => operand.nested_object(),
            Some(_) => format!("((java.lang.Object) {})", operand.nested_object()),
        }
    }

    /// A postfix-safe reference cast for receiver and index positions.
    fn cast_receiver(operand: &Rendered, type_name: &str) -> String {
        match operand.exact.as_deref() {
            Some(exact) if exact == type_name => operand.nested_object(),
            None | Some("java.lang.Object") => {
                format!("(({type_name}) {})", operand.nested_object())
            }
            Some(_) if type_name == "java.lang.Object" => {
                format!("((java.lang.Object) {})", operand.nested_object())
            }
            Some(_) => format!(
                "(({type_name}) (java.lang.Object) {})",
                operand.nested_object()
            ),
        }
    }

    /// One recovered value selection: `condition ? when_true : when_false`
    /// with each arm coerced to the selection's own type.
    fn select_value(
        &self,
        operand_ids: &[ExpressionId],
        value_type: ValueType,
    ) -> Result<Rendered, RenderFailure> {
        let [condition, when_true, when_false] = operand_ids else {
            return Err(RenderFailure::new("select expects three operands"));
        };
        if let Some(rendered) =
            self.select_boolean(*condition, *when_true, *when_false, &value_type)?
        {
            return Ok(rendered);
        }
        let (condition, condition_calls) = self.condition(*condition, false)?;
        let when_true = self.render(*when_true)?;
        let when_false = self.render(*when_false)?;
        let reference = value_type.is_reference();
        let exact = self.exact_reference(&value_type);
        let arm = |operand: &Rendered| {
            if reference {
                match &exact {
                    Some(name) => Self::cast_value(operand, name),
                    None => operand.object().to_owned(),
                }
            } else {
                operand.nested()
            }
        };
        Ok(Rendered {
            text: format!("{condition} ? {} : {}", arm(&when_true), arm(&when_false)),
            object: None,
            int: None,
            boolean: None,
            atomic: false,
            pure: false,
            calls: condition_calls || when_true.calls || when_false.calls,
            exact,
            value_type,
        })
    }

    /// Recovers `&&`/`||` from a select over boolean-valued arms.
    ///
    /// javac compiles short-circuit operators as branch trees whose
    /// leaves load `0` or `1`, so a select whose arms are such constants
    /// or further boolean selects IS a boolean expression:
    /// `c ? b : 0` is `c && b`, `c ? 1 : b` is `c || b`, and inverted
    /// polarities negate the condition exactly. The result keeps the
    /// `(… ? 1 : 0)` slot wrap with the raw boolean underneath, exactly
    /// like `instanceof`.
    fn select_boolean(
        &self,
        condition: ExpressionId,
        when_true: ExpressionId,
        when_false: ExpressionId,
        value_type: &ValueType,
    ) -> Result<Option<Rendered>, RenderFailure> {
        let (Some(true_arm), Some(false_arm)) =
            (self.boolean_arm(when_true)?, self.boolean_arm(when_false)?)
        else {
            return Ok(None);
        };
        let (text, calls) = match (true_arm, false_arm) {
            (BoolArm::True, BoolArm::False) => self.condition(condition, false)?,
            (BoolArm::False, BoolArm::True) => self.condition(condition, true)?,
            (BoolArm::True, BoolArm::Value(arm, arm_calls)) => {
                let (test, test_calls) = self.condition(condition, false)?;
                (
                    format!("{} || {}", Self::guard(&test), Self::guard(&arm)),
                    test_calls || arm_calls,
                )
            }
            (BoolArm::Value(arm, arm_calls), BoolArm::False) => {
                let (test, test_calls) = self.condition(condition, false)?;
                (
                    format!("{} && {}", Self::guard(&test), Self::guard(&arm)),
                    test_calls || arm_calls,
                )
            }
            (BoolArm::False, BoolArm::Value(arm, arm_calls)) => {
                let (test, test_calls) = self.condition(condition, true)?;
                (
                    format!("{} && {}", Self::guard(&test), Self::guard(&arm)),
                    test_calls || arm_calls,
                )
            }
            (BoolArm::Value(arm, arm_calls), BoolArm::True) => {
                let (test, test_calls) = self.condition(condition, true)?;
                (
                    format!("{} || {}", Self::guard(&test), Self::guard(&arm)),
                    test_calls || arm_calls,
                )
            }
            (BoolArm::True, BoolArm::True)
            | (BoolArm::False, BoolArm::False)
            | (BoolArm::Value(..), BoolArm::Value(..)) => return Ok(None),
        };
        Ok(Some(self.slot_value(
            &text,
            &JavaType::Boolean,
            calls,
            value_type.clone(),
        )))
    }

    /// One select arm as a boolean value, when it provably is one.
    fn boolean_arm(&self, id: ExpressionId) -> Result<Option<BoolArm>, RenderFailure> {
        match self.expression_kind(id)?.kind() {
            ExpressionKind::Constant(Constant::Integer(1)) => Ok(Some(BoolArm::True)),
            ExpressionKind::Constant(Constant::Integer(0)) => Ok(Some(BoolArm::False)),
            ExpressionKind::Operation {
                operation: Operation::Select | Operation::InstanceOf(_),
                ..
            } => {
                let rendered = self.render(id)?;
                Ok(rendered
                    .boolean
                    .map(|boolean| BoolArm::Value(boolean, rendered.calls)))
            }
            _ => Ok(None),
        }
    }

    /// Parenthesizes a boolean term whose spelling would rebind under a
    /// surrounding short-circuit operator.
    pub(super) fn guard(term: &str) -> String {
        if term.contains(" && ") || term.contains(" || ") || term.contains(" ? ") {
            format!("({term})")
        } else {
            term.to_owned()
        }
    }

    /// The delivered exception of the enclosing rendered catch.
    fn caught_value(&self, value_type: ValueType) -> Rendered {
        let caught = self.caught_name().to_owned();
        Rendered {
            text: caught.clone(),
            object: Some(caught),
            int: None,
            boolean: None,
            atomic: true,
            pure: true,
            calls: false,
            exact: Some("java.lang.Throwable".to_owned()),
            value_type,
        }
    }

    /// One binary operator application in JVM operand order.
    fn binary_text(operator: BinaryOperator, left: &Rendered, right: &Rendered) -> String {
        if operator == BinaryOperator::ReverseSubtract {
            return format!("{} - {}", right.nested(), left.nested());
        }
        // Adding a negative literal spells as subtraction (`x + (-5)` is
        // javac's `x - 5`).
        if operator == BinaryOperator::Add
            && right.pure
            && let Some(magnitude) = right.text.strip_prefix('-')
            && magnitude
                .strip_suffix('L')
                .unwrap_or(magnitude)
                .chars()
                .all(|character| character.is_ascii_digit())
        {
            return format!("{} - {magnitude}", left.nested());
        }
        format!(
            "{} {} {}",
            left.nested(),
            binary_symbol(operator),
            right.nested()
        )
    }

    /// One value-producing invocation, slot-coerced to its return type.
    fn call_value(
        &self,
        kind: CallKind,
        target: &disassembler::Reference,
        descriptor: Option<&str>,
        operands: &[Rendered],
        value_type: ValueType,
    ) -> Result<Rendered, RenderFailure> {
        let (invocation, parsed) = self.invocation(kind, target, descriptor, operands)?;
        let ReturnType::Type(return_type) = &parsed.return_type else {
            return Err(RenderFailure::new(
                "a void call has no HLIL expression value",
            ));
        };
        Ok(self.slot_value(&invocation, return_type, true, value_type))
    }

    /// The exact three-way comparison. Float and double forms must evaluate
    /// each operand more than once, so they require effect-free operands.
    fn compare(
        comparison: ThreeWayComparison,
        operands: &[Rendered],
        value_type: ValueType,
    ) -> Result<Rendered, RenderFailure> {
        let (left, right) = (&operands[0], &operands[1]);
        let calls = left.calls || right.calls;
        if comparison == ThreeWayComparison::Long {
            return Ok(Rendered {
                text: format!("java.lang.Long.compare({}, {})", left.text, right.text),
                object: None,
                int: None,
                boolean: None,
                atomic: true,
                pure: false,
                calls,
                exact: None,
                value_type,
            });
        }
        if !(left.pure && right.pure) {
            return Err(RenderFailure::new(
                "a floating-point comparison duplicates an effectful operand",
            ));
        }
        let nan = match comparison {
            ThreeWayComparison::FloatNanLow | ThreeWayComparison::DoubleNanLow => -1,
            ThreeWayComparison::FloatNanHigh | ThreeWayComparison::DoubleNanHigh => 1,
            ThreeWayComparison::Long => 0,
        };
        let (left, right) = (&left.text, &right.text);
        Ok(Rendered {
            text: format!(
                "((java.lang.Double.isNaN((double) {left}) || java.lang.Double.isNaN((double) {right})) ? {nan} : ({left} < {right} ? -1 : ({left} > {right} ? 1 : 0)))"
            ),
            object: None,
            int: None,
            boolean: None,
            atomic: true,
            pure: true,
            calls: false,
            exact: None,
            value_type,
        })
    }

    /// The `array[index]` place and its element type.
    pub(super) fn array_element(
        &self,
        array: &Rendered,
        index: &Rendered,
        element: ElementType,
    ) -> Result<(String, JavaType), RenderFailure> {
        let descriptor = match &array.value_type {
            ValueType::Reference(Some(descriptor)) if descriptor.starts_with('[') => {
                descriptor.as_str()
            }
            _ => element_array_descriptor(element),
        };
        let array_type = self
            .names
            .type_descriptor(descriptor)
            .map_err(|error| RenderFailure::new(error.to_string()))?;
        let JavaType::Array(element_type) =
            parse_field(descriptor).map_err(|error| RenderFailure::new(error.to_string()))?
        else {
            return Err(RenderFailure::new(
                "array access has a non-array descriptor",
            ));
        };
        let place = format!(
            "{}[{}]",
            Self::cast_receiver(array, &array_type),
            index.int()
        );
        Ok((place, *element_type))
    }

    /// The field place expression and the field's Java type.
    pub(super) fn field_place(
        &self,
        access: FieldAccess,
        field: &disassembler::Reference,
        receiver: Option<&Rendered>,
    ) -> Result<(String, JavaType), RenderFailure> {
        let (raw_owner, name, descriptor) = super::super::instruction::field_symbol(field)?;
        let (name, changed) = crate::names::identifier(&name.text);
        if changed {
            return Err(RenderFailure::new(
                "field name is not expressible as a Java identifier",
            ));
        }
        let owner = self.names.class_name(raw_owner);
        let field_type =
            parse_field(descriptor).map_err(|error| RenderFailure::new(error.to_string()))?;
        let place = match access {
            FieldAccess::GetStatic | FieldAccess::PutStatic => {
                if raw_owner == self.owner && self.class_initializer {
                    name
                } else {
                    format!("{owner}.{name}")
                }
            }
            FieldAccess::GetInstance | FieldAccess::PutInstance => {
                let receiver = receiver
                    .ok_or_else(|| RenderFailure::new("instance field access has no receiver"))?;
                format!("{}.{name}", Self::cast_receiver(receiver, &owner))
            }
        };
        Ok((place, field_type))
    }

    /// The bare invocation text and parsed descriptor of one non-`<init>`
    /// call.
    pub(super) fn invocation(
        &self,
        kind: CallKind,
        target: &disassembler::Reference,
        descriptor: Option<&str>,
        operands: &[Rendered],
    ) -> Result<(String, MethodDescriptor), RenderFailure> {
        if kind == CallKind::Dynamic {
            return Err(RenderFailure::new(
                "dynamic call site requires bootstrap-aware Java reconstruction",
            ));
        }
        let (owner, name, _) = method_symbol(target)?;
        if name.text == "<init>" {
            return Err(RenderFailure::new(
                "constructor invocation is a statement, not a value",
            ));
        }
        let source_owner = reference_type_name(owner, self.names)?;
        let descriptor =
            descriptor.ok_or_else(|| RenderFailure::new("call has no effective descriptor"))?;
        let parsed =
            parse_method(descriptor).map_err(|error| RenderFailure::new(error.to_string()))?;
        let receiver_count = usize::from(!matches!(kind, CallKind::Static));
        let arguments = self.call_arguments(&parsed, receiver_count, operands)?;
        let (method_name, changed) = crate::names::identifier(&name.text);
        if changed {
            return Err(RenderFailure::new(
                "method name is not expressible as a Java identifier",
            ));
        }
        let invocation = match kind {
            CallKind::Static => format!("{source_owner}.{method_name}({arguments})"),
            CallKind::Super => format!("super.{method_name}({arguments})"),
            CallKind::Virtual | CallKind::Interface | CallKind::Direct | CallKind::Polymorphic => {
                let receiver = operands
                    .first()
                    .ok_or_else(|| RenderFailure::new("instance call has no receiver"))?;
                format!(
                    "{}.{method_name}({arguments})",
                    Self::cast_receiver(receiver, &source_owner)
                )
            }
            CallKind::Dynamic => unreachable!(),
        };
        Ok((invocation, parsed))
    }

    /// Descriptor-typed argument list text for one call.
    pub(super) fn call_arguments(
        &self,
        parsed: &MethodDescriptor,
        receiver_count: usize,
        operands: &[Rendered],
    ) -> Result<String, RenderFailure> {
        Ok(parsed
            .parameters
            .iter()
            .enumerate()
            .map(|(index, parameter)| {
                operands
                    .get(index + receiver_count)
                    .ok_or_else(|| RenderFailure::new("call is missing an argument"))
                    .map(|operand| self.as_java_type(operand, parameter))
            })
            .collect::<Result<Vec<_>, _>>()?
            .join(", "))
    }

    fn allocation(
        &self,
        kind: &AllocationKind,
        operands: &[Rendered],
        value_type: &ValueType,
    ) -> Result<Rendered, RenderFailure> {
        let value = |text: String, atomic: bool| Rendered {
            text,
            object: None,
            int: None,
            boolean: None,
            atomic,
            pure: false,
            calls: operands.iter().any(|operand| operand.calls),
            exact: self.exact_reference(value_type),
            value_type: value_type.clone(),
        };
        Ok(match kind {
            // The reference stays null until `<init>` runs; the constructor
            // statement materializes the observable object.
            AllocationKind::Object(_) => value("null".to_owned(), true),
            AllocationKind::Array {
                array_type,
                dimensions,
            } => {
                let lengths = (0..usize::from(*dimensions))
                    .map(|index| {
                        operands
                            .get(index)
                            .map(|operand| operand.int().to_owned())
                            .ok_or_else(|| {
                                RenderFailure::new("array allocation is missing a length")
                            })
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                value(
                    new_array(array_type.descriptor(), &lengths, self.names)?,
                    true,
                )
            }
            AllocationKind::InitializedArray { array_type } => {
                let element = parse_field(array_type.descriptor())
                    .map_err(|error| RenderFailure::new(error.to_string()))?;
                let JavaType::Array(element) = element else {
                    return Err(RenderFailure::new(
                        "initialized-array descriptor is not an array",
                    ));
                };
                let values = operands
                    .iter()
                    .map(|operand| self.as_java_type(operand, &element))
                    .collect::<Vec<_>>()
                    .join(", ");
                value(
                    format!("new {}[] {{{values}}}", self.names.value_type(&element)),
                    true,
                )
            }
        })
    }

    /// One operand coerced from its slot view to a definite Java type.
    pub(super) fn as_java_type(&self, operand: &Rendered, target: &JavaType) -> String {
        match target {
            JavaType::Boolean => operand
                .boolean
                .clone()
                .unwrap_or_else(|| format!("{} != 0", operand.int())),
            JavaType::Byte => format!("(byte) {}", operand.nested_int()),
            JavaType::Char => format!("(char) {}", operand.nested_int()),
            JavaType::Short => format!("(short) {}", operand.nested_int()),
            JavaType::Int => operand.int().to_owned(),
            JavaType::Long | JavaType::Float | JavaType::Double => operand.text.clone(),
            JavaType::Object(_) | JavaType::Array(_) => self.reference_value(operand, target),
        }
    }

    /// A reference value in a descriptor-declared Java context. Exact
    /// classpath assignability makes a source cast unnecessary; unresolved
    /// relationships retain the conservative double-cast fallback.
    fn reference_value(&self, operand: &Rendered, target: &JavaType) -> String {
        let target_name = self.names.value_type(target);
        if operand.exact.is_some()
            && let Some(hierarchy) = self.hierarchy
            && let (Some(source), Some(target)) = (
                value_reference_key(&operand.value_type),
                java_reference_key(target),
            )
            && hierarchy.is_assignable(source, &target)
        {
            return operand.object().to_owned();
        }
        Self::cast_value(operand, &target_name)
    }

    /// One operand coerced to an assignment target's slot view and
    /// declared type.
    pub(super) fn coerced_to_variable(
        &self,
        value: &Rendered,
        variable: mlil::VariableId,
        target_type: &ValueType,
    ) -> String {
        if !target_type.is_reference() {
            return value.text.clone();
        }
        if value.object() == "null" {
            return "null".to_owned();
        }
        match self.variables.object_type(variable) {
            None | Some("java.lang.Object") => value.object().to_owned(),
            Some(declared) => Self::cast_value(value, declared),
        }
    }

    /// A Java boolean condition for one HLIL condition expression, with the
    /// requested polarity. Returns the text and whether it invokes methods.
    pub(super) fn condition(
        &self,
        id: ExpressionId,
        negated: bool,
    ) -> Result<(String, bool), RenderFailure> {
        let expression = self
            .function
            .expression(id)
            .ok_or_else(|| RenderFailure::new("HLIL condition identity is unresolvable"))?;
        if let ExpressionKind::Operation {
            operation: Operation::Branch(predicate),
            operands,
        } = expression.kind()
        {
            let operands = self.render_operands(operands)?;
            let calls = operands.iter().any(|operand| operand.calls);
            let relation = if negated {
                inverted_relation(predicate.relation)
            } else {
                predicate.relation
            };
            let relation = relation_symbol(relation);
            let operand = |index: usize| -> Result<&Rendered, RenderFailure> {
                operands
                    .get(index)
                    .ok_or_else(|| RenderFailure::new("branch condition is missing an operand"))
            };
            let text = match predicate.operands {
                BranchOperandKind::IntegerZero | BranchOperandKind::Boolean => {
                    let tested = operand(0)?;
                    match (&tested.boolean, relation) {
                        // A wrapped boolean tested against zero unwraps.
                        (Some(boolean), "!=") => boolean.clone(),
                        (Some(boolean), "==") => format!("!({boolean})"),
                        _ => format!("{} {relation} 0", tested.nested_int()),
                    }
                }
                BranchOperandKind::IntegerPair => {
                    format!(
                        "{} {relation} {}",
                        operand(0)?.nested_int(),
                        operand(1)?.nested_int()
                    )
                }
                BranchOperandKind::ReferencePair => {
                    let (left, right) = (operand(0)?, operand(1)?);
                    // Two differently named static types may be provably
                    // unrelated, which javac rejects for `==`; widening one
                    // side to `java.lang.Object` is always comparable.
                    let widen = matches!(
                        (left.exact.as_deref(), right.exact.as_deref()),
                        (Some(first), Some(second))
                            if first != second
                                && first != "java.lang.Object"
                                && second != "java.lang.Object"
                    );
                    let left = if widen {
                        format!("(java.lang.Object) {}", left.nested_object())
                    } else {
                        left.nested_object()
                    };
                    format!("{left} {relation} {}", right.nested_object())
                }
                BranchOperandKind::ReferenceNull => {
                    format!("{} {relation} null", operand(0)?.nested_object())
                }
            };
            return Ok((text, calls));
        }
        let rendered = self.render(id)?;
        let text = match (&rendered.boolean, negated) {
            (Some(boolean), false) => boolean.clone(),
            (Some(boolean), true) => format!("!({boolean})"),
            (None, false) => format!("{} != 0", rendered.int()),
            (None, true) => format!("{} == 0", rendered.int()),
        };
        Ok((text, rendered.calls))
    }
}

/// JVM hierarchy key for an exact MLIL reference occurrence: internal names
/// for objects and descriptors for arrays.
fn value_reference_key(value_type: &ValueType) -> Option<&str> {
    let descriptor = match value_type {
        ValueType::Reference(Some(descriptor)) | ValueType::UninitializedThis(descriptor) => {
            descriptor
        }
        ValueType::Uninitialized { descriptor, .. } => descriptor,
        _ => return None,
    };
    descriptor_reference_key(descriptor)
}

fn descriptor_reference_key(descriptor: &str) -> Option<&str> {
    if descriptor.starts_with('[') {
        Some(descriptor)
    } else {
        descriptor
            .strip_prefix('L')
            .and_then(|name| name.strip_suffix(';'))
    }
}

fn java_reference_key(value: &JavaType) -> Option<String> {
    match value {
        JavaType::Object(name) => Some(name.clone()),
        JavaType::Array(_) => Some(java_type_descriptor(value)),
        _ => None,
    }
}

fn java_type_descriptor(value: &JavaType) -> String {
    match value {
        JavaType::Byte => "B".to_owned(),
        JavaType::Char => "C".to_owned(),
        JavaType::Double => "D".to_owned(),
        JavaType::Float => "F".to_owned(),
        JavaType::Int => "I".to_owned(),
        JavaType::Long => "J".to_owned(),
        JavaType::Short => "S".to_owned(),
        JavaType::Boolean => "Z".to_owned(),
        JavaType::Object(name) => format!("L{name};"),
        JavaType::Array(element) => format!("[{}", java_type_descriptor(element)),
    }
}

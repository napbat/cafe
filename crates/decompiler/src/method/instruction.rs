//! Java statements for individual semantic MLIL instructions.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use disassembler::{ExactText, Reference, ReferenceSymbol};
use java::descriptor::{JavaType, ReturnType, parse_field, parse_method};
use mlil::{
    AllocationKind, AllocationSite, ArrayAccess, BinaryOperator, BranchOperandKind,
    BranchPredicate, CallKind, Constant, Conversion, ElementType, FieldAccess, Function,
    Instruction, MonitorAction, Operation, Relation, ThreeWayComparison, UnaryOperator, ValueType,
    VariableId,
};

use crate::names::{SourceNames, identifier, rust_string_literal, string_literal};

use super::variables::{SlotKind, VariableLayout, java_kind};

#[derive(Debug)]
pub(super) struct RenderFailure {
    pub(super) message: String,
}

impl RenderFailure {
    pub(super) fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

pub(super) struct InstructionRenderer<'a> {
    function: &'a Function,
    variables: &'a VariableLayout,
    return_type: &'a ReturnType,
    owner: &'a str,
    rethrow: &'a str,
    names: &'a SourceNames,
    class_initializer: bool,
    allocation_aliases: BTreeMap<AllocationSite, BTreeSet<VariableId>>,
}

impl<'a> InstructionRenderer<'a> {
    pub(super) fn new(
        function: &'a Function,
        variables: &'a VariableLayout,
        return_type: &'a ReturnType,
        owner: &'a str,
        rethrow: &'a str,
        names: &'a SourceNames,
        class_initializer: bool,
    ) -> Self {
        Self {
            function,
            variables,
            return_type,
            owner,
            rethrow,
            names,
            class_initializer,
            allocation_aliases: allocation_aliases(function),
        }
    }

    pub(super) fn statements(
        &self,
        instruction: &Instruction,
        caught: &str,
    ) -> Result<Vec<String>, RenderFailure> {
        let uses = instruction.uses();
        let defs = instruction.defs();
        let lines = match instruction.operation() {
            Operation::Nop
            | Operation::Discard
            | Operation::Jump
            | Operation::Branch(_)
            | Operation::Switch(_) => Vec::new(),
            Operation::Copy => vec![self.assignment(instruction, 0, 0)?],
            Operation::ParallelCopy => self.parallel_copy(instruction),
            Operation::TypeRefine => self.type_refine(instruction),
            Operation::Constant(constant) => self.constant(instruction, constant)?,
            Operation::Unary(operator) => vec![format!(
                "{} = {}{};",
                self.definition(instruction, 0)?,
                unary_symbol(*operator),
                self.use_value(instruction, 0)?
            )],
            Operation::Binary(operator) => vec![self.binary(instruction, *operator)?],
            Operation::Convert(conversion) => vec![self.convert(instruction, *conversion)?],
            Operation::Compare(comparison) => vec![self.compare(instruction, *comparison)?],
            Operation::Return => vec![self.return_statement(instruction)?],
            Operation::Throw => vec![format!(
                "throw {}((java.lang.Throwable) {});",
                self.rethrow,
                self.object_use(instruction, 0)?
            )],
            Operation::Array { access, element } => {
                vec![self.array(instruction, *access, *element)?]
            }
            Operation::ArrayLength => vec![format!(
                "{} = java.lang.reflect.Array.getLength({});",
                self.definition(instruction, 0)?,
                self.object_use(instruction, 0)?
            )],
            Operation::Field { access, field } => vec![self.field(instruction, *access, field)?],
            Operation::Call {
                kind,
                target,
                descriptor,
            } => self.call(instruction, *kind, target, descriptor.as_deref())?,
            Operation::Allocate(kind) => self.allocate(instruction, kind)?,
            Operation::InitializeArray { array_type, values } => {
                self.initialize_array(instruction, array_type.descriptor(), values)?
            }
            Operation::CheckCast(reference) => vec![format!(
                "{} = ({}) {};",
                self.definition(instruction, 0)?,
                reference_type(reference, self.names)?,
                self.object_use(instruction, 0)?
            )],
            Operation::InstanceOf(reference) => vec![format!(
                "{} = ({} instanceof {} ? 1 : 0);",
                self.definition(instruction, 0)?,
                self.object_use(instruction, 0)?,
                reference_type(reference, self.names)?
            )],
            Operation::CaughtException(_) => {
                vec![format!("{} = {caught};", self.definition(instruction, 0)?)]
            }
            Operation::Monitor(MonitorAction::Enter | MonitorAction::Exit) => {
                return Err(RenderFailure::new(
                    "unpaired monitor operations require synchronized-region recovery",
                ));
            }
            Operation::Intrinsic(name) => {
                return Err(RenderFailure::new(format!(
                    "intrinsic `{name}` has no Java source policy"
                )));
            }
            Operation::Select => {
                return Err(RenderFailure::new(
                    "select is HLIL-only vocabulary; the expression renderer owns it",
                ));
            }
        };
        let _ = (uses, defs, self.function, self.owner);
        Ok(lines)
    }

    pub(super) fn condition(
        &self,
        instruction: &Instruction,
        predicate: BranchPredicate,
    ) -> String {
        let relation = relation_symbol(predicate.relation);
        match predicate.operands {
            BranchOperandKind::IntegerZero | BranchOperandKind::Boolean => {
                format!("{} {relation} 0", self.variables.int(instruction.uses()[0]))
            }
            BranchOperandKind::IntegerPair => format!(
                "{} {relation} {}",
                self.variables.int(instruction.uses()[0]),
                self.variables.int(instruction.uses()[1])
            ),
            BranchOperandKind::ReferencePair => format!(
                "{} {relation} {}",
                self.variables.object(instruction.uses()[0]),
                self.variables.object(instruction.uses()[1])
            ),
            BranchOperandKind::ReferenceNull => format!(
                "{} {relation} null",
                self.variables.object(instruction.uses()[0])
            ),
        }
    }

    pub(super) fn switch_value(&self, instruction: &Instruction) -> String {
        self.variables.int(instruction.uses()[0])
    }

    pub(super) const fn rethrow_name(&self) -> &str {
        self.rethrow
    }

    pub(super) const fn names(&self) -> &SourceNames {
        self.names
    }

    fn assignment(
        &self,
        instruction: &Instruction,
        def: usize,
        usage: usize,
    ) -> Result<String, RenderFailure> {
        let destination = self.definition(instruction, def)?;
        let value = self.coerced_use(instruction, usage, &instruction.def_types()[def])?;
        Ok(format!("{destination} = {value};"))
    }

    fn parallel_copy(&self, instruction: &Instruction) -> Vec<String> {
        let mut lines = Vec::new();
        for (position, (&variable, value_type)) in instruction
            .uses()
            .iter()
            .zip(instruction.use_types())
            .enumerate()
        {
            lines.push(format!(
                "{} cafe_copy_{}_{} = {};",
                slot_java_type(self.variables.kind(value_type)),
                instruction.id().raw(),
                position,
                self.variables.value(variable, value_type)
            ));
        }
        for (position, (&variable, value_type)) in instruction
            .defs()
            .iter()
            .zip(instruction.def_types())
            .enumerate()
        {
            lines.push(format!(
                "{} = cafe_copy_{}_{};",
                self.variables.value(variable, value_type),
                instruction.id().raw(),
                position
            ));
        }
        lines
    }

    fn type_refine(&self, instruction: &Instruction) -> Vec<String> {
        instruction
            .uses()
            .iter()
            .zip(instruction.defs())
            .map(|(&source, &destination)| {
                format!(
                    "{} = {};",
                    self.variables.object(destination),
                    self.variables.object(source)
                )
            })
            .collect()
    }

    fn constant(
        &self,
        instruction: &Instruction,
        constant: &Constant,
    ) -> Result<Vec<String>, RenderFailure> {
        let definition = instruction.defs()[0];
        if matches!(instruction.def_types()[0], ValueType::Zero) {
            return Ok(vec![
                format!("{} = 0;", self.variables.int(definition)),
                format!("{} = null;", self.variables.object(definition)),
            ]);
        }
        Ok(vec![format!(
            "{} = {};",
            self.definition(instruction, 0)?,
            constant_expression(constant, self.names)?
        )])
    }

    fn binary(
        &self,
        instruction: &Instruction,
        operator: BinaryOperator,
    ) -> Result<String, RenderFailure> {
        let left = self.use_value(instruction, 0)?;
        let right = self.use_value(instruction, 1)?;
        let expression = if operator == BinaryOperator::ReverseSubtract {
            format!("{right} - {left}")
        } else {
            format!("{left} {} {right}", binary_symbol(operator))
        };
        Ok(format!(
            "{} = {expression};",
            self.definition(instruction, 0)?
        ))
    }

    fn convert(
        &self,
        instruction: &Instruction,
        conversion: Conversion,
    ) -> Result<String, RenderFailure> {
        Ok(format!(
            "{} = ({}) {};",
            self.definition(instruction, 0)?,
            conversion_target(conversion),
            self.use_value(instruction, 0)?
        ))
    }

    fn compare(
        &self,
        instruction: &Instruction,
        comparison: ThreeWayComparison,
    ) -> Result<String, RenderFailure> {
        let left = self.use_value(instruction, 0)?;
        let right = self.use_value(instruction, 1)?;
        let nan = match comparison {
            ThreeWayComparison::FloatNanLow | ThreeWayComparison::DoubleNanLow => -1,
            ThreeWayComparison::FloatNanHigh | ThreeWayComparison::DoubleNanHigh => 1,
            ThreeWayComparison::Long => 0,
        };
        let expression = if comparison == ThreeWayComparison::Long {
            format!("java.lang.Long.compare({left}, {right})")
        } else {
            format!(
                "((java.lang.Double.isNaN((double) {left}) || java.lang.Double.isNaN((double) {right})) ? {nan} : ({left} < {right} ? -1 : ({left} > {right} ? 1 : 0)))"
            )
        };
        Ok(format!(
            "{} = {expression};",
            self.definition(instruction, 0)?
        ))
    }

    fn return_statement(&self, instruction: &Instruction) -> Result<String, RenderFailure> {
        match self.return_type {
            ReturnType::Void => Ok("return;".to_owned()),
            ReturnType::Type(value) => Ok(format!(
                "return {};",
                self.use_as_java_type(instruction, 0, value)?
            )),
        }
    }

    fn array(
        &self,
        instruction: &Instruction,
        access: ArrayAccess,
        element: ElementType,
    ) -> Result<String, RenderFailure> {
        let descriptor = match instruction.use_types().first() {
            Some(ValueType::Reference(Some(descriptor))) if descriptor.starts_with('[') => {
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
        let array = format!("(({array_type}) {})", self.object_use(instruction, 0)?);
        let index = self.variables.int(instruction.uses()[1]);
        match access {
            ArrayAccess::Get => Ok(format!(
                "{} = {};",
                self.definition(instruction, 0)?,
                java_value_to_slot(&format!("{array}[{index}]"), &element_type)
            )),
            ArrayAccess::Put => Ok(format!(
                "{array}[{index}] = {};",
                self.use_as_java_type(instruction, 2, &element_type)?
            )),
        }
    }

    fn field(
        &self,
        instruction: &Instruction,
        access: FieldAccess,
        field: &Reference,
    ) -> Result<String, RenderFailure> {
        let (raw_owner, name, descriptor) = field_symbol(field)?;
        let (name, changed) = identifier(&name.text);
        if changed {
            return Err(RenderFailure::new(
                "field name is not expressible as a Java identifier",
            ));
        }
        let owner = self.names.class_name(raw_owner);
        let static_field = if raw_owner == self.owner && self.class_initializer {
            name.clone()
        } else {
            format!("{owner}.{name}")
        };
        let field_type =
            parse_field(descriptor).map_err(|error| RenderFailure::new(error.to_string()))?;
        Ok(match access {
            FieldAccess::GetStatic => format!(
                "{} = {};",
                self.definition(instruction, 0)?,
                java_value_to_slot(&static_field, &field_type)
            ),
            FieldAccess::PutStatic => format!(
                "{static_field} = {};",
                self.use_as_java_type(instruction, 0, &field_type)?
            ),
            FieldAccess::GetInstance => format!(
                "{} = {};",
                self.definition(instruction, 0)?,
                java_value_to_slot(
                    &format!("(({owner}) {}).{name}", self.object_use(instruction, 0)?),
                    &field_type,
                )
            ),
            FieldAccess::PutInstance => format!(
                "(({owner}) {}).{name} = {};",
                self.object_use(instruction, 0)?,
                self.use_as_java_type(instruction, 1, &field_type)?
            ),
        })
    }

    fn call(
        &self,
        instruction: &Instruction,
        kind: CallKind,
        target: &Reference,
        descriptor: Option<&str>,
    ) -> Result<Vec<String>, RenderFailure> {
        if kind == CallKind::Dynamic {
            return Err(RenderFailure::new(
                "dynamic call site requires bootstrap-aware Java reconstruction",
            ));
        }
        let (owner, name, _) = method_symbol(target)?;
        let source_owner = reference_type_name(owner, self.names)?;
        let descriptor =
            descriptor.ok_or_else(|| RenderFailure::new("call has no effective descriptor"))?;
        let parsed =
            parse_method(descriptor).map_err(|error| RenderFailure::new(error.to_string()))?;
        let receiver_count = usize::from(!matches!(kind, CallKind::Static));
        let arguments = parsed
            .parameters
            .iter()
            .enumerate()
            .map(|(index, parameter)| {
                self.use_as_java_type(instruction, index + receiver_count, parameter)
            })
            .collect::<Result<Vec<_>, _>>()?
            .join(", ");
        if name.text == "<init>" {
            let receiver_type = instruction
                .use_types()
                .first()
                .ok_or_else(|| RenderFailure::new("constructor has no receiver"))?;
            if matches!(receiver_type, ValueType::UninitializedThis(_)) {
                return Ok(Vec::new());
            }
            let ValueType::Uninitialized { site, .. } = receiver_type else {
                return Err(RenderFailure::new(
                    "constructor receiver is not an uninitialized allocation",
                ));
            };
            let receiver = instruction.uses()[0];
            let mut lines = vec![format!(
                "{} = new {}({arguments});",
                self.variables.object(receiver),
                source_owner
            )];
            if let Some(aliases) = self.allocation_aliases.get(site) {
                for &alias in aliases {
                    if alias != receiver {
                        lines.push(format!(
                            "{} = {};",
                            self.variables.object(alias),
                            self.variables.object(receiver)
                        ));
                    }
                }
            }
            return Ok(lines);
        }
        let (method_name, changed) = identifier(&name.text);
        if changed {
            return Err(RenderFailure::new(
                "method name is not expressible as a Java identifier",
            ));
        }
        let invocation = match kind {
            CallKind::Static => {
                format!("{source_owner}.{method_name}({arguments})")
            }
            CallKind::Super => format!("super.{method_name}({arguments})"),
            CallKind::Virtual | CallKind::Interface | CallKind::Direct | CallKind::Polymorphic => {
                format!(
                    "(({}) {}).{}({arguments})",
                    source_owner,
                    self.object_use(instruction, 0)?,
                    method_name
                )
            }
            CallKind::Dynamic => unreachable!(),
        };
        if instruction.defs().is_empty() {
            Ok(vec![format!("{invocation};")])
        } else {
            let expression = match &parsed.return_type {
                ReturnType::Void => {
                    return Err(RenderFailure::new(
                        "void call unexpectedly defines an MLIL value",
                    ));
                }
                ReturnType::Type(value_type) => java_value_to_slot(&invocation, value_type),
            };
            Ok(vec![format!(
                "{} = {expression};",
                self.definition(instruction, 0)?,
            )])
        }
    }

    fn allocate(
        &self,
        instruction: &Instruction,
        kind: &AllocationKind,
    ) -> Result<Vec<String>, RenderFailure> {
        match kind {
            AllocationKind::Object(_) => Ok(vec![format!(
                "{} = null;",
                self.definition(instruction, 0)?
            )]),
            AllocationKind::Array {
                array_type,
                dimensions,
            } => {
                let lengths = (0..usize::from(*dimensions))
                    .map(|index| self.use_value(instruction, index))
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(vec![format!(
                    "{} = {};",
                    self.definition(instruction, 0)?,
                    new_array(array_type.descriptor(), &lengths, self.names)?
                )])
            }
            AllocationKind::InitializedArray { array_type } => {
                let element = parse_field(array_type.descriptor())
                    .map_err(|error| RenderFailure::new(error.to_string()))?;
                let JavaType::Array(element) = element else {
                    return Err(RenderFailure::new(
                        "initialized-array descriptor is not an array",
                    ));
                };
                let values = instruction
                    .uses()
                    .iter()
                    .enumerate()
                    .map(|(index, _)| self.use_as_java_type(instruction, index, &element))
                    .collect::<Result<Vec<_>, _>>()?
                    .join(", ");
                Ok(vec![format!(
                    "{} = new {}[] {{{values}}};",
                    self.definition(instruction, 0)?,
                    self.names.value_type(&element)
                )])
            }
        }
    }

    fn initialize_array(
        &self,
        instruction: &Instruction,
        descriptor: &str,
        values: &[Constant],
    ) -> Result<Vec<String>, RenderFailure> {
        let array_type = self
            .names
            .type_descriptor(descriptor)
            .map_err(|error| RenderFailure::new(error.to_string()))?;
        let JavaType::Array(element_type) =
            parse_field(descriptor).map_err(|error| RenderFailure::new(error.to_string()))?
        else {
            return Err(RenderFailure::new(
                "array initializer has a non-array descriptor",
            ));
        };
        let array = format!("(({array_type}) {})", self.object_use(instruction, 0)?);
        values
            .iter()
            .enumerate()
            .map(|(index, value)| {
                Ok(format!(
                    "{array}[{index}] = {};",
                    constant_as_java_type(value, &element_type, self.names)?
                ))
            })
            .collect()
    }

    fn definition(&self, instruction: &Instruction, index: usize) -> Result<String, RenderFailure> {
        let variable = *instruction
            .defs()
            .get(index)
            .ok_or_else(|| RenderFailure::new("missing definition"))?;
        let value_type = instruction
            .def_types()
            .get(index)
            .ok_or_else(|| RenderFailure::new("missing definition type"))?;
        Ok(self.variables.value(variable, value_type))
    }

    fn use_value(&self, instruction: &Instruction, index: usize) -> Result<String, RenderFailure> {
        let variable = *instruction
            .uses()
            .get(index)
            .ok_or_else(|| RenderFailure::new("missing operand"))?;
        let value_type = instruction
            .use_types()
            .get(index)
            .ok_or_else(|| RenderFailure::new("missing operand type"))?;
        Ok(self.variables.value(variable, value_type))
    }

    fn object_use(&self, instruction: &Instruction, index: usize) -> Result<String, RenderFailure> {
        let variable = *instruction
            .uses()
            .get(index)
            .ok_or_else(|| RenderFailure::new("missing reference operand"))?;
        Ok(self.variables.object(variable))
    }

    fn coerced_use(
        &self,
        instruction: &Instruction,
        index: usize,
        target: &ValueType,
    ) -> Result<String, RenderFailure> {
        let source = instruction
            .use_types()
            .get(index)
            .ok_or_else(|| RenderFailure::new("missing operand type"))?;
        if matches!(
            target,
            ValueType::Reference(_)
                | ValueType::Null
                | ValueType::UninitializedThis(_)
                | ValueType::Uninitialized { .. }
        ) {
            if matches!(source, ValueType::Zero) {
                return Ok("null".to_owned());
            }
            return self.object_use(instruction, index);
        }
        self.use_value(instruction, index)
    }

    fn use_as_java_type(
        &self,
        instruction: &Instruction,
        index: usize,
        target: &JavaType,
    ) -> Result<String, RenderFailure> {
        let variable = *instruction
            .uses()
            .get(index)
            .ok_or_else(|| RenderFailure::new("missing typed operand"))?;
        Ok(match target {
            JavaType::Boolean => format!("{} != 0", self.variables.int(variable)),
            JavaType::Byte => format!("(byte) {}", self.variables.int(variable)),
            JavaType::Char => format!("(char) {}", self.variables.int(variable)),
            JavaType::Short => format!("(short) {}", self.variables.int(variable)),
            JavaType::Int => self.variables.int(variable),
            JavaType::Long | JavaType::Float | JavaType::Double => {
                self.variables.name(variable, java_kind(target))
            }
            JavaType::Object(_) | JavaType::Array(_) => format!(
                "({}) {}",
                self.names.value_type(target),
                self.variables.object(variable)
            ),
        })
    }
}

pub(super) fn allocation_aliases(
    function: &Function,
) -> BTreeMap<AllocationSite, BTreeSet<VariableId>> {
    let mut aliases = BTreeMap::new();
    for block in function.cfg().blocks() {
        for instruction in block.instructions() {
            for (&variable, value_type) in instruction
                .uses()
                .iter()
                .zip(instruction.use_types())
                .chain(instruction.defs().iter().zip(instruction.def_types()))
            {
                if let ValueType::Uninitialized { site, .. } = value_type {
                    aliases
                        .entry(*site)
                        .or_insert_with(BTreeSet::new)
                        .insert(variable);
                }
            }
        }
    }
    aliases
}

pub(super) fn constant_expression(
    value: &Constant,
    names: &SourceNames,
) -> Result<String, RenderFailure> {
    Ok(match value {
        Constant::Null => "null".to_owned(),
        Constant::Integer(value) => value.to_string(),
        Constant::Long(value) => format!("{value}L"),
        Constant::Float(bits) => format!("java.lang.Float.intBitsToFloat(0x{bits:08x})"),
        Constant::Double(bits) => format!("java.lang.Double.longBitsToDouble(0x{bits:016x}L)"),
        Constant::Reference(reference) => match &reference.symbol {
            Some(ReferenceSymbol::String(value)) => string_literal(value),
            Some(ReferenceSymbol::Type(value)) => {
                format!("{}.class", reference_type_name(value, names)?)
            }
            Some(ReferenceSymbol::Integer(value)) => value.to_string(),
            Some(ReferenceSymbol::Long(value)) => format!("{value}L"),
            Some(ReferenceSymbol::Float(value)) => {
                format!("java.lang.Float.intBitsToFloat(0x{value:08x})")
            }
            Some(ReferenceSymbol::Double(value)) => {
                format!("java.lang.Double.longBitsToDouble(0x{value:016x}L)")
            }
            Some(ReferenceSymbol::MethodPrototype(descriptor)) => format!(
                "java.lang.invoke.MethodType.fromMethodDescriptorString({}, {}.class.getClassLoader())",
                rust_string_literal(descriptor),
                "java.lang.Object"
            ),
            _ => {
                return Err(RenderFailure::new(
                    "constant lacks reconstructable Java source metadata",
                ));
            }
        },
    })
}

pub(super) fn reference_type(
    reference: &Reference,
    names: &SourceNames,
) -> Result<String, RenderFailure> {
    let Some(ReferenceSymbol::Type(value)) = &reference.symbol else {
        return Err(RenderFailure::new(
            "type reference lacks a structured symbol",
        ));
    };
    reference_type_name(value, names)
}

pub(super) fn reference_type_name(
    value: &str,
    names: &SourceNames,
) -> Result<String, RenderFailure> {
    if value.starts_with('[')
        || (value.starts_with('L') && value.ends_with(';'))
        || matches!(value, "Z" | "B" | "C" | "S" | "I" | "J" | "F" | "D")
    {
        names
            .type_descriptor(value)
            .map_err(|error| RenderFailure::new(error.to_string()))
    } else {
        Ok(names.class_name(value))
    }
}

pub(super) fn java_value_to_slot(expression: &str, value_type: &JavaType) -> String {
    if matches!(value_type, JavaType::Boolean) {
        format!("({expression} ? 1 : 0)")
    } else {
        expression.to_owned()
    }
}

pub(super) fn constant_as_java_type(
    constant: &Constant,
    value_type: &JavaType,
    names: &SourceNames,
) -> Result<String, RenderFailure> {
    let expression = constant_expression(constant, names)?;
    Ok(match value_type {
        JavaType::Boolean => match constant {
            Constant::Integer(value) => (*value != 0).to_string(),
            _ => format!("({expression} != 0)"),
        },
        JavaType::Byte => format!("(byte) {expression}"),
        JavaType::Char => format!("(char) {expression}"),
        JavaType::Short => format!("(short) {expression}"),
        JavaType::Int
        | JavaType::Long
        | JavaType::Float
        | JavaType::Double
        | JavaType::Object(_)
        | JavaType::Array(_) => expression,
    })
}

pub(super) fn field_symbol(
    reference: &Reference,
) -> Result<(&str, &ExactText, &str), RenderFailure> {
    match &reference.symbol {
        Some(ReferenceSymbol::Field {
            owner,
            name,
            descriptor,
        }) => Ok((owner, name, descriptor)),
        _ => Err(RenderFailure::new(
            "field reference lacks a structured symbol",
        )),
    }
}

pub(super) fn method_symbol(
    reference: &Reference,
) -> Result<(&str, &ExactText, &str), RenderFailure> {
    match &reference.symbol {
        Some(ReferenceSymbol::Method {
            owner,
            name,
            descriptor,
        }) => Ok((owner, name, descriptor)),
        _ => Err(RenderFailure::new(
            "method reference lacks a structured symbol",
        )),
    }
}

pub(super) fn new_array(
    descriptor: &str,
    lengths: &[String],
    names: &SourceNames,
) -> Result<String, RenderFailure> {
    let mut dimensions = 0usize;
    let mut component = descriptor;
    while let Some(rest) = component.strip_prefix('[') {
        dimensions += 1;
        component = rest;
    }
    if dimensions == 0 || lengths.is_empty() || lengths.len() > dimensions {
        return Err(RenderFailure::new(
            "invalid multidimensional array allocation",
        ));
    }
    let base = names
        .type_descriptor(component)
        .map_err(|error| RenderFailure::new(error.to_string()))?;
    let mut expression = format!("new {base}");
    for length in lengths {
        write!(expression, "[{length}]").expect("writing to a String cannot fail");
    }
    for _ in lengths.len()..dimensions {
        expression.push_str("[]");
    }
    Ok(expression)
}

pub(super) const fn unary_symbol(operator: UnaryOperator) -> &'static str {
    match operator {
        UnaryOperator::Negate => "-",
        UnaryOperator::BitwiseNot => "~",
    }
}

pub(super) const fn binary_symbol(operator: BinaryOperator) -> &'static str {
    match operator {
        BinaryOperator::Add => "+",
        BinaryOperator::Subtract | BinaryOperator::ReverseSubtract => "-",
        BinaryOperator::Multiply => "*",
        BinaryOperator::Divide => "/",
        BinaryOperator::Remainder => "%",
        BinaryOperator::And => "&",
        BinaryOperator::Or => "|",
        BinaryOperator::Xor => "^",
        BinaryOperator::ShiftLeft => "<<",
        BinaryOperator::ShiftRight => ">>",
        BinaryOperator::UnsignedShiftRight => ">>>",
    }
}

pub(super) const fn relation_symbol(relation: Relation) -> &'static str {
    match relation {
        Relation::Equal => "==",
        Relation::NotEqual => "!=",
        Relation::Less => "<",
        Relation::GreaterOrEqual => ">=",
        Relation::Greater => ">",
        Relation::LessOrEqual => "<=",
    }
}

pub(super) const fn conversion_target(conversion: Conversion) -> &'static str {
    match conversion {
        Conversion::IntToLong | Conversion::FloatToLong | Conversion::DoubleToLong => "long",
        Conversion::IntToFloat | Conversion::LongToFloat | Conversion::DoubleToFloat => "float",
        Conversion::IntToDouble | Conversion::LongToDouble | Conversion::FloatToDouble => "double",
        Conversion::LongToInt | Conversion::FloatToInt | Conversion::DoubleToInt => "int",
        Conversion::IntToByte => "byte",
        Conversion::IntToChar => "char",
        Conversion::IntToShort => "short",
    }
}

const fn slot_java_type(kind: SlotKind) -> &'static str {
    match kind {
        SlotKind::Int => "int",
        SlotKind::Long => "long",
        SlotKind::Float => "float",
        SlotKind::Double => "double",
        SlotKind::Object => "java.lang.Object",
    }
}

pub(super) const fn element_array_descriptor(element: ElementType) -> &'static str {
    match element {
        ElementType::Boolean => "[Z",
        ElementType::Byte | ElementType::ByteOrBoolean => "[B",
        ElementType::Char => "[C",
        ElementType::Short => "[S",
        ElementType::Integer | ElementType::Bits32 => "[I",
        ElementType::Long | ElementType::Bits64 => "[J",
        ElementType::Float => "[F",
        ElementType::Double => "[D",
        ElementType::Reference => "[Ljava/lang/Object;",
    }
}

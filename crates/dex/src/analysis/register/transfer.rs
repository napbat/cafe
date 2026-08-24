//! Instruction transfer functions and descriptor-driven operand refinement.

use crate::file::{CallSiteIndex, FieldIndex, MethodIndex, PrototypeId, PrototypeIndex, TypeIndex};
use crate::instruction::{Instruction, InstructionData, Opcode, Operands};
use crate::{Error, Result};

use super::super::ValueKind;
use super::analyze::{MethodContext, register_width, write_value};
use super::classify::{
    is_array_get, is_array_put, is_field_get, is_field_put, is_instance_field_get,
    is_instance_field_put, is_invocation, is_move_object,
};
use super::model::{ReferenceType, RegisterFrame, RegisterType};
use super::operands::{first_register, three_registers, two_registers};

const CONSTRUCTOR_NAME: &str = "<init>";
const JAVA_LANG_CLASS_DESCRIPTOR: &str = "Ljava/lang/Class;";
const JAVA_LANG_STRING_DESCRIPTOR: &str = "Ljava/lang/String;";
const JAVA_LANG_THROWABLE_DESCRIPTOR: &str = "Ljava/lang/Throwable;";
const JAVA_LANG_METHOD_HANDLE_DESCRIPTOR: &str = "Ljava/lang/invoke/MethodHandle;";
const JAVA_LANG_METHOD_TYPE_DESCRIPTOR: &str = "Ljava/lang/invoke/MethodType;";

pub(super) fn transfer(
    context: &MethodContext<'_>,
    instruction: &Instruction,
    entry: &RegisterFrame,
) -> Result<RegisterFrame> {
    let InstructionData::Operation { opcode, .. } = instruction.data() else {
        return Err(Error::invalid_instruction(
            instruction.offset(),
            "data payload reached the register transfer function",
        ));
    };
    let facts = context
        .body
        .instruction(instruction.offset())
        .expect("body analysis covers every instruction");
    if facts.semantics.produced.is_none() && !is_move_object(*opcode) {
        for operand in &facts.semantics.reads {
            require_kind(entry, operand.register, &operand.kind, instruction.offset())?;
        }
    }
    validate_resolved_inputs(context, instruction, entry)?;

    let mut output = entry.clone();
    for operand in &facts.semantics.writes {
        write_value(
            &mut output,
            usize::from(operand.register),
            type_from_kind(&operand.kind),
        )?;
    }
    apply_special_writes(context, instruction, entry, &mut output)?;
    if context.is_constructor() && *opcode == Opcode::ReturnVoid {
        let still_uninitialized = output.registers.iter().any(|value| {
            matches!(
                value,
                RegisterType::Reference(ReferenceType::UninitializedThis { .. })
            )
        });
        if still_uninitialized {
            return Err(Error::invalid_instruction(
                instruction.offset(),
                "constructor returns before initializing its receiver",
            ));
        }
    }
    Ok(output)
}

pub(super) fn descriptor_type(descriptor: &str, offset: u32) -> Result<Option<RegisterType>> {
    let bytes = descriptor.as_bytes();
    let value = match bytes {
        [b'V'] => None,
        [b'Z' | b'B' | b'S' | b'C' | b'I'] => Some(RegisterType::Integer),
        [b'F'] => Some(RegisterType::Float),
        [b'J'] => Some(RegisterType::Long),
        [b'D'] => Some(RegisterType::Double),
        _ if valid_reference_descriptor(bytes) => Some(RegisterType::Reference(
            ReferenceType::Descriptor(descriptor.to_owned()),
        )),
        _ => {
            return Err(Error::invalid_instruction(
                offset,
                format!("invalid type descriptor `{descriptor}`"),
            ));
        }
    };
    Ok(value)
}

fn valid_reference_descriptor(bytes: &[u8]) -> bool {
    if matches!(bytes, [b'L', middle @ .., b';'] if !middle.is_empty()) {
        return true;
    }
    let component = bytes.iter().position(|byte| *byte != b'[');
    let Some(component) = component else {
        return false;
    };
    component > 0
        && match &bytes[component..] {
            [b'Z' | b'B' | b'S' | b'C' | b'I' | b'F' | b'J' | b'D'] => true,
            [b'L', middle @ .., b';'] => !middle.is_empty(),
            _ => false,
        }
}

fn validate_resolved_inputs(
    context: &MethodContext<'_>,
    instruction: &Instruction,
    frame: &RegisterFrame,
) -> Result<()> {
    let InstructionData::Operation { opcode, operands } = instruction.data() else {
        return Ok(());
    };
    if is_move_object(*opcode) {
        let (_, source) = two_registers(operands, instruction.offset())?;
        require_movable_reference(frame, source, instruction.offset())?;
    }
    if is_field_put(*opcode) {
        let field_type = field_type(context, operands, instruction.offset())?;
        let value_register = first_register(operands, instruction.offset())?;
        if matches!(field_type, RegisterType::Reference(_)) {
            require_assignable_reference(
                context,
                frame,
                value_register,
                &field_type,
                instruction.offset(),
            )?;
        } else {
            require_type(
                frame,
                value_register,
                &field_type,
                instruction.offset(),
                false,
            )?;
        }
    }
    if is_instance_field_get(*opcode) || is_instance_field_put(*opcode) {
        let (_, receiver) = two_registers(operands, instruction.offset())?;
        let owner = field_owner(context, operands, instruction.offset())?;
        let expected = RegisterType::Reference(ReferenceType::Descriptor(owner));
        require_assignable_reference(context, frame, receiver, &expected, instruction.offset())?;
    }
    if is_array_get(*opcode) || is_array_put(*opcode) {
        validate_array_access(context, *opcode, operands, frame, instruction.offset())?;
    } else if *opcode == Opcode::ArrayLength {
        let (_, array) = two_registers(operands, instruction.offset())?;
        let _ = array_component(frame, array, instruction.offset())?;
    } else if *opcode == Opcode::FillArrayData {
        let array = first_register(operands, instruction.offset())?;
        if let Some(component) = array_component(frame, array, instruction.offset())?
            && matches!(component.value, RegisterType::Reference(_))
        {
            return Err(type_error(
                instruction.offset(),
                array,
                entry_value(frame, array, instruction.offset())?,
                "an array with a primitive component",
            ));
        }
    }
    if is_invocation(*opcode) {
        let _ = validate_invocation(context, *opcode, operands, frame, instruction.offset())?;
    } else if matches!(opcode, Opcode::FilledNewArray | Opcode::FilledNewArrayRange) {
        validate_filled_array(context, operands, frame, instruction.offset())?;
    }
    if opcode.is_return() && *opcode != Opcode::ReturnVoid {
        let expected = descriptor_type(
            context
                .file
                .type_descriptor(context.prototype.return_type)?,
            instruction.offset(),
        )?
        .ok_or_else(|| {
            Error::invalid_instruction(instruction.offset(), "value returned from void method")
        })?;
        let register = first_register(operands, instruction.offset())?;
        if matches!(expected, RegisterType::Reference(_)) {
            require_assignable_reference(
                context,
                frame,
                register,
                &expected,
                instruction.offset(),
            )?;
        } else {
            require_type(frame, register, &expected, instruction.offset(), false)?;
        }
    }
    Ok(())
}

#[allow(clippy::too_many_lines)]
fn apply_special_writes(
    context: &MethodContext<'_>,
    instruction: &Instruction,
    entry: &RegisterFrame,
    output: &mut RegisterFrame,
) -> Result<()> {
    let InstructionData::Operation { opcode, operands } = instruction.data() else {
        return Ok(());
    };
    let destination = first_register(operands, instruction.offset()).ok();
    match opcode {
        Opcode::Move
        | Opcode::MoveFrom16
        | Opcode::Move16
        | Opcode::MoveWide
        | Opcode::MoveWideFrom16
        | Opcode::MoveWide16
        | Opcode::MoveObject
        | Opcode::MoveObjectFrom16
        | Opcode::MoveObject16 => {
            let (destination, source) = two_registers(operands, instruction.offset())?;
            let value = entry_value(entry, source, instruction.offset())?.clone();
            write_value(output, usize::from(destination), value)?;
        }
        Opcode::MoveResult | Opcode::MoveResultWide | Opcode::MoveResultObject => {
            let value = result_type(context, instruction)?;
            let required = context
                .body
                .instruction(instruction.offset())
                .unwrap()
                .semantics
                .writes[0]
                .kind
                .clone();
            if !kind_accepts_type(&required, &value) {
                return Err(Error::invalid_instruction(
                    instruction.offset(),
                    "move-result form does not match the producer return type",
                ));
            }
            write_value(
                output,
                usize::from(destination.expect("move-result has a destination")),
                value,
            )?;
        }
        Opcode::MoveException => {
            let value = handler_exception_type(context, instruction.offset())?;
            write_value(
                output,
                usize::from(destination.expect("move-exception has a destination")),
                value,
            )?;
        }
        Opcode::Const4
        | Opcode::Const16
        | Opcode::Const
        | Opcode::ConstHigh16
        | Opcode::ConstWide16
        | Opcode::ConstWide32
        | Opcode::ConstWide
        | Opcode::ConstWideHigh16 => {
            let Operands::RegisterLiteral { literal, .. } = operands else {
                return Err(Error::invalid_instruction(
                    instruction.offset(),
                    "constant lacks its register and literal",
                ));
            };
            if *literal == 0 {
                let value = if matches!(
                    opcode,
                    Opcode::ConstWide16
                        | Opcode::ConstWide32
                        | Opcode::ConstWide
                        | Opcode::ConstWideHigh16
                ) {
                    RegisterType::WideZero
                } else {
                    RegisterType::Zero
                };
                write_value(
                    output,
                    usize::from(destination.expect("constant has a destination")),
                    value,
                )?;
            }
        }
        Opcode::ConstString | Opcode::ConstStringJumbo => write_reference(
            output,
            destination,
            JAVA_LANG_STRING_DESCRIPTOR,
            instruction.offset(),
        )?,
        Opcode::ConstClass => write_reference(
            output,
            destination,
            JAVA_LANG_CLASS_DESCRIPTOR,
            instruction.offset(),
        )?,
        Opcode::ConstMethodHandle => write_reference(
            output,
            destination,
            JAVA_LANG_METHOD_HANDLE_DESCRIPTOR,
            instruction.offset(),
        )?,
        Opcode::ConstMethodType => write_reference(
            output,
            destination,
            JAVA_LANG_METHOD_TYPE_DESCRIPTOR,
            instruction.offset(),
        )?,
        Opcode::NewInstance => {
            let descriptor = indexed_type(context, operands, instruction.offset())?;
            if !descriptor.starts_with('L') || !descriptor.ends_with(';') {
                return Err(Error::invalid_instruction(
                    instruction.offset(),
                    "new-instance type is not an object descriptor",
                ));
            }
            write_value(
                output,
                usize::from(destination.expect("new-instance has a destination")),
                RegisterType::Reference(ReferenceType::Uninitialized {
                    descriptor,
                    allocation_offset: instruction.offset(),
                }),
            )?;
        }
        Opcode::NewArray => {
            let descriptor = indexed_type(context, operands, instruction.offset())?;
            if !descriptor.starts_with('[') {
                return Err(Error::invalid_instruction(
                    instruction.offset(),
                    "new-array type is not an array descriptor",
                ));
            }
            write_reference(output, destination, &descriptor, instruction.offset())?;
        }
        Opcode::CheckCast => {
            let descriptor = indexed_type(context, operands, instruction.offset())?;
            write_reference(output, destination, &descriptor, instruction.offset())?;
        }
        _ if is_array_get(*opcode) => {
            let (_, array, _) = three_registers(operands, instruction.offset())?;
            if let Some(component) = array_component(entry, array, instruction.offset())? {
                write_value(
                    output,
                    usize::from(destination.expect("array get has a destination")),
                    component.value,
                )?;
            }
        }
        _ if is_field_get(*opcode) => {
            let value = field_type(context, operands, instruction.offset())?;
            let required = context
                .body
                .instruction(instruction.offset())
                .unwrap()
                .semantics
                .writes[0]
                .kind
                .clone();
            if !kind_accepts_type(&required, &value) {
                return Err(Error::invalid_instruction(
                    instruction.offset(),
                    "field-get opcode does not match the referenced field type",
                ));
            }
            write_value(
                output,
                usize::from(destination.expect("field get has a destination")),
                value,
            )?;
        }
        _ => {}
    }

    if is_invocation(*opcode)
        && let Some(initialized) =
            validate_invocation(context, *opcode, operands, entry, instruction.offset())?
    {
        initialize_aliases(output, &initialized);
    }
    Ok(())
}

fn require_kind(
    frame: &RegisterFrame,
    register: u16,
    required: &ValueKind,
    offset: u32,
) -> Result<()> {
    let actual = entry_value(frame, register, offset)?;
    if !kind_accepts_type(required, actual)
        || matches!(
            actual,
            RegisterType::Reference(reference) if !reference.is_initialized()
        )
    {
        return Err(type_error(
            offset,
            register,
            actual,
            &format!("{required:?}"),
        ));
    }
    if required.register_words() == 2 {
        require_wide_continuation(frame, register, offset)?;
    }
    Ok(())
}

fn require_type(
    frame: &RegisterFrame,
    register: u16,
    required: &RegisterType,
    offset: u32,
    allow_uninitialized: bool,
) -> Result<()> {
    let actual = entry_value(frame, register, offset)?;
    let valid = type_is_compatible(actual, required, allow_uninitialized);
    if !valid {
        return Err(type_error(
            offset,
            register,
            actual,
            &format!("{required:?}"),
        ));
    }
    if register_width(required) == 2 {
        require_wide_continuation(frame, register, offset)?;
    }
    Ok(())
}

#[allow(clippy::match_same_arms)]
fn type_is_compatible(
    actual: &RegisterType,
    required: &RegisterType,
    allow_uninitialized: bool,
) -> bool {
    use RegisterType as R;
    match (actual, required) {
        (R::Zero | R::Single | R::Integer, R::Integer) => true,
        (R::Zero | R::Single | R::Float, R::Float) => true,
        (R::WideZero | R::Wide | R::Long, R::Long) => true,
        (R::WideZero | R::Wide | R::Double, R::Double) => true,
        (R::Zero, R::Reference(_)) => true,
        (R::Reference(actual), R::Reference(ReferenceType::Any)) => {
            actual.is_initialized() || allow_uninitialized
        }
        (
            R::Reference(ReferenceType::Descriptor(actual)),
            R::Reference(ReferenceType::Descriptor(required)),
        ) => actual == required,
        (R::Reference(actual), R::Reference(ReferenceType::Descriptor(_))) => {
            allow_uninitialized && !actual.is_initialized()
        }
        _ => actual == required,
    }
}

fn kind_accepts_type(kind: &ValueKind, value: &RegisterType) -> bool {
    use RegisterType as R;
    match kind {
        ValueKind::Single => matches!(
            value,
            R::Zero
                | R::Single
                | R::Integer
                | R::Float
                | R::Reference(ReferenceType::Any | ReferenceType::Descriptor(_))
        ),
        ValueKind::Wide => matches!(value, R::WideZero | R::Wide | R::Long | R::Double),
        ValueKind::Integer => matches!(value, R::Zero | R::Single | R::Integer),
        ValueKind::Float => matches!(value, R::Zero | R::Single | R::Float),
        ValueKind::Long => matches!(value, R::WideZero | R::Wide | R::Long),
        ValueKind::Double => matches!(value, R::WideZero | R::Wide | R::Double),
        ValueKind::Reference => matches!(
            value,
            R::Zero | R::Reference(ReferenceType::Any | ReferenceType::Descriptor(_))
        ),
    }
}

fn type_from_kind(kind: &ValueKind) -> RegisterType {
    match kind {
        ValueKind::Single => RegisterType::Single,
        ValueKind::Wide => RegisterType::Wide,
        ValueKind::Integer => RegisterType::Integer,
        ValueKind::Float => RegisterType::Float,
        ValueKind::Long => RegisterType::Long,
        ValueKind::Double => RegisterType::Double,
        ValueKind::Reference => RegisterType::Reference(ReferenceType::Any),
    }
}

fn validate_invocation(
    context: &MethodContext<'_>,
    opcode: Opcode,
    operands: &Operands,
    frame: &RegisterFrame,
    offset: u32,
) -> Result<Option<ReferenceType>> {
    let invocation = invocation(context, opcode, operands, offset)?;
    let registers = register_words(operands, offset)?;
    let mut cursor = 0;
    let mut initialized = None;
    if let Some(owner) = invocation.receiver {
        let register = *registers
            .first()
            .ok_or_else(|| Error::invalid_instruction(offset, "invocation receiver is missing"))?;
        let actual = entry_value(frame, register, offset)?;
        if invocation.constructor {
            let RegisterType::Reference(
                reference @ (ReferenceType::Uninitialized { .. }
                | ReferenceType::UninitializedThis { .. }),
            ) = actual
            else {
                return Err(type_error(
                    offset,
                    register,
                    actual,
                    "an uninitialized constructor receiver",
                ));
            };
            let descriptor = uninitialized_descriptor(reference);
            if !context.hierarchy.is_assignable(descriptor, &owner) {
                return Err(type_error(offset, register, actual, &owner));
            }
            initialized = Some(reference.clone());
        } else {
            let expected = RegisterType::Reference(ReferenceType::Descriptor(owner));
            require_assignable_reference(context, frame, register, &expected, offset)?;
        }
        cursor += 1;
    }
    for &parameter in &invocation.prototype.parameters {
        let expected = descriptor_type(context.file.type_descriptor(parameter)?, offset)?
            .ok_or_else(|| Error::invalid_instruction(offset, "invocation parameter is void"))?;
        let register = *registers.get(cursor).ok_or_else(|| {
            Error::invalid_instruction(offset, "invocation argument register is missing")
        })?;
        if matches!(expected, RegisterType::Reference(_)) {
            require_assignable_reference(context, frame, register, &expected, offset)?;
        } else {
            require_type(frame, register, &expected, offset, false)?;
        }
        cursor += register_width(&expected);
    }
    if cursor != registers.len() {
        return Err(Error::invalid_instruction(
            offset,
            "invocation register words do not match its prototype",
        ));
    }
    Ok(initialized)
}

struct Invocation<'a> {
    prototype: &'a PrototypeId,
    receiver: Option<String>,
    constructor: bool,
}

fn invocation<'a>(
    context: &'a MethodContext<'_>,
    opcode: Opcode,
    operands: &Operands,
    offset: u32,
) -> Result<Invocation<'a>> {
    let (primary, secondary) = indices(operands, offset)?;
    match opcode {
        Opcode::InvokeStatic | Opcode::InvokeStaticRange => {
            let method = context.file.resolve_method_id(MethodIndex::new(primary))?;
            Ok(Invocation {
                prototype: context.file.resolve_prototype(method.prototype)?,
                receiver: None,
                constructor: false,
            })
        }
        Opcode::InvokeVirtual
        | Opcode::InvokeSuper
        | Opcode::InvokeDirect
        | Opcode::InvokeInterface
        | Opcode::InvokeVirtualRange
        | Opcode::InvokeSuperRange
        | Opcode::InvokeDirectRange
        | Opcode::InvokeInterfaceRange => {
            let method_index = MethodIndex::new(primary);
            let method = context.file.resolve_method_id(method_index)?;
            let name = &context.file.resolve_string(method.name)?.text;
            Ok(Invocation {
                prototype: context.file.resolve_prototype(method.prototype)?,
                receiver: Some(context.file.type_descriptor(method.class)?.to_owned()),
                constructor: matches!(opcode, Opcode::InvokeDirect | Opcode::InvokeDirectRange)
                    && name == CONSTRUCTOR_NAME,
            })
        }
        Opcode::InvokePolymorphic | Opcode::InvokePolymorphicRange => {
            let method = context.file.resolve_method_id(MethodIndex::new(primary))?;
            let prototype = secondary.ok_or_else(|| {
                Error::invalid_instruction(offset, "polymorphic prototype is missing")
            })?;
            Ok(Invocation {
                prototype: context
                    .file
                    .resolve_prototype(PrototypeIndex::new(prototype))?,
                receiver: Some(context.file.type_descriptor(method.class)?.to_owned()),
                constructor: false,
            })
        }
        Opcode::InvokeCustom | Opcode::InvokeCustomRange => {
            let call_site = context
                .file
                .resolve_call_site(CallSiteIndex::new(primary))?;
            let components = call_site.components().ok_or_else(|| {
                Error::invalid_instruction(offset, "call site lacks a method type")
            })?;
            Ok(Invocation {
                prototype: context.file.resolve_prototype(components.method_type)?,
                receiver: None,
                constructor: false,
            })
        }
        _ => Err(Error::invalid_instruction(
            offset,
            "opcode is not an invocation",
        )),
    }
}

fn validate_filled_array(
    context: &MethodContext<'_>,
    operands: &Operands,
    frame: &RegisterFrame,
    offset: u32,
) -> Result<()> {
    let descriptor = indexed_type(context, operands, offset)?;
    let component = descriptor.strip_prefix('[').ok_or_else(|| {
        Error::invalid_instruction(offset, "filled-new-array type is not an array")
    })?;
    let expected = descriptor_type(component, offset)?
        .ok_or_else(|| Error::invalid_instruction(offset, "filled-new-array component is void"))?;
    if register_width(&expected) != 1 {
        return Err(Error::invalid_instruction(
            offset,
            "filled-new-array cannot contain wide elements",
        ));
    }
    for register in register_words(operands, offset)? {
        if matches!(expected, RegisterType::Reference(_)) {
            require_assignable_reference(context, frame, register, &expected, offset)?;
        } else {
            require_type(frame, register, &expected, offset, false)?;
        }
    }
    Ok(())
}

fn validate_array_access(
    context: &MethodContext<'_>,
    opcode: Opcode,
    operands: &Operands,
    frame: &RegisterFrame,
    offset: u32,
) -> Result<()> {
    let (value, array, _) = three_registers(operands, offset)?;
    let Some(component) = array_component(frame, array, offset)? else {
        return Ok(());
    };
    if !array_opcode_accepts(opcode, component.descriptor, &component.value) {
        return Err(type_error(
            offset,
            array,
            entry_value(frame, array, offset)?,
            &format!("an array compatible with {}", opcode.mnemonic()),
        ));
    }
    if is_array_put(opcode) {
        if matches!(component.value, RegisterType::Reference(_)) {
            require_assignable_reference(context, frame, value, &component.value, offset)?;
        } else {
            require_type(frame, value, &component.value, offset, false)?;
        }
    }
    Ok(())
}

struct ArrayComponent<'a> {
    descriptor: &'a str,
    value: RegisterType,
}

fn array_component(
    frame: &RegisterFrame,
    array: u16,
    offset: u32,
) -> Result<Option<ArrayComponent<'_>>> {
    let actual = entry_value(frame, array, offset)?;
    let descriptor = match actual {
        RegisterType::Zero | RegisterType::Reference(ReferenceType::Any) => return Ok(None),
        RegisterType::Reference(ReferenceType::Descriptor(descriptor)) => descriptor,
        _ => {
            return Err(type_error(
                offset,
                array,
                actual,
                "an initialized array reference",
            ));
        }
    };
    let component = descriptor.strip_prefix('[').ok_or_else(|| {
        type_error(
            offset,
            array,
            actual,
            "an array descriptor rather than an object reference",
        )
    })?;
    let value = descriptor_type(component, offset)?.ok_or_else(|| {
        Error::invalid_instruction(offset, "array component descriptor cannot be void")
    })?;
    Ok(Some(ArrayComponent {
        descriptor: component,
        value,
    }))
}

fn array_opcode_accepts(opcode: Opcode, descriptor: &str, component: &RegisterType) -> bool {
    match opcode {
        Opcode::Aget | Opcode::Aput => matches!(descriptor, "I" | "F"),
        Opcode::AgetWide | Opcode::AputWide => matches!(descriptor, "J" | "D"),
        Opcode::AgetObject | Opcode::AputObject => {
            matches!(component, RegisterType::Reference(_))
        }
        Opcode::AgetBoolean | Opcode::AputBoolean => descriptor == "Z",
        Opcode::AgetByte | Opcode::AputByte => descriptor == "B",
        Opcode::AgetChar | Opcode::AputChar => descriptor == "C",
        Opcode::AgetShort | Opcode::AputShort => descriptor == "S",
        _ => false,
    }
}

fn require_assignable_reference(
    context: &MethodContext<'_>,
    frame: &RegisterFrame,
    register: u16,
    expected: &RegisterType,
    offset: u32,
) -> Result<()> {
    let actual = entry_value(frame, register, offset)?;
    let RegisterType::Reference(ReferenceType::Descriptor(target)) = expected else {
        return require_type(frame, register, expected, offset, false);
    };
    let valid = match actual {
        RegisterType::Zero | RegisterType::Reference(ReferenceType::Any) => true,
        RegisterType::Reference(ReferenceType::Descriptor(source)) => {
            context.hierarchy.is_assignable(source, target)
        }
        _ => false,
    };
    if valid {
        Ok(())
    } else {
        Err(type_error(offset, register, actual, target))
    }
}

fn result_type(context: &MethodContext<'_>, instruction: &Instruction) -> Result<RegisterType> {
    let producer_offset = context
        .body
        .instruction(instruction.offset())
        .and_then(|facts| facts.result_producer)
        .ok_or_else(|| {
            Error::invalid_instruction(instruction.offset(), "move-result producer is missing")
        })?;
    let producer = context
        .code
        .instructions
        .iter()
        .find(|candidate| candidate.offset() == producer_offset)
        .expect("body analysis retains the producer");
    let InstructionData::Operation { opcode, operands } = producer.data() else {
        unreachable!("result producer is executable");
    };
    if matches!(opcode, Opcode::FilledNewArray | Opcode::FilledNewArrayRange) {
        return Ok(RegisterType::Reference(ReferenceType::Descriptor(
            indexed_type(context, operands, producer_offset)?,
        )));
    }
    let invocation = invocation(context, *opcode, operands, producer_offset)?;
    descriptor_type(
        context
            .file
            .type_descriptor(invocation.prototype.return_type)?,
        producer_offset,
    )?
    .ok_or_else(|| {
        Error::invalid_instruction(
            instruction.offset(),
            "move-result follows an invocation returning void",
        )
    })
}

fn handler_exception_type(context: &MethodContext<'_>, offset: u32) -> Result<RegisterType> {
    let types = &context
        .body
        .instruction(offset)
        .expect("handler offset is analyzed")
        .handler_types;
    let mut descriptor: Option<String> = None;
    for exception_type in types {
        let current = exception_type.map_or_else(
            || Ok(JAVA_LANG_THROWABLE_DESCRIPTOR.to_owned()),
            |index| context.file.type_descriptor(index).map(str::to_owned),
        )?;
        descriptor = Some(match descriptor {
            Some(previous) => context
                .hierarchy
                .common_supertype(&previous, &current)
                .unwrap_or_else(|| JAVA_LANG_THROWABLE_DESCRIPTOR.to_owned()),
            None => current,
        });
    }
    Ok(RegisterType::Reference(ReferenceType::Descriptor(
        descriptor.unwrap_or_else(|| JAVA_LANG_THROWABLE_DESCRIPTOR.to_owned()),
    )))
}

fn field_type(
    context: &MethodContext<'_>,
    operands: &Operands,
    offset: u32,
) -> Result<RegisterType> {
    let (index, _) = indices(operands, offset)?;
    let field = context.file.resolve_field_id(FieldIndex::new(index))?;
    descriptor_type(context.file.type_descriptor(field.field_type)?, offset)?
        .ok_or_else(|| Error::invalid_instruction(offset, "field descriptor cannot be void"))
}

fn field_owner(context: &MethodContext<'_>, operands: &Operands, offset: u32) -> Result<String> {
    let (index, _) = indices(operands, offset)?;
    let field = context.file.resolve_field_id(FieldIndex::new(index))?;
    Ok(context.file.type_descriptor(field.class)?.to_owned())
}

fn indexed_type(context: &MethodContext<'_>, operands: &Operands, offset: u32) -> Result<String> {
    let (index, _) = indices(operands, offset)?;
    Ok(context
        .file
        .type_descriptor(TypeIndex::new(index))?
        .to_owned())
}

fn indices(operands: &Operands, offset: u32) -> Result<(u32, Option<u32>)> {
    match operands {
        Operands::RegisterIndex { index, .. } | Operands::RegistersIndex { index, .. } => {
            Ok((*index, None))
        }
        Operands::RegisterListIndex {
            index,
            secondary_index,
            ..
        }
        | Operands::RegisterRangeIndex {
            index,
            secondary_index,
            ..
        } => Ok((*index, *secondary_index)),
        _ => Err(Error::invalid_instruction(
            offset,
            "indexed operand is missing",
        )),
    }
}

fn register_words(operands: &Operands, offset: u32) -> Result<Vec<u16>> {
    match operands {
        Operands::RegisterListIndex { registers, .. } => Ok(registers.clone()),
        Operands::RegisterRangeIndex { start, count, .. } => (0..u16::from(*count))
            .map(|delta| {
                start.checked_add(delta).ok_or_else(|| {
                    Error::invalid_instruction(offset, "invocation register range overflowed")
                })
            })
            .collect(),
        _ => Err(Error::invalid_instruction(
            offset,
            "register-list or range operand is missing",
        )),
    }
}

fn require_movable_reference(frame: &RegisterFrame, register: u16, offset: u32) -> Result<()> {
    let actual = entry_value(frame, register, offset)?;
    if matches!(actual, RegisterType::Zero | RegisterType::Reference(_)) {
        Ok(())
    } else {
        Err(type_error(offset, register, actual, "a reference"))
    }
}

fn require_wide_continuation(frame: &RegisterFrame, register: u16, offset: u32) -> Result<()> {
    let next = usize::from(register)
        .checked_add(1)
        .and_then(|position| frame.registers.get(position));
    if next == Some(&RegisterType::WideContinuation) {
        Ok(())
    } else {
        Err(Error::invalid_instruction(
            offset,
            format!("wide register v{register} lacks its adjacent high word"),
        ))
    }
}

fn entry_value(frame: &RegisterFrame, register: u16, offset: u32) -> Result<&RegisterType> {
    let value = frame.register(register).ok_or_else(|| {
        Error::invalid_instruction(offset, format!("register v{register} is outside the frame"))
    })?;
    if matches!(
        value,
        RegisterType::Unknown | RegisterType::Conflict | RegisterType::WideContinuation
    ) {
        Err(type_error(offset, register, value, "an initialized value"))
    } else {
        Ok(value)
    }
}

fn initialize_aliases(frame: &mut RegisterFrame, initialized: &ReferenceType) {
    let descriptor = uninitialized_descriptor(initialized).to_owned();
    for value in &mut frame.registers {
        if matches!(value, RegisterType::Reference(reference) if reference == initialized) {
            *value = RegisterType::Reference(ReferenceType::Descriptor(descriptor.clone()));
        }
    }
}

fn uninitialized_descriptor(reference: &ReferenceType) -> &str {
    match reference {
        ReferenceType::Uninitialized { descriptor, .. }
        | ReferenceType::UninitializedThis { descriptor } => descriptor,
        ReferenceType::Any | ReferenceType::Descriptor(_) => {
            unreachable!("constructor initialization requires an uninitialized reference")
        }
    }
}

fn write_reference(
    frame: &mut RegisterFrame,
    destination: Option<u16>,
    descriptor: &str,
    offset: u32,
) -> Result<()> {
    write_value(
        frame,
        usize::from(destination.ok_or_else(|| {
            Error::invalid_instruction(offset, "reference-producing destination is missing")
        })?),
        RegisterType::Reference(ReferenceType::Descriptor(descriptor.to_owned())),
    )
}

fn type_error(offset: u32, register: u16, actual: &RegisterType, expected: &str) -> Error {
    Error::invalid_instruction(
        offset,
        format!("register v{register} is {actual:?}, expected {expected}"),
    )
}

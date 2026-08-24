//! JVM instruction verification and frame transfer functions.

use crate::bytecode::{ArrayType, Instruction, Opcode, Operand};
use crate::classfile::{Constant, ConstantPool};
use crate::descriptor::{self, JavaType, ReturnType};
use crate::{Error, Result};

use super::frame::{MethodContext, frame_value, local_index, write_local};
use super::model::{FrameState, FrameValue};
use super::stack_ops::{apply_stack_manipulation, take_top};

const JAVA_LANG_CLASS_NAME: &str = "java/lang/Class";
const JAVA_LANG_INVOKE_METHOD_HANDLE_NAME: &str = "java/lang/invoke/MethodHandle";
const JAVA_LANG_INVOKE_METHOD_TYPE_NAME: &str = "java/lang/invoke/MethodType";
const JAVA_LANG_OBJECT_NAME: &str = "java/lang/Object";
const JAVA_LANG_STRING_NAME: &str = "java/lang/String";
const CONSTRUCTOR_NAME: &str = "<init>";

#[allow(clippy::match_same_arms, clippy::too_many_lines)]
pub(super) fn transfer(
    context: &MethodContext<'_>,
    instruction: &Instruction,
    entry: &FrameState,
) -> Result<FrameState> {
    use Opcode as O;
    let mut frame = entry.clone();
    let offset = instruction.offset;
    match instruction.opcode {
        O::Nop => {}
        O::AConstNull => frame.stack.push(FrameValue::Null),
        O::IConstM1
        | O::IConst0
        | O::IConst1
        | O::IConst2
        | O::IConst3
        | O::IConst4
        | O::IConst5
        | O::BiPush
        | O::SiPush => frame.stack.push(FrameValue::Integer),
        O::LConst0 | O::LConst1 => frame.stack.push(FrameValue::Long),
        O::FConst0 | O::FConst1 | O::FConst2 => frame.stack.push(FrameValue::Float),
        O::DConst0 | O::DConst1 => frame.stack.push(FrameValue::Double),
        O::Ldc | O::LdcW | O::Ldc2W => {
            frame.stack.push(load_constant(
                context.pool,
                constant_index(instruction)?,
                instruction.opcode,
                offset,
            )?);
        }

        O::ILoad | O::ILoad0 | O::ILoad1 | O::ILoad2 | O::ILoad3 => {
            load_local(&mut frame, instruction, FrameValue::Integer)?;
        }
        O::LLoad | O::LLoad0 | O::LLoad1 | O::LLoad2 | O::LLoad3 => {
            load_local(&mut frame, instruction, FrameValue::Long)?;
        }
        O::FLoad | O::FLoad0 | O::FLoad1 | O::FLoad2 | O::FLoad3 => {
            load_local(&mut frame, instruction, FrameValue::Float)?;
        }
        O::DLoad | O::DLoad0 | O::DLoad1 | O::DLoad2 | O::DLoad3 => {
            load_local(&mut frame, instruction, FrameValue::Double)?;
        }
        O::ALoad | O::ALoad0 | O::ALoad1 | O::ALoad2 | O::ALoad3 => {
            load_reference_local(&mut frame, instruction)?;
        }

        O::IStore | O::IStore0 | O::IStore1 | O::IStore2 | O::IStore3 => {
            store_local(&mut frame, instruction, FrameValue::Integer)?;
        }
        O::LStore | O::LStore0 | O::LStore1 | O::LStore2 | O::LStore3 => {
            store_local(&mut frame, instruction, FrameValue::Long)?;
        }
        O::FStore | O::FStore0 | O::FStore1 | O::FStore2 | O::FStore3 => {
            store_local(&mut frame, instruction, FrameValue::Float)?;
        }
        O::DStore | O::DStore0 | O::DStore1 | O::DStore2 | O::DStore3 => {
            store_local(&mut frame, instruction, FrameValue::Double)?;
        }
        O::AStore | O::AStore0 | O::AStore1 | O::AStore2 | O::AStore3 => {
            store_reference_local(&mut frame, instruction)?;
        }

        O::IALoad => array_load(&mut frame, &["I"], FrameValue::Integer, offset)?,
        O::LALoad => array_load(&mut frame, &["J"], FrameValue::Long, offset)?,
        O::FALoad => array_load(&mut frame, &["F"], FrameValue::Float, offset)?,
        O::DALoad => array_load(&mut frame, &["D"], FrameValue::Double, offset)?,
        O::AALoad => reference_array_load(&mut frame, offset)?,
        O::BALoad => array_load(&mut frame, &["B", "Z"], FrameValue::Integer, offset)?,
        O::CALoad => array_load(&mut frame, &["C"], FrameValue::Integer, offset)?,
        O::SALoad => array_load(&mut frame, &["S"], FrameValue::Integer, offset)?,
        O::IAStore => array_store(&mut frame, &["I"], &FrameValue::Integer, offset)?,
        O::LAStore => array_store(&mut frame, &["J"], &FrameValue::Long, offset)?,
        O::FAStore => array_store(&mut frame, &["F"], &FrameValue::Float, offset)?,
        O::DAStore => array_store(&mut frame, &["D"], &FrameValue::Double, offset)?,
        O::AAStore => reference_array_store(context, &mut frame, offset)?,
        O::BAStore => array_store(&mut frame, &["B", "Z"], &FrameValue::Integer, offset)?,
        O::CAStore => array_store(&mut frame, &["C"], &FrameValue::Integer, offset)?,
        O::SAStore => array_store(&mut frame, &["S"], &FrameValue::Integer, offset)?,

        O::Pop
        | O::Pop2
        | O::Dup
        | O::DupX1
        | O::DupX2
        | O::Dup2
        | O::Dup2X1
        | O::Dup2X2
        | O::Swap => apply_stack_manipulation(instruction.opcode, &mut frame, offset)?,

        O::IAdd | O::ISub | O::IMul | O::IDiv | O::IRem | O::IAnd | O::IOr | O::IXor => {
            binary(&mut frame, FrameValue::Integer, offset)?;
        }
        O::LAdd | O::LSub | O::LMul | O::LDiv | O::LRem | O::LAnd | O::LOr | O::LXor => {
            binary(&mut frame, FrameValue::Long, offset)?;
        }
        O::FAdd | O::FSub | O::FMul | O::FDiv | O::FRem => {
            binary(&mut frame, FrameValue::Float, offset)?;
        }
        O::DAdd | O::DSub | O::DMul | O::DDiv | O::DRem => {
            binary(&mut frame, FrameValue::Double, offset)?;
        }
        O::IShl | O::IShr | O::IUShr => shift(&mut frame, FrameValue::Integer, offset)?,
        O::LShl | O::LShr | O::LUShr => shift(&mut frame, FrameValue::Long, offset)?,
        O::INeg => unary(
            &mut frame,
            &FrameValue::Integer,
            FrameValue::Integer,
            offset,
        )?,
        O::LNeg => unary(&mut frame, &FrameValue::Long, FrameValue::Long, offset)?,
        O::FNeg => unary(&mut frame, &FrameValue::Float, FrameValue::Float, offset)?,
        O::DNeg => unary(&mut frame, &FrameValue::Double, FrameValue::Double, offset)?,

        O::IInc => {
            let index = local_index(instruction)?;
            require_local(&frame, index, &FrameValue::Integer, offset)?;
        }
        O::I2L => unary(&mut frame, &FrameValue::Integer, FrameValue::Long, offset)?,
        O::I2F => unary(&mut frame, &FrameValue::Integer, FrameValue::Float, offset)?,
        O::I2D => unary(&mut frame, &FrameValue::Integer, FrameValue::Double, offset)?,
        O::L2I => unary(&mut frame, &FrameValue::Long, FrameValue::Integer, offset)?,
        O::L2F => unary(&mut frame, &FrameValue::Long, FrameValue::Float, offset)?,
        O::L2D => unary(&mut frame, &FrameValue::Long, FrameValue::Double, offset)?,
        O::F2I => unary(&mut frame, &FrameValue::Float, FrameValue::Integer, offset)?,
        O::F2L => unary(&mut frame, &FrameValue::Float, FrameValue::Long, offset)?,
        O::F2D => unary(&mut frame, &FrameValue::Float, FrameValue::Double, offset)?,
        O::D2I => unary(&mut frame, &FrameValue::Double, FrameValue::Integer, offset)?,
        O::D2L => unary(&mut frame, &FrameValue::Double, FrameValue::Long, offset)?,
        O::D2F => unary(&mut frame, &FrameValue::Double, FrameValue::Float, offset)?,
        O::I2B | O::I2C | O::I2S => {
            unary(
                &mut frame,
                &FrameValue::Integer,
                FrameValue::Integer,
                offset,
            )?;
        }
        O::LCmp => compare(&mut frame, &FrameValue::Long, offset)?,
        O::FCmpL | O::FCmpG => compare(&mut frame, &FrameValue::Float, offset)?,
        O::DCmpL | O::DCmpG => compare(&mut frame, &FrameValue::Double, offset)?,

        O::IfEq | O::IfNe | O::IfLt | O::IfGe | O::IfGt | O::IfLe => {
            pop_expected(&mut frame, &FrameValue::Integer, offset)?;
        }
        O::IfICmpEq | O::IfICmpNe | O::IfICmpLt | O::IfICmpGe | O::IfICmpGt | O::IfICmpLe => {
            pop_expected(&mut frame, &FrameValue::Integer, offset)?;
            pop_expected(&mut frame, &FrameValue::Integer, offset)?;
        }
        O::IfACmpEq | O::IfACmpNe => {
            pop_reference(&mut frame, false, offset)?;
            pop_reference(&mut frame, false, offset)?;
        }
        O::IfNull | O::IfNonNull => {
            pop_reference(&mut frame, false, offset)?;
        }
        O::Goto | O::GotoW => {}
        O::TableSwitch | O::LookupSwitch => {
            pop_expected(&mut frame, &FrameValue::Integer, offset)?;
        }

        O::IReturn => method_return(context, &mut frame, &FrameValue::Integer, offset)?,
        O::LReturn => method_return(context, &mut frame, &FrameValue::Long, offset)?,
        O::FReturn => method_return(context, &mut frame, &FrameValue::Float, offset)?,
        O::DReturn => method_return(context, &mut frame, &FrameValue::Double, offset)?,
        O::AReturn => reference_return(context, &mut frame, offset)?,
        O::Return => void_return(context, &frame, offset)?,

        O::GetStatic | O::PutStatic | O::GetField | O::PutField => {
            field_access(context, instruction, &mut frame)?;
        }
        O::InvokeVirtual
        | O::InvokeSpecial
        | O::InvokeStatic
        | O::InvokeInterface
        | O::InvokeDynamic => invocation(context, instruction, &mut frame)?,
        O::New => new_object(context, instruction, &mut frame)?,
        O::NewArray => new_primitive_array(instruction, &mut frame)?,
        O::ANewArray => new_reference_array(context, instruction, &mut frame)?,
        O::ArrayLength => {
            let _ = pop_array(&mut frame, offset)?;
            frame.stack.push(FrameValue::Integer);
        }
        O::AThrow => {
            pop_reference(&mut frame, false, offset)?;
            frame.stack.clear();
        }
        O::CheckCast => check_cast(context, instruction, &mut frame)?,
        O::InstanceOf => {
            pop_reference(&mut frame, false, offset)?;
            frame.stack.push(FrameValue::Integer);
        }
        O::MonitorEnter | O::MonitorExit => {
            pop_reference(&mut frame, false, offset)?;
        }
        O::MultiANewArray => multi_array(context, instruction, &mut frame)?,

        O::Jsr | O::JsrW | O::Ret => {
            return Err(Error::invalid_bytecode(
                offset,
                "legacy jsr/ret subroutine reached frame transfer",
            ));
        }
        O::Wide | O::Breakpoint | O::ImpDep1 | O::ImpDep2 => {
            return Err(Error::invalid_bytecode(
                offset,
                format!(
                    "{} is not executable standard method code",
                    instruction.mnemonic()
                ),
            ));
        }
    }
    Ok(frame)
}

fn load_constant(
    pool: &ConstantPool,
    index: u16,
    opcode: Opcode,
    offset: usize,
) -> Result<FrameValue> {
    let value = match pool.get(index)? {
        Constant::Integer(_) => Ok(FrameValue::Integer),
        Constant::Float(_) => Ok(FrameValue::Float),
        Constant::Long(_) => Ok(FrameValue::Long),
        Constant::Double(_) => Ok(FrameValue::Double),
        Constant::String { .. } => Ok(FrameValue::Reference(JAVA_LANG_STRING_NAME.to_owned())),
        Constant::Class { .. } => Ok(FrameValue::Reference(JAVA_LANG_CLASS_NAME.to_owned())),
        Constant::MethodHandle { .. } => Ok(FrameValue::Reference(
            JAVA_LANG_INVOKE_METHOD_HANDLE_NAME.to_owned(),
        )),
        Constant::MethodType { .. } => Ok(FrameValue::Reference(
            JAVA_LANG_INVOKE_METHOD_TYPE_NAME.to_owned(),
        )),
        Constant::Dynamic {
            name_and_type_index,
            ..
        } => {
            let (_, descriptor) = pool.name_and_type(*name_and_type_index)?;
            Ok(frame_value(&descriptor::parse_field(descriptor)?))
        }
        constant => Err(Error::invalid_bytecode(
            offset,
            format!("ldc cannot load {} constant #{index}", constant.tag_name()),
        )),
    }?;
    if (opcode == Opcode::Ldc2W) != value.is_category_two() {
        return Err(Error::invalid_bytecode(
            offset,
            format!("{} is incompatible with {value:?}", opcode.mnemonic()),
        ));
    }
    Ok(value)
}

fn load_local(
    frame: &mut FrameState,
    instruction: &Instruction,
    expected: FrameValue,
) -> Result<()> {
    let index = local_index(instruction)?;
    require_local(frame, index, &expected, instruction.offset)?;
    frame.stack.push(expected);
    Ok(())
}

fn load_reference_local(frame: &mut FrameState, instruction: &Instruction) -> Result<()> {
    let index = local_index(instruction)?;
    let value = frame.locals.get(usize::from(index)).ok_or_else(|| {
        Error::invalid_bytecode(instruction.offset, "local index is out of range")
    })?;
    if !value.is_reference() {
        return Err(local_type_error(
            instruction.offset,
            index,
            value,
            "a reference",
        ));
    }
    frame.stack.push(value.clone());
    Ok(())
}

fn store_local(
    frame: &mut FrameState,
    instruction: &Instruction,
    expected: FrameValue,
) -> Result<()> {
    pop_expected(frame, &expected, instruction.offset)?;
    write_local(
        frame,
        usize::from(local_index(instruction)?),
        expected,
        instruction.offset,
    )
}

fn store_reference_local(frame: &mut FrameState, instruction: &Instruction) -> Result<()> {
    let value = pop_reference(frame, true, instruction.offset)?;
    write_local(
        frame,
        usize::from(local_index(instruction)?),
        value,
        instruction.offset,
    )
}

fn require_local(
    frame: &FrameState,
    index: u16,
    expected: &FrameValue,
    offset: usize,
) -> Result<()> {
    let actual = frame
        .locals
        .get(usize::from(index))
        .ok_or_else(|| Error::invalid_bytecode(offset, "local index is out of range"))?;
    if actual != expected {
        return Err(local_type_error(
            offset,
            index,
            actual,
            &format!("{expected:?}"),
        ));
    }
    if expected.is_category_two()
        && frame.locals.get(usize::from(index) + 1) != Some(&FrameValue::WideContinuation)
    {
        return Err(Error::invalid_bytecode(
            offset,
            format!("wide local {index} lacks its continuation slot"),
        ));
    }
    Ok(())
}

fn array_load(
    frame: &mut FrameState,
    components: &[&str],
    value: FrameValue,
    offset: usize,
) -> Result<()> {
    pop_expected(frame, &FrameValue::Integer, offset)?;
    if let Some(component) = pop_array(frame, offset)?
        && !components.contains(&component.as_str())
    {
        return Err(Error::invalid_bytecode(
            offset,
            format!("array component `{component}` is incompatible with the load opcode"),
        ));
    }
    frame.stack.push(value);
    Ok(())
}

fn reference_array_load(frame: &mut FrameState, offset: usize) -> Result<()> {
    pop_expected(frame, &FrameValue::Integer, offset)?;
    let component = match pop_array(frame, offset)? {
        Some(component) => reference_component(&component).ok_or_else(|| {
            Error::invalid_bytecode(offset, "aaload requires an array of reference values")
        })?,
        None => JAVA_LANG_OBJECT_NAME.to_owned(),
    };
    frame.stack.push(FrameValue::Reference(component));
    Ok(())
}

fn array_store(
    frame: &mut FrameState,
    components: &[&str],
    value: &FrameValue,
    offset: usize,
) -> Result<()> {
    pop_expected(frame, value, offset)?;
    pop_expected(frame, &FrameValue::Integer, offset)?;
    if let Some(component) = pop_array(frame, offset)?
        && !components.contains(&component.as_str())
    {
        return Err(Error::invalid_bytecode(
            offset,
            format!("array component `{component}` is incompatible with the store opcode"),
        ));
    }
    Ok(())
}

fn reference_array_store(
    context: &MethodContext<'_>,
    frame: &mut FrameState,
    offset: usize,
) -> Result<()> {
    let value = pop_reference(frame, false, offset)?;
    pop_expected(frame, &FrameValue::Integer, offset)?;
    if let Some(component) = pop_array(frame, offset)? {
        let target = reference_component(&component).ok_or_else(|| {
            Error::invalid_bytecode(offset, "aastore requires an array of reference values")
        })?;
        require_reference_assignable(context, &value, &target, offset)?;
    }
    Ok(())
}

fn pop_array(frame: &mut FrameState, offset: usize) -> Result<Option<String>> {
    match pop_reference(frame, false, offset)? {
        FrameValue::Null => Ok(None),
        FrameValue::Reference(descriptor) => descriptor
            .strip_prefix('[')
            .map(str::to_owned)
            .map(Some)
            .ok_or_else(|| Error::invalid_bytecode(offset, "operand is not an array reference")),
        _ => unreachable!("initialized reference pop returned an uninitialized value"),
    }
}

fn binary(frame: &mut FrameState, value: FrameValue, offset: usize) -> Result<()> {
    pop_expected(frame, &value, offset)?;
    pop_expected(frame, &value, offset)?;
    frame.stack.push(value);
    Ok(())
}

fn shift(frame: &mut FrameState, value: FrameValue, offset: usize) -> Result<()> {
    pop_expected(frame, &FrameValue::Integer, offset)?;
    pop_expected(frame, &value, offset)?;
    frame.stack.push(value);
    Ok(())
}

fn unary(
    frame: &mut FrameState,
    input: &FrameValue,
    output: FrameValue,
    offset: usize,
) -> Result<()> {
    pop_expected(frame, input, offset)?;
    frame.stack.push(output);
    Ok(())
}

fn compare(frame: &mut FrameState, value: &FrameValue, offset: usize) -> Result<()> {
    pop_expected(frame, value, offset)?;
    pop_expected(frame, value, offset)?;
    frame.stack.push(FrameValue::Integer);
    Ok(())
}

fn method_return(
    context: &MethodContext<'_>,
    frame: &mut FrameState,
    expected: &FrameValue,
    offset: usize,
) -> Result<()> {
    let declared = match &context.descriptor.return_type {
        ReturnType::Type(value) => frame_value(value),
        ReturnType::Void => {
            return Err(Error::invalid_bytecode(
                offset,
                "value returned from void method",
            ));
        }
    };
    if declared != *expected {
        return Err(Error::invalid_bytecode(
            offset,
            format!("return opcode expects {expected:?}, method declares {declared:?}"),
        ));
    }
    pop_expected(frame, expected, offset)?;
    require_empty_stack(frame, offset)
}

fn reference_return(
    context: &MethodContext<'_>,
    frame: &mut FrameState,
    offset: usize,
) -> Result<()> {
    let ReturnType::Type(value @ (JavaType::Object(_) | JavaType::Array(_))) =
        &context.descriptor.return_type
    else {
        return Err(Error::invalid_bytecode(
            offset,
            "areturn does not match the method return descriptor",
        ));
    };
    let FrameValue::Reference(target) = frame_value(value) else {
        unreachable!("object and array descriptors produce reference frames");
    };
    pop_assignable_reference(context, frame, &target, offset)?;
    require_empty_stack(frame, offset)
}

fn void_return(context: &MethodContext<'_>, frame: &FrameState, offset: usize) -> Result<()> {
    if context.descriptor.return_type != ReturnType::Void {
        return Err(Error::invalid_bytecode(
            offset,
            "return does not match a value-returning method descriptor",
        ));
    }
    if context.name == CONSTRUCTOR_NAME
        && frame
            .locals
            .iter()
            .chain(&frame.stack)
            .any(|value| *value == FrameValue::UninitializedThis)
    {
        return Err(Error::invalid_bytecode(
            offset,
            "constructor returns before initializing its receiver",
        ));
    }
    require_empty_stack(frame, offset)
}

fn field_access(
    context: &MethodContext<'_>,
    instruction: &Instruction,
    frame: &mut FrameState,
) -> Result<()> {
    let index = constant_index(instruction)?;
    let (owner, _, descriptor) = field_reference(context.pool, index, instruction.offset)?;
    let value = frame_value(&descriptor::parse_field(descriptor)?);
    match instruction.opcode {
        Opcode::GetStatic => frame.stack.push(value),
        Opcode::PutStatic => pop_assignable_value(context, frame, &value, instruction.offset)?,
        Opcode::GetField => {
            pop_initialized_receiver(context, frame, owner, instruction.offset)?;
            frame.stack.push(value);
        }
        Opcode::PutField => {
            pop_assignable_value(context, frame, &value, instruction.offset)?;
            pop_initialized_receiver(context, frame, owner, instruction.offset)?;
        }
        _ => unreachable!("caller filters field opcodes"),
    }
    Ok(())
}

fn invocation(
    context: &MethodContext<'_>,
    instruction: &Instruction,
    frame: &mut FrameState,
) -> Result<()> {
    let (owner, name, descriptor, receiver) = method_reference(context, instruction)?;
    let parsed = descriptor::parse_method(descriptor)?;
    if let Operand::InvokeInterface { count, .. } = instruction.operand {
        let expected = parsed
            .parameters
            .iter()
            .try_fold(1_usize, |slots, parameter| {
                slots
                    .checked_add(parameter.slot_width().slot_count())
                    .ok_or_else(|| {
                        Error::invalid_bytecode(instruction.offset, "invokeinterface slot overflow")
                    })
            })?;
        if usize::from(count) != expected {
            return Err(Error::invalid_bytecode(
                instruction.offset,
                format!("invokeinterface count {count} does not match {expected} argument slots"),
            ));
        }
    }
    for parameter in parsed.parameters.iter().rev() {
        let expected = frame_value(parameter);
        pop_assignable_value(context, frame, &expected, instruction.offset)?;
    }
    if receiver {
        if instruction.opcode == Opcode::InvokeSpecial && name == CONSTRUCTOR_NAME {
            let value = pop_reference(frame, true, instruction.offset)?;
            initialize_constructor_receiver(context, frame, &value, owner, instruction.offset)?;
        } else {
            pop_initialized_receiver(context, frame, owner, instruction.offset)?;
        }
    }
    if let ReturnType::Type(value) = parsed.return_type {
        frame.stack.push(frame_value(&value));
    }
    Ok(())
}

fn new_object(
    context: &MethodContext<'_>,
    instruction: &Instruction,
    frame: &mut FrameState,
) -> Result<()> {
    let class = context
        .pool
        .class_name(constant_index(instruction)?)?
        .to_owned();
    if class.starts_with('[') {
        return Err(Error::invalid_bytecode(
            instruction.offset,
            "new instruction cannot allocate an array class",
        ));
    }
    let offset = u16::try_from(instruction.offset).map_err(|_| {
        Error::invalid_bytecode(instruction.offset, "new instruction offset exceeds u16")
    })?;
    frame
        .stack
        .push(FrameValue::Uninitialized { class, offset });
    Ok(())
}

fn new_primitive_array(instruction: &Instruction, frame: &mut FrameState) -> Result<()> {
    pop_expected(frame, &FrameValue::Integer, instruction.offset)?;
    let Operand::ArrayType(array_type) = instruction.operand else {
        return Err(Error::invalid_bytecode(
            instruction.offset,
            "newarray lacks its primitive type",
        ));
    };
    frame.stack.push(FrameValue::Reference(
        primitive_array_descriptor(array_type).to_owned(),
    ));
    Ok(())
}

fn new_reference_array(
    context: &MethodContext<'_>,
    instruction: &Instruction,
    frame: &mut FrameState,
) -> Result<()> {
    pop_expected(frame, &FrameValue::Integer, instruction.offset)?;
    let component = context.pool.class_name(constant_index(instruction)?)?;
    let descriptor = if component.starts_with('[') {
        format!("[{component}")
    } else {
        format!("[L{component};")
    };
    frame.stack.push(FrameValue::Reference(descriptor));
    Ok(())
}

fn check_cast(
    context: &MethodContext<'_>,
    instruction: &Instruction,
    frame: &mut FrameState,
) -> Result<()> {
    let value = pop_reference(frame, false, instruction.offset)?;
    if value == FrameValue::Null {
        frame.stack.push(FrameValue::Null);
    } else {
        frame.stack.push(FrameValue::Reference(
            context
                .pool
                .class_name(constant_index(instruction)?)?
                .to_owned(),
        ));
    }
    Ok(())
}

fn multi_array(
    context: &MethodContext<'_>,
    instruction: &Instruction,
    frame: &mut FrameState,
) -> Result<()> {
    let Operand::MultiArray { index, dimensions } = instruction.operand else {
        return Err(Error::invalid_bytecode(
            instruction.offset,
            "multianewarray lacks its operands",
        ));
    };
    for _ in 0..dimensions {
        pop_expected(frame, &FrameValue::Integer, instruction.offset)?;
    }
    let descriptor = context.pool.class_name(index)?;
    let available = descriptor.bytes().take_while(|byte| *byte == b'[').count();
    if available == 0 || usize::from(dimensions) > available {
        return Err(Error::invalid_bytecode(
            instruction.offset,
            format!("multianewarray dimensions {dimensions} exceed `{descriptor}`"),
        ));
    }
    frame
        .stack
        .push(FrameValue::Reference(descriptor.to_owned()));
    Ok(())
}

fn method_reference<'a>(
    context: &'a MethodContext<'_>,
    instruction: &Instruction,
) -> Result<(&'a str, &'a str, &'a str, bool)> {
    if instruction.opcode == Opcode::InvokeDynamic {
        let Operand::InvokeDynamic(index) = instruction.operand else {
            return Err(Error::invalid_bytecode(
                instruction.offset,
                "invokedynamic lacks its constant index",
            ));
        };
        let Constant::InvokeDynamic {
            name_and_type_index,
            ..
        } = context.pool.get(index)?
        else {
            return Err(Error::invalid_bytecode(
                instruction.offset,
                "invokedynamic constant has the wrong tag",
            ));
        };
        let (name, descriptor) = context.pool.name_and_type(*name_and_type_index)?;
        return Ok(("", name, descriptor, false));
    }
    let (Operand::Constant(index) | Operand::InvokeInterface { index, .. }) = instruction.operand
    else {
        return Err(Error::invalid_bytecode(
            instruction.offset,
            "invocation constant index is missing",
        ));
    };
    let (class_index, name_and_type_index) = match context.pool.get(index)? {
        Constant::MethodRef {
            class_index,
            name_and_type_index,
        }
        | Constant::InterfaceMethodRef {
            class_index,
            name_and_type_index,
        } => (*class_index, *name_and_type_index),
        constant => {
            return Err(Error::invalid_bytecode(
                instruction.offset,
                format!("invocation references {} constant", constant.tag_name()),
            ));
        }
    };
    let owner = context.pool.class_name(class_index)?;
    let (name, descriptor) = context.pool.name_and_type(name_and_type_index)?;
    Ok((
        owner,
        name,
        descriptor,
        instruction.opcode != Opcode::InvokeStatic,
    ))
}

fn field_reference(pool: &ConstantPool, index: u16, offset: usize) -> Result<(&str, &str, &str)> {
    let Constant::FieldRef {
        class_index,
        name_and_type_index,
    } = pool.get(index)?
    else {
        return Err(Error::invalid_bytecode(
            offset,
            "field instruction does not reference a Fieldref constant",
        ));
    };
    let owner = pool.class_name(*class_index)?;
    let (name, descriptor) = pool.name_and_type(*name_and_type_index)?;
    Ok((owner, name, descriptor))
}

fn initialize_constructor_receiver(
    context: &MethodContext<'_>,
    frame: &mut FrameState,
    receiver: &FrameValue,
    owner: &str,
    offset: usize,
) -> Result<()> {
    let initialized = match receiver {
        FrameValue::Uninitialized { class, .. } if class == owner => {
            FrameValue::Reference(class.clone())
        }
        FrameValue::UninitializedThis if context.hierarchy.is_assignable(context.owner, owner) => {
            FrameValue::Reference(context.owner.to_owned())
        }
        _ => {
            return Err(Error::invalid_bytecode(
                offset,
                format!("constructor receiver {receiver:?} is not uninitialized `{owner}`"),
            ));
        }
    };
    for value in frame.locals.iter_mut().chain(&mut frame.stack) {
        if value == receiver {
            *value = initialized.clone();
        }
    }
    Ok(())
}

fn pop_initialized_receiver(
    context: &MethodContext<'_>,
    frame: &mut FrameState,
    owner: &str,
    offset: usize,
) -> Result<()> {
    let value = pop_reference(frame, false, offset)?;
    require_reference_assignable(context, &value, owner, offset)
}

fn pop_assignable_value(
    context: &MethodContext<'_>,
    frame: &mut FrameState,
    expected: &FrameValue,
    offset: usize,
) -> Result<()> {
    if let FrameValue::Reference(target) = expected {
        pop_assignable_reference(context, frame, target, offset)
    } else {
        pop_expected(frame, expected, offset)
    }
}

fn pop_assignable_reference(
    context: &MethodContext<'_>,
    frame: &mut FrameState,
    target: &str,
    offset: usize,
) -> Result<()> {
    let value = pop_reference(frame, false, offset)?;
    require_reference_assignable(context, &value, target, offset)
}

fn require_reference_assignable(
    context: &MethodContext<'_>,
    value: &FrameValue,
    target: &str,
    offset: usize,
) -> Result<()> {
    let valid = match value {
        FrameValue::Null => true,
        FrameValue::Reference(source) => context.hierarchy.is_assignable(source, target),
        FrameValue::Top
        | FrameValue::Integer
        | FrameValue::Float
        | FrameValue::Long
        | FrameValue::Double
        | FrameValue::UninitializedThis
        | FrameValue::Uninitialized { .. }
        | FrameValue::WideContinuation => false,
    };
    if valid {
        Ok(())
    } else {
        Err(Error::invalid_bytecode(
            offset,
            format!("reference {value:?} is not assignable to `{target}`"),
        ))
    }
}

fn pop_expected(frame: &mut FrameState, expected: &FrameValue, offset: usize) -> Result<()> {
    let actual = take_top(frame, offset)?;
    if actual == *expected {
        Ok(())
    } else {
        Err(Error::invalid_bytecode(
            offset,
            format!("operand stack has {actual:?}, expected {expected:?}"),
        ))
    }
}

fn pop_reference(
    frame: &mut FrameState,
    allow_uninitialized: bool,
    offset: usize,
) -> Result<FrameValue> {
    let value = take_top(frame, offset)?;
    let valid = matches!(value, FrameValue::Null | FrameValue::Reference(_))
        || (allow_uninitialized
            && matches!(
                value,
                FrameValue::UninitializedThis | FrameValue::Uninitialized { .. }
            ));
    if valid {
        Ok(value)
    } else {
        Err(Error::invalid_bytecode(
            offset,
            format!("operand stack has {value:?}, expected a reference"),
        ))
    }
}

fn require_empty_stack(frame: &FrameState, offset: usize) -> Result<()> {
    if frame.stack.is_empty() {
        Ok(())
    } else {
        Err(Error::invalid_bytecode(
            offset,
            "return leaves extra values on the operand stack",
        ))
    }
}

fn constant_index(instruction: &Instruction) -> Result<u16> {
    match instruction.operand {
        Operand::Constant(index) => Ok(index),
        _ => Err(Error::invalid_bytecode(
            instruction.offset,
            "constant-pool index is missing",
        )),
    }
}

fn reference_component(component: &str) -> Option<String> {
    if let Some(name) = component
        .strip_prefix('L')
        .and_then(|name| name.strip_suffix(';'))
    {
        Some(name.to_owned())
    } else if component.starts_with('[') {
        Some(component.to_owned())
    } else {
        None
    }
}

const fn primitive_array_descriptor(array_type: ArrayType) -> &'static str {
    match array_type {
        ArrayType::Boolean => "[Z",
        ArrayType::Char => "[C",
        ArrayType::Float => "[F",
        ArrayType::Double => "[D",
        ArrayType::Byte => "[B",
        ArrayType::Short => "[S",
        ArrayType::Int => "[I",
        ArrayType::Long => "[J",
    }
}

fn local_type_error(offset: usize, index: u16, actual: &FrameValue, expected: &str) -> Error {
    Error::invalid_bytecode(
        offset,
        format!("local {index} is {actual:?}, expected {expected}"),
    )
}

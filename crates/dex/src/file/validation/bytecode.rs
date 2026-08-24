//! Method declarations and statically checkable Dalvik constraints.

use crate::file::layout::{UNLOCATED_ERROR_OFFSET, UNREPRESENTABLE_FILE_OFFSET};
use crate::file::validation::descriptor::{DescriptorKind, RegisterWidth};
use crate::file::{
    AccessFlags, CallSite, DexString, DexVersion, EncodedMethod, FieldId, MethodHandle, MethodId,
    PrototypeId, TypeId,
};
use crate::instruction::{IndexKind, Instruction, InstructionData, Opcode, Operands};
use crate::{Error, Result};

const METHOD_ENTRY_INSTRUCTION_OFFSET: u32 = 0;
const WIDE_REGISTER_LAST_DELTA: u16 = 1;
const NEXT_REGISTER_LIST_POSITION: usize = 1;

#[allow(clippy::too_many_arguments)]
pub(super) fn method(
    version: DexVersion,
    strings: &[DexString],
    types: &[TypeId],
    descriptors: &[DescriptorKind],
    prototypes: &[PrototypeId],
    fields: &[FieldId],
    methods: &[MethodId],
    call_sites: &[CallSite],
    method_handles: &[MethodHandle],
    declaration: &EncodedMethod,
) -> Result<()> {
    let identity = get(methods, declaration.method.get(), "declared method")?;
    let prototype = get(
        prototypes,
        identity.prototype.get(),
        "declared method prototype",
    )?;
    let class_name = type_name(strings, types, identity.class.get())
        .unwrap_or_else(|| format!("type@{}", identity.class.get()));
    let method_name = get(strings, identity.name.get(), "method name")?
        .text
        .clone();
    let signature = prototype_text(strings, types, prototype);
    validate_method_body(
        version,
        strings.len(),
        descriptors,
        prototypes,
        fields,
        methods,
        call_sites,
        method_handles,
        prototype,
        declaration,
    )
    .map_err(|error| error.in_method(class_name, method_name, signature))
}

#[allow(clippy::too_many_arguments)]
fn validate_method_body(
    version: DexVersion,
    string_count: usize,
    descriptors: &[DescriptorKind],
    prototypes: &[PrototypeId],
    fields: &[FieldId],
    methods: &[MethodId],
    call_sites: &[CallSite],
    method_handles: &[MethodHandle],
    prototype: &PrototypeId,
    declaration: &EncodedMethod,
) -> Result<()> {
    let lacks_code = declaration.access_flags.contains(AccessFlags::ABSTRACT)
        || declaration.access_flags.contains(AccessFlags::NATIVE);
    if lacks_code == declaration.code.is_some() {
        return Err(Error::invalid_dex(
            declaration
                .code
                .as_ref()
                .map_or(UNLOCATED_ERROR_OFFSET, |code| {
                    usize::try_from(code.data_offset).unwrap_or(UNREPRESENTABLE_FILE_OFFSET)
                }),
            "abstract/native code presence does not match method access flags",
        ));
    }
    let Some(code) = &declaration.code else {
        return Ok(());
    };
    if code.instructions.is_empty() {
        return Err(Error::invalid_dex(
            usize::try_from(code.data_offset).unwrap_or(UNREPRESENTABLE_FILE_OFFSET),
            "method instruction stream is empty",
        ));
    }
    crate::analysis::analyze_body(code)?;
    let expected_incoming = incoming_words(
        descriptors,
        prototype,
        !declaration.access_flags.contains(AccessFlags::STATIC),
        METHOD_ENTRY_INSTRUCTION_OFFSET,
    )?;
    if u32::from(code.ins_size) != expected_incoming {
        return Err(Error::invalid_dex(
            usize::try_from(code.data_offset).unwrap_or(UNREPRESENTABLE_FILE_OFFSET),
            format!(
                "method declares {} incoming words but its prototype needs {expected_incoming}",
                code.ins_size
            ),
        ));
    }
    if code
        .debug_info
        .as_ref()
        .is_some_and(|debug| debug.parameter_names.len() != prototype.parameters.len())
    {
        return Err(Error::invalid_dex(
            code.debug_info
                .as_ref()
                .map_or(UNLOCATED_ERROR_OFFSET, |debug| {
                    usize::try_from(debug.data_offset).unwrap_or(UNREPRESENTABLE_FILE_OFFSET)
                }),
            "debug parameter-name count does not match the method prototype",
        ));
    }
    let return_kind = *get(
        descriptors,
        prototype.return_type.get(),
        "method return type",
    )?;
    for instruction in &code.instructions {
        let InstructionData::Operation { opcode, operands } = instruction.data() else {
            continue;
        };
        validate_opcode_version(version, *opcode, instruction.offset())?;
        validate_index(
            *opcode,
            operands,
            string_count,
            descriptors.len(),
            fields.len(),
            methods.len(),
            prototypes.len(),
            call_sites.len(),
            method_handles.len(),
            instruction.offset(),
        )?;
        validate_type_operand(*opcode, operands, descriptors, instruction.offset())?;
        validate_wide_registers(instruction, code.registers_size)?;
        validate_return(*opcode, return_kind, instruction.offset())?;
        validate_invocation(
            *opcode,
            operands,
            descriptors,
            prototypes,
            methods,
            call_sites,
            code.outs_size,
            instruction.offset(),
        )?;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn validate_index(
    opcode: Opcode,
    operands: &Operands,
    string_count: usize,
    type_count: usize,
    field_count: usize,
    method_count: usize,
    prototype_count: usize,
    call_site_count: usize,
    method_handle_count: usize,
    offset: u32,
) -> Result<()> {
    let Some(kind) = opcode.index_kind() else {
        return Ok(());
    };
    let (index, secondary) = match operands {
        Operands::RegisterIndex { index, .. }
        | Operands::RegistersIndex { index, .. }
        | Operands::RegisterListIndex {
            index,
            secondary_index: None,
            ..
        }
        | Operands::RegisterRangeIndex {
            index,
            secondary_index: None,
            ..
        } => (*index, None),
        Operands::RegisterListIndex {
            index,
            secondary_index,
            ..
        }
        | Operands::RegisterRangeIndex {
            index,
            secondary_index,
            ..
        } => (*index, *secondary_index),
        _ => {
            return Err(Error::invalid_instruction(
                offset,
                format!("{} lacks its indexed operand", opcode.mnemonic()),
            ));
        }
    };
    let limit = match kind {
        IndexKind::String => string_count,
        IndexKind::Type => type_count,
        IndexKind::Field => field_count,
        IndexKind::Method => method_count,
        IndexKind::Prototype => prototype_count,
        IndexKind::CallSite => call_site_count,
        IndexKind::MethodHandle => method_handle_count,
    };
    if usize::try_from(index).map_or(true, |index| index >= limit) {
        return Err(Error::invalid_instruction(
            offset,
            format!(
                "{} {:?} index {index} is outside 0..{limit}",
                opcode.mnemonic(),
                kind
            ),
        ));
    }
    if secondary
        .is_some_and(|index| usize::try_from(index).map_or(true, |index| index >= prototype_count))
    {
        return Err(Error::invalid_instruction(
            offset,
            format!("{} prototype index is out of bounds", opcode.mnemonic()),
        ));
    }
    Ok(())
}

fn validate_opcode_version(version: DexVersion, opcode: Opcode, offset: u32) -> Result<()> {
    let minimum = match opcode {
        Opcode::InvokePolymorphic
        | Opcode::InvokePolymorphicRange
        | Opcode::InvokeCustom
        | Opcode::InvokeCustomRange => Some(DexVersion::V038),
        Opcode::ConstMethodHandle | Opcode::ConstMethodType => Some(DexVersion::V039),
        _ => None,
    };
    if minimum.is_some_and(|minimum| version < minimum) {
        Err(Error::invalid_instruction(
            offset,
            format!(
                "{} is unavailable in DEX version {}",
                opcode.mnemonic(),
                String::from_utf8_lossy(&version.digits())
            ),
        ))
    } else {
        Ok(())
    }
}

fn validate_type_operand(
    opcode: Opcode,
    operands: &Operands,
    descriptors: &[DescriptorKind],
    offset: u32,
) -> Result<()> {
    let index = match operands {
        Operands::RegisterIndex { index, .. }
        | Operands::RegistersIndex { index, .. }
        | Operands::RegisterListIndex { index, .. }
        | Operands::RegisterRangeIndex { index, .. }
            if opcode.index_kind() == Some(IndexKind::Type) =>
        {
            *index
        }
        _ => return Ok(()),
    };
    let kind = *get(descriptors, index, "instruction type")?;
    let valid = match opcode {
        Opcode::NewInstance => kind.is_class(),
        Opcode::NewArray
        | Opcode::FilledNewArray
        | Opcode::FilledNewArrayRange
        | Opcode::FillArrayData => matches!(kind, DescriptorKind::Array),
        _ => !kind.is_void(),
    };
    if valid {
        Ok(())
    } else {
        Err(Error::invalid_instruction(
            offset,
            format!("{} references an incompatible type", opcode.mnemonic()),
        ))
    }
}

fn validate_wide_registers(instruction: &Instruction, register_count: u16) -> Result<()> {
    let InstructionData::Operation { opcode, operands } = instruction.data() else {
        return Ok(());
    };
    for register in wide_bases(*opcode, operands) {
        if register
            .checked_add(WIDE_REGISTER_LAST_DELTA)
            .is_none_or(|last| last >= register_count)
        {
            return Err(Error::invalid_instruction(
                instruction.offset(),
                format!(
                    "{} wide operand v{register}..v{} exceeds the register frame",
                    opcode.mnemonic(),
                    register.saturating_add(WIDE_REGISTER_LAST_DELTA)
                ),
            ));
        }
    }
    Ok(())
}

#[allow(clippy::too_many_lines)]
fn wide_bases(opcode: Opcode, operands: &Operands) -> Vec<u16> {
    let first = first_register(operands);
    let second = second_register(operands);
    let third = third_register(operands);
    match opcode {
        Opcode::MoveWide
        | Opcode::MoveWideFrom16
        | Opcode::MoveWide16
        | Opcode::NegLong
        | Opcode::NotLong
        | Opcode::NegDouble
        | Opcode::LongToDouble
        | Opcode::DoubleToLong
        | Opcode::ShlLong
        | Opcode::ShrLong
        | Opcode::UshrLong
        | Opcode::AddLong2Addr
        | Opcode::SubLong2Addr
        | Opcode::MulLong2Addr
        | Opcode::DivLong2Addr
        | Opcode::RemLong2Addr
        | Opcode::AndLong2Addr
        | Opcode::OrLong2Addr
        | Opcode::XorLong2Addr
        | Opcode::AddDouble2Addr
        | Opcode::SubDouble2Addr
        | Opcode::MulDouble2Addr
        | Opcode::DivDouble2Addr
        | Opcode::RemDouble2Addr => first.into_iter().chain(second).collect(),
        Opcode::MoveResultWide
        | Opcode::ReturnWide
        | Opcode::ConstWide16
        | Opcode::ConstWide32
        | Opcode::ConstWide
        | Opcode::ConstWideHigh16
        | Opcode::AgetWide
        | Opcode::AputWide
        | Opcode::IgetWide
        | Opcode::IputWide
        | Opcode::SgetWide
        | Opcode::SputWide
        | Opcode::IntToLong
        | Opcode::IntToDouble
        | Opcode::FloatToLong
        | Opcode::FloatToDouble => first.into_iter().collect(),
        Opcode::CmplDouble | Opcode::CmpgDouble | Opcode::CmpLong => {
            second.into_iter().chain(third).collect()
        }
        Opcode::LongToInt | Opcode::LongToFloat | Opcode::DoubleToInt | Opcode::DoubleToFloat => {
            second.into_iter().collect()
        }
        Opcode::AddLong
        | Opcode::SubLong
        | Opcode::MulLong
        | Opcode::DivLong
        | Opcode::RemLong
        | Opcode::AndLong
        | Opcode::OrLong
        | Opcode::XorLong
        | Opcode::AddDouble
        | Opcode::SubDouble
        | Opcode::MulDouble
        | Opcode::DivDouble
        | Opcode::RemDouble => first.into_iter().chain(second).chain(third).collect(),
        Opcode::ShlLong2Addr | Opcode::ShrLong2Addr | Opcode::UshrLong2Addr => {
            first.into_iter().collect()
        }
        _ => Vec::new(),
    }
}

fn validate_return(opcode: Opcode, return_kind: DescriptorKind, offset: u32) -> Result<()> {
    let is_return = matches!(
        opcode,
        Opcode::ReturnVoid | Opcode::Return | Opcode::ReturnWide | Opcode::ReturnObject
    );
    if !is_return {
        return Ok(());
    }
    let valid = match opcode {
        Opcode::ReturnVoid => return_kind.is_void(),
        Opcode::ReturnWide => return_kind.register_width() == Some(RegisterWidth::Double),
        Opcode::ReturnObject => {
            matches!(return_kind, DescriptorKind::Class | DescriptorKind::Array)
        }
        Opcode::Return => {
            matches!(return_kind, DescriptorKind::Primitive { .. })
                && return_kind.register_width() == Some(RegisterWidth::Single)
        }
        _ => true,
    };
    if valid {
        Ok(())
    } else {
        Err(Error::invalid_instruction(
            offset,
            format!(
                "{} does not match the method return type",
                opcode.mnemonic()
            ),
        ))
    }
}

#[allow(clippy::too_many_arguments)]
fn validate_invocation(
    opcode: Opcode,
    operands: &Operands,
    descriptors: &[DescriptorKind],
    prototypes: &[PrototypeId],
    methods: &[MethodId],
    call_sites: &[CallSite],
    outs_size: u16,
    offset: u32,
) -> Result<()> {
    let Some((prototype, receiver)) =
        invocation_prototype(opcode, operands, prototypes, methods, call_sites, offset)?
    else {
        return Ok(());
    };
    let expected = incoming_words(descriptors, prototype, receiver, offset)?;
    let actual = invocation_count(operands).ok_or_else(|| {
        Error::invalid_instruction(
            offset,
            format!("{} lacks invocation registers", opcode.mnemonic()),
        )
    })?;
    if actual != expected || u32::from(outs_size) < actual {
        return Err(Error::invalid_instruction(
            offset,
            format!(
                "{} supplies {actual} register words, needs {expected}, and method outs_size is {outs_size}",
                opcode.mnemonic()
            ),
        ));
    }
    validate_explicit_wide_arguments(operands, descriptors, prototype, receiver, offset)
}

fn invocation_prototype<'a>(
    opcode: Opcode,
    operands: &Operands,
    prototypes: &'a [PrototypeId],
    methods: &[MethodId],
    call_sites: &[CallSite],
    offset: u32,
) -> Result<Option<(&'a PrototypeId, bool)>> {
    let (primary, secondary) = match operands {
        Operands::RegisterListIndex {
            index,
            secondary_index,
            ..
        }
        | Operands::RegisterRangeIndex {
            index,
            secondary_index,
            ..
        } => (*index, *secondary_index),
        _ => return Ok(None),
    };
    let result = match opcode {
        Opcode::InvokeStatic | Opcode::InvokeStaticRange => {
            let method = get(methods, primary, "invoked method")?;
            Some((
                get(prototypes, method.prototype.get(), "invoked prototype")?,
                false,
            ))
        }
        Opcode::InvokeVirtual
        | Opcode::InvokeSuper
        | Opcode::InvokeDirect
        | Opcode::InvokeInterface
        | Opcode::InvokeVirtualRange
        | Opcode::InvokeSuperRange
        | Opcode::InvokeDirectRange
        | Opcode::InvokeInterfaceRange => {
            let method = get(methods, primary, "invoked method")?;
            Some((
                get(prototypes, method.prototype.get(), "invoked prototype")?,
                true,
            ))
        }
        Opcode::InvokePolymorphic | Opcode::InvokePolymorphicRange => Some((
            get(
                prototypes,
                secondary.ok_or_else(|| {
                    Error::invalid_instruction(offset, "missing polymorphic prototype")
                })?,
                "polymorphic prototype",
            )?,
            true,
        )),
        Opcode::InvokeCustom | Opcode::InvokeCustomRange => {
            let call_site = get(call_sites, primary, "invoked call site")?;
            let Some(components) = call_site.components() else {
                return Err(Error::invalid_instruction(
                    offset,
                    "call-site method type is missing",
                ));
            };
            Some((
                get(
                    prototypes,
                    components.method_type.get(),
                    "call-site prototype",
                )?,
                false,
            ))
        }
        _ => None,
    };
    Ok(result)
}

fn validate_explicit_wide_arguments(
    operands: &Operands,
    descriptors: &[DescriptorKind],
    prototype: &PrototypeId,
    receiver: bool,
    offset: u32,
) -> Result<()> {
    let Operands::RegisterListIndex { registers, .. } = operands else {
        return Ok(());
    };
    let mut cursor = usize::from(receiver);
    for parameter in &prototype.parameters {
        let kind = *get(descriptors, parameter.get(), "invocation parameter")?;
        let width = kind.register_width().ok_or_else(|| {
            Error::invalid_instruction(offset, "invocation parameter cannot have void type")
        })?;
        if width == RegisterWidth::Double {
            let first = registers.get(cursor).copied();
            let second = registers.get(cursor + NEXT_REGISTER_LIST_POSITION).copied();
            if first.zip(second).is_none_or(|(first, second)| {
                first.checked_add(WIDE_REGISTER_LAST_DELTA) != Some(second)
            }) {
                return Err(Error::invalid_instruction(
                    offset,
                    "wide invocation argument does not use adjacent registers",
                ));
            }
        }
        cursor += usize::from(width.words());
    }
    Ok(())
}

fn incoming_words(
    descriptors: &[DescriptorKind],
    prototype: &PrototypeId,
    receiver: bool,
    offset: u32,
) -> Result<u32> {
    prototype
        .parameters
        .iter()
        .try_fold(u32::from(receiver), |total, parameter| {
            let kind = get(descriptors, parameter.get(), "parameter type")?;
            let words = u32::from(
                kind.register_width()
                    .ok_or_else(|| {
                        Error::invalid_instruction(offset, "parameter cannot have void type")
                    })?
                    .words(),
            );
            total.checked_add(words).ok_or_else(|| {
                Error::invalid_instruction(offset, "prototype register width overflowed")
            })
        })
}

fn invocation_count(operands: &Operands) -> Option<u32> {
    match operands {
        Operands::RegisterListIndex { registers, .. } => u32::try_from(registers.len()).ok(),
        Operands::RegisterRangeIndex { count, .. } => Some(u32::from(*count)),
        _ => None,
    }
}

fn first_register(operands: &Operands) -> Option<u16> {
    match operands {
        Operands::Register(register)
        | Operands::RegisterLiteral { register, .. }
        | Operands::RegisterBranch { register, .. }
        | Operands::RegisterIndex { register, .. } => Some(*register),
        Operands::Registers { first, .. }
        | Operands::ThreeRegisters { first, .. }
        | Operands::RegistersLiteral { first, .. }
        | Operands::RegistersBranch { first, .. }
        | Operands::RegistersIndex { first, .. } => Some(*first),
        _ => None,
    }
}

fn second_register(operands: &Operands) -> Option<u16> {
    match operands {
        Operands::Registers { second, .. }
        | Operands::ThreeRegisters { second, .. }
        | Operands::RegistersLiteral { second, .. }
        | Operands::RegistersBranch { second, .. }
        | Operands::RegistersIndex { second, .. } => Some(*second),
        _ => None,
    }
}

fn third_register(operands: &Operands) -> Option<u16> {
    match operands {
        Operands::ThreeRegisters { third, .. } => Some(*third),
        _ => None,
    }
}

fn get<'a, T>(values: &'a [T], index: u32, what: &str) -> Result<&'a T> {
    usize::try_from(index)
        .ok()
        .and_then(|index| values.get(index))
        .ok_or_else(|| {
            Error::invalid_dex(
                UNLOCATED_ERROR_OFFSET,
                format!("{what} index {index} is out of bounds"),
            )
        })
}

fn type_name(strings: &[DexString], types: &[TypeId], index: u32) -> Option<String> {
    let descriptor = types.get(usize::try_from(index).ok()?)?.descriptor.get();
    Some(strings.get(usize::try_from(descriptor).ok()?)?.text.clone())
}

fn prototype_text(strings: &[DexString], types: &[TypeId], prototype: &PrototypeId) -> String {
    let mut output = String::from("(");
    for parameter in &prototype.parameters {
        output.push_str(
            &type_name(strings, types, parameter.get())
                .unwrap_or_else(|| format!("type@{}", parameter.get())),
        );
    }
    output.push(')');
    output.push_str(
        &type_name(strings, types, prototype.return_type.get())
            .unwrap_or_else(|| format!("type@{}", prototype.return_type.get())),
    );
    output
}

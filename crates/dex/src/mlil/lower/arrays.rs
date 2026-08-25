//! Target legalization for MLIL array forms without direct Dalvik encodings.

use ::mlil::{ArrayType, Instruction, ValueType};
use disassembler::{
    AddressRange, CodeAddress, ExactText, Reference, ReferenceKind, ReferenceSymbol,
};

use crate::file::DexFile;
use crate::instruction::{IndexKind, Opcode, Operands};

use super::instruction::{MoveKind, move_value, plain, require_u16_index};
use super::layout::Planner;
use super::registers::RegisterAllocation;
use super::{DexMlilReferenceResolver, Error, Result};

const WORK_WORDS: u16 = 8;
const NEGATIVE_ARRAY_SIZE_DESCRIPTOR: &str = "Ljava/lang/NegativeArraySizeException;";
const CONSTRUCTOR_NAME: &str = "<init>";
const VOID_METHOD_DESCRIPTOR: &str = "()V";

/// Expands JVM's single-operation multidimensional allocation into nested
/// Dalvik `new-array` loops while retaining one protected throw range.
pub(super) fn emit_multidimensional_array<R: DexMlilReferenceResolver>(
    planner: &mut Planner,
    instruction: &Instruction,
    allocation: &RegisterAllocation,
    file: &DexFile,
    resolver: &mut R,
    array_type: &ArrayType,
    dimensions: u8,
) -> Result<Option<AddressRange>> {
    validate_dimensions(array_type.descriptor(), dimensions, instruction)?;
    let dimensions = u16::from(dimensions);
    let dimension_base = WORK_WORDS;
    let array_base = dimension_base + dimensions;
    let index_base = array_base + dimensions - 1;
    let negative = planner.new_label()?;
    let done = planner.new_label()?;

    for (position, &variable) in instruction.uses().iter().enumerate() {
        let persistent = dimension_base
            + u16::try_from(position).map_err(|_| {
                Error::lowering(instruction.id(), "array dimension position exceeds u16")
            })?;
        move_value(
            planner,
            persistent,
            allocation.register(variable),
            MoveKind::Narrow,
        )?;
        move_value(planner, 0, persistent, MoveKind::Narrow)?;
        planner.conditional_skip(Opcode::IfGez, 0, None)?;
        planner.goto_label(negative)?;
    }

    let throw_start = planner.cursor();
    let root_type = resolve_array_type(file, resolver, array_type.descriptor(), instruction)?;
    move_value(planner, 1, dimension_base, MoveKind::Narrow)?;
    plain(
        planner,
        Opcode::NewArray,
        Operands::RegistersIndex {
            first: 0,
            second: 1,
            index: root_type,
        },
    )?;
    let definition = *instruction.defs().first().ok_or_else(|| {
        Error::lowering(
            instruction.id(),
            "multidimensional array allocation has no result variable",
        )
    })?;
    let root_register = allocation.register(definition);
    move_value(planner, root_register, 0, MoveKind::Object)?;

    emit_child_loop(
        planner,
        instruction,
        file,
        resolver,
        array_type.descriptor(),
        dimensions,
        1,
        root_register,
        dimension_base,
        array_base,
        index_base,
    )?;
    planner.goto_label(done)?;

    planner.bind_label(negative)?;
    emit_negative_array_size(planner, instruction, file, resolver)?;
    planner.bind_label(done)?;
    let throw_end = planner.cursor();
    Ok(instruction
        .may_throw()
        .then(|| AddressRange::new(CodeAddress::from(throw_start), CodeAddress::from(throw_end))))
}

/// Expands initialized arrays that cannot use Dalvik's narrow
/// `filled-new-array` word list into allocation plus typed stores.
pub(super) fn emit_initialized_array<R: DexMlilReferenceResolver>(
    planner: &mut Planner,
    instruction: &Instruction,
    allocation: &RegisterAllocation,
    file: &DexFile,
    resolver: &mut R,
    array_type: &ArrayType,
) -> Result<Option<AddressRange>> {
    let descriptor = array_type.descriptor();
    let array_index = resolve_array_type(file, resolver, descriptor, instruction)?;
    let length = i64::try_from(instruction.uses().len())
        .map_err(|_| Error::lowering(instruction.id(), "initialized array length exceeds i64"))?;
    plain(
        planner,
        Opcode::Const,
        Operands::RegisterLiteral {
            register: 1,
            literal: length,
        },
    )?;
    let throw_start = planner.cursor();
    plain(
        planner,
        Opcode::NewArray,
        Operands::RegistersIndex {
            first: 0,
            second: 1,
            index: array_index,
        },
    )?;
    let definition = *instruction.defs().first().ok_or_else(|| {
        Error::lowering(
            instruction.id(),
            "initialized array allocation has no result variable",
        )
    })?;
    let array_register = allocation.register(definition);
    move_value(planner, array_register, 0, MoveKind::Object)?;
    let store = initialized_store_opcode(descriptor, instruction)?;
    for (position, (&variable, value_type)) in instruction
        .uses()
        .iter()
        .zip(instruction.use_types())
        .enumerate()
    {
        move_value(
            planner,
            0,
            allocation.register(variable),
            value_move_kind(value_type),
        )?;
        move_value(planner, 2, array_register, MoveKind::Object)?;
        let index = i64::try_from(position)
            .map_err(|_| Error::lowering(instruction.id(), "array index exceeds i64"))?;
        plain(
            planner,
            Opcode::Const,
            Operands::RegisterLiteral {
                register: 3,
                literal: index,
            },
        )?;
        plain(
            planner,
            store,
            Operands::ThreeRegisters {
                first: 0,
                second: 2,
                third: 3,
            },
        )?;
    }
    let throw_end = planner.cursor();
    Ok(instruction
        .may_throw()
        .then(|| AddressRange::new(CodeAddress::from(throw_start), CodeAddress::from(throw_end))))
}

pub(super) fn initialized_array_needs_expansion(instruction: &Instruction) -> bool {
    instruction.uses().len() > usize::from(u8::MAX)
        || instruction.use_types().iter().any(|value| {
            matches!(
                value,
                ValueType::Long | ValueType::Double | ValueType::Bits64
            )
        })
}

#[allow(clippy::too_many_arguments)]
fn emit_child_loop<R: DexMlilReferenceResolver>(
    planner: &mut Planner,
    instruction: &Instruction,
    file: &DexFile,
    resolver: &mut R,
    descriptor: &str,
    dimensions: u16,
    level: u16,
    parent_register: u16,
    dimension_base: u16,
    array_base: u16,
    index_base: u16,
) -> Result<()> {
    if level >= dimensions {
        return Ok(());
    }
    let child_register = array_base + level - 1;
    let index_register = index_base + level - 1;
    const_narrow(planner, index_register, 0)?;
    let repeat = planner.new_label()?;
    let complete = planner.new_label()?;
    planner.bind_label(repeat)?;

    move_value(planner, 0, index_register, MoveKind::Narrow)?;
    move_value(planner, 1, dimension_base + level - 1, MoveKind::Narrow)?;
    planner.conditional_skip(Opcode::IfLt, 0, Some(1))?;
    planner.goto_label(complete)?;

    let child_descriptor = descriptor
        .get(usize::from(level)..)
        .ok_or_else(|| Error::lowering(instruction.id(), "array descriptor depth overflowed"))?;
    let child_type = resolve_array_type(file, resolver, child_descriptor, instruction)?;
    move_value(planner, 1, dimension_base + level, MoveKind::Narrow)?;
    plain(
        planner,
        Opcode::NewArray,
        Operands::RegistersIndex {
            first: 0,
            second: 1,
            index: child_type,
        },
    )?;
    move_value(planner, child_register, 0, MoveKind::Object)?;

    move_value(planner, 0, child_register, MoveKind::Object)?;
    move_value(planner, 1, parent_register, MoveKind::Object)?;
    move_value(planner, 2, index_register, MoveKind::Narrow)?;
    plain(
        planner,
        Opcode::AputObject,
        Operands::ThreeRegisters {
            first: 0,
            second: 1,
            third: 2,
        },
    )?;

    emit_child_loop(
        planner,
        instruction,
        file,
        resolver,
        descriptor,
        dimensions,
        level + 1,
        child_register,
        dimension_base,
        array_base,
        index_base,
    )?;
    move_value(planner, 0, index_register, MoveKind::Narrow)?;
    plain(
        planner,
        Opcode::AddIntLit8,
        Operands::RegistersLiteral {
            first: 0,
            second: 0,
            literal: 1,
        },
    )?;
    move_value(planner, index_register, 0, MoveKind::Narrow)?;
    planner.goto_label(repeat)?;
    planner.bind_label(complete)
}

fn emit_negative_array_size<R: DexMlilReferenceResolver>(
    planner: &mut Planner,
    instruction: &Instruction,
    file: &DexFile,
    resolver: &mut R,
) -> Result<()> {
    let exception_type = resolver
        .resolve_type(file, NEGATIVE_ARRAY_SIZE_DESCRIPTOR)
        .map_err(|source| Error::Reference {
            instruction: instruction.id(),
            source,
        })?
        .get();
    require_u16_index(exception_type, instruction.id(), "exception type")?;
    plain(
        planner,
        Opcode::NewInstance,
        Operands::RegisterIndex {
            register: 0,
            index: exception_type,
        },
    )?;

    let constructor = Reference::resolved(
        ReferenceKind::Method,
        u32::MAX,
        "java.lang.NegativeArraySizeException.<init>:()V",
    )
    .with_symbol(ReferenceSymbol::Method {
        owner: NEGATIVE_ARRAY_SIZE_DESCRIPTOR.to_owned(),
        name: ExactText::new(CONSTRUCTOR_NAME),
        descriptor: VOID_METHOD_DESCRIPTOR.to_owned(),
    });
    let constructor = resolver
        .resolve(file, &constructor, IndexKind::Method)
        .map_err(|source| Error::Reference {
            instruction: instruction.id(),
            source,
        })?;
    require_u16_index(constructor, instruction.id(), "constructor")?;
    plain(
        planner,
        Opcode::InvokeDirectRange,
        Operands::RegisterRangeIndex {
            start: 0,
            count: 1,
            index: constructor,
            secondary_index: None,
        },
    )?;
    plain(planner, Opcode::Throw, Operands::Register(0))
}

fn resolve_array_type<R: DexMlilReferenceResolver>(
    file: &DexFile,
    resolver: &mut R,
    descriptor: &str,
    instruction: &Instruction,
) -> Result<u32> {
    let index = resolver
        .resolve_type(file, descriptor)
        .map_err(|source| Error::Reference {
            instruction: instruction.id(),
            source,
        })?
        .get();
    require_u16_index(index, instruction.id(), "array type")?;
    Ok(index)
}

fn const_narrow(planner: &mut Planner, register: u16, value: i64) -> Result<()> {
    plain(
        planner,
        Opcode::Const,
        Operands::RegisterLiteral {
            register: 0,
            literal: value,
        },
    )?;
    move_value(planner, register, 0, MoveKind::Narrow)
}

fn validate_dimensions(descriptor: &str, dimensions: u8, instruction: &Instruction) -> Result<()> {
    if dimensions < 2
        || usize::from(dimensions) > descriptor.bytes().take_while(|b| *b == b'[').count()
    {
        return Err(Error::lowering(
            instruction.id(),
            "multidimensional allocation disagrees with its array descriptor",
        ));
    }
    if instruction.uses().len() != usize::from(dimensions) {
        return Err(Error::lowering(
            instruction.id(),
            "multidimensional allocation has the wrong number of dimension operands",
        ));
    }
    Ok(())
}

fn value_move_kind(value_type: &ValueType) -> MoveKind {
    if value_type.is_reference() {
        return MoveKind::Object;
    }
    match value_type {
        ValueType::Long | ValueType::Double | ValueType::Bits64 => MoveKind::Wide,
        _ => MoveKind::Narrow,
    }
}

fn initialized_store_opcode(descriptor: &str, instruction: &Instruction) -> Result<Opcode> {
    let component = descriptor
        .strip_prefix('[')
        .and_then(|value| value.as_bytes().first())
        .copied()
        .ok_or_else(|| Error::lowering(instruction.id(), "array has no component descriptor"))?;
    Ok(match component {
        b'Z' => Opcode::AputBoolean,
        b'B' => Opcode::AputByte,
        b'C' => Opcode::AputChar,
        b'S' => Opcode::AputShort,
        b'I' | b'F' => Opcode::Aput,
        b'J' | b'D' => Opcode::AputWide,
        b'L' | b'[' => Opcode::AputObject,
        _ => {
            return Err(Error::lowering(
                instruction.id(),
                "initialized array has an invalid component descriptor",
            ));
        }
    })
}

//! JVM legalization for target-neutral MLIL array operations.

use ::mlil::{
    AllocationKind, ArrayAccess, ArrayType as MlilArrayType, Constant, ElementType, Function,
    Instruction,
};
use disassembler::{Reference, ReferenceKind, ReferenceSymbol};

use crate::JavaReferenceResolver;
use crate::bytecode::{ArrayType, CodeBuilder, Label, Opcode, Operand};
use crate::classfile::ConstantPool;

use super::super::{Error, Result};
use super::instruction::{
    array_opcode, emit_constant, emit_integer, load_use, load_uses, plain, primary,
    store_definitions,
};
use super::locals::LocalAllocation;

#[allow(clippy::too_many_arguments)]
pub(super) fn emit_allocation<R: JavaReferenceResolver>(
    builder: &mut CodeBuilder,
    instruction: &Instruction,
    allocation: &LocalAllocation,
    function: &Function,
    kind: &AllocationKind,
    pool: &mut ConstantPool,
    resolver: &mut R,
    throw_range: &mut Option<(Label, Label)>,
) -> Result<()> {
    match kind {
        AllocationKind::Object(reference) => {
            *throw_range = primary(builder, instruction, |builder| {
                let index = resolve(reference, instruction, pool, resolver)?;
                plain(builder, Opcode::New, Operand::Constant(index));
                Ok(())
            })?;
        }
        AllocationKind::Array {
            array_type,
            dimensions,
        } => {
            load_uses(builder, instruction, allocation, function)?;
            *throw_range = primary(builder, instruction, |builder| {
                emit_new_array(
                    builder,
                    instruction,
                    array_type,
                    *dimensions,
                    pool,
                    resolver,
                )
            })?;
        }
        AllocationKind::InitializedArray { array_type } => {
            let descriptor = array_type.descriptor();
            let count = i32::try_from(instruction.uses().len()).map_err(|_| {
                Error::lowering(instruction.id(), "initialized array length exceeds i32")
            })?;
            *throw_range = primary(builder, instruction, |builder| {
                emit_integer(builder, count, pool)?;
                emit_new_array(builder, instruction, array_type, 1, pool, resolver)?;
                let element = array_element(descriptor, instruction)?;
                for (position, &variable) in instruction.uses().iter().enumerate() {
                    plain(builder, Opcode::Dup, Operand::None);
                    emit_integer(
                        builder,
                        i32::try_from(position).map_err(|_| {
                            Error::lowering(instruction.id(), "array index exceeds i32")
                        })?,
                        pool,
                    )?;
                    load_use(
                        builder,
                        instruction,
                        allocation,
                        function,
                        position,
                        variable,
                    )?;
                    plain(
                        builder,
                        array_opcode(ArrayAccess::Put, element),
                        Operand::None,
                    );
                }
                Ok(())
            })?;
        }
    }
    store_definitions(builder, instruction, allocation)
}

#[allow(clippy::too_many_arguments)]
pub(super) fn emit_array_initialization<R: JavaReferenceResolver>(
    builder: &mut CodeBuilder,
    instruction: &Instruction,
    allocation: &LocalAllocation,
    function: &Function,
    array_type: &MlilArrayType,
    values: &[Constant],
    pool: &mut ConstantPool,
    resolver: &mut R,
) -> Result<Option<(Label, Label)>> {
    let descriptor = array_type.descriptor();
    let element = array_element(descriptor, instruction)?;
    primary(builder, instruction, |builder| {
        for (position, value) in values.iter().enumerate() {
            load_use(
                builder,
                instruction,
                allocation,
                function,
                0,
                instruction.uses()[0],
            )?;
            emit_integer(
                builder,
                i32::try_from(position)
                    .map_err(|_| Error::lowering(instruction.id(), "array index exceeds i32"))?,
                pool,
            )?;
            emit_constant(builder, value, instruction, pool, resolver)?;
            plain(
                builder,
                array_opcode(ArrayAccess::Put, element),
                Operand::None,
            );
        }
        Ok(())
    })
}

fn emit_new_array<R: JavaReferenceResolver>(
    builder: &mut CodeBuilder,
    instruction: &Instruction,
    array_type: &MlilArrayType,
    dimensions: u8,
    pool: &mut ConstantPool,
    resolver: &mut R,
) -> Result<()> {
    let descriptor = array_type.descriptor();
    if dimensions == 0 {
        return Err(Error::lowering(
            instruction.id(),
            "array allocation has zero dimensions",
        ));
    }
    if dimensions > 1 {
        let index = resolve_array_class(descriptor, array_type, instruction, pool, resolver)?;
        plain(
            builder,
            Opcode::MultiANewArray,
            Operand::MultiArray { index, dimensions },
        );
        return Ok(());
    }
    let component = descriptor.strip_prefix('[').ok_or_else(|| {
        Error::lowering(instruction.id(), "array result lacks an array descriptor")
    })?;
    if let Some(array_type) = primitive_array(component) {
        plain(builder, Opcode::NewArray, Operand::ArrayType(array_type));
    } else {
        let index = resolve_array_class(component, array_type, instruction, pool, resolver)?;
        plain(builder, Opcode::ANewArray, Operand::Constant(index));
    }
    Ok(())
}

fn resolve_array_class<R: JavaReferenceResolver>(
    descriptor: &str,
    array_type: &MlilArrayType,
    instruction: &Instruction,
    pool: &mut ConstantPool,
    resolver: &mut R,
) -> Result<u16> {
    let source = match array_type.source_reference() {
        Some(reference)
            if reference
                .symbol
                .as_ref()
                .and_then(type_symbol)
                .is_some_and(|source| equivalent_class_name(source, descriptor)) =>
        {
            reference.clone()
        }
        _ => Reference::resolved(ReferenceKind::Type, 0, descriptor)
            .with_symbol(ReferenceSymbol::Type(descriptor.to_owned())),
    };
    resolve(&source, instruction, pool, resolver)
}

fn array_element(descriptor: &str, instruction: &Instruction) -> Result<ElementType> {
    match descriptor
        .strip_prefix('[')
        .and_then(|value| value.as_bytes().first())
    {
        Some(b'Z') => Ok(ElementType::Boolean),
        Some(b'B') => Ok(ElementType::Byte),
        Some(b'C') => Ok(ElementType::Char),
        Some(b'S') => Ok(ElementType::Short),
        Some(b'I') => Ok(ElementType::Integer),
        Some(b'J') => Ok(ElementType::Long),
        Some(b'F') => Ok(ElementType::Float),
        Some(b'D') => Ok(ElementType::Double),
        Some(b'L' | b'[') => Ok(ElementType::Reference),
        _ => Err(Error::lowering(
            instruction.id(),
            "array descriptor has no valid component type",
        )),
    }
}

const fn primitive_array(component: &str) -> Option<ArrayType> {
    match component.as_bytes() {
        b"Z" => Some(ArrayType::Boolean),
        b"B" => Some(ArrayType::Byte),
        b"C" => Some(ArrayType::Char),
        b"S" => Some(ArrayType::Short),
        b"I" => Some(ArrayType::Int),
        b"J" => Some(ArrayType::Long),
        b"F" => Some(ArrayType::Float),
        b"D" => Some(ArrayType::Double),
        _ => None,
    }
}

fn type_symbol(symbol: &ReferenceSymbol) -> Option<&str> {
    match symbol {
        ReferenceSymbol::Type(descriptor) => Some(descriptor),
        _ => None,
    }
}

fn equivalent_class_name(left: &str, right: &str) -> bool {
    fn normalized(value: &str) -> &str {
        value
            .strip_prefix('L')
            .and_then(|value| value.strip_suffix(';'))
            .unwrap_or(value)
    }
    normalized(left) == normalized(right)
}

fn resolve<R: JavaReferenceResolver>(
    reference: &Reference,
    instruction: &Instruction,
    pool: &mut ConstantPool,
    resolver: &mut R,
) -> Result<u16> {
    resolver
        .resolve(reference, pool)
        .map_err(|source| Error::Reference {
            instruction: instruction.id(),
            source,
        })
}

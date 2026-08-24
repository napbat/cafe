//! Constant-pool resolution for indexed JVM instructions.

use crate::bytecode::{Instruction, Opcode, Operand};
use crate::classfile::{Constant, ConstantPool, MethodHandleKind, Utf8Constant};
use crate::descriptor;
use crate::{Error, Result};

use super::{
    ClassSymbol, DynamicSymbol, ExactString, FieldSymbol, InstructionReference, LoadableConstant,
    MethodHandleSymbol, MethodHandleTargetSymbol, MethodReferenceKind, MethodSymbol,
};

/// Resolves the constant-pool symbol carried by one JVM instruction.
///
/// Instructions without a symbolic constant operand return `None`. Exact
/// UTF-16 contents are retained for strings and member names.
///
/// # Errors
///
/// Returns an error for an invalid index or an opcode/constant tag mismatch.
pub fn resolve_instruction_reference(
    pool: &ConstantPool,
    instruction: &Instruction,
) -> Result<Option<InstructionReference>> {
    let offset = instruction.offset;
    let reference =
        match instruction.opcode {
            Opcode::Ldc | Opcode::LdcW | Opcode::Ldc2W => Some(InstructionReference::Constant(
                loadable(pool, constant_index(instruction)?, offset)?,
            )),
            Opcode::GetStatic | Opcode::PutStatic | Opcode::GetField | Opcode::PutField => Some(
                InstructionReference::Field(field(pool, constant_index(instruction)?, offset)?),
            ),
            Opcode::InvokeVirtual => Some(InstructionReference::Method(method(
                pool,
                invocation_index(instruction)?,
                offset,
                Some(MethodReferenceKind::Class),
            )?)),
            Opcode::InvokeInterface => Some(InstructionReference::Method(method(
                pool,
                invocation_index(instruction)?,
                offset,
                Some(MethodReferenceKind::Interface),
            )?)),
            Opcode::InvokeSpecial | Opcode::InvokeStatic => Some(InstructionReference::Method(
                method(pool, invocation_index(instruction)?, offset, None)?,
            )),
            Opcode::InvokeDynamic => {
                let Operand::InvokeDynamic(index) = instruction.operand else {
                    return Err(Error::invalid_bytecode(
                        offset,
                        "invokedynamic constant index is missing",
                    ));
                };
                Some(InstructionReference::DynamicCallSite(dynamic(
                    pool, index, true, offset,
                )?))
            }
            Opcode::New
            | Opcode::ANewArray
            | Opcode::CheckCast
            | Opcode::InstanceOf
            | Opcode::MultiANewArray => Some(InstructionReference::Class(class(
                pool,
                type_index(instruction)?,
                offset,
            )?)),
            _ => None,
        };
    Ok(reference)
}

fn loadable(pool: &ConstantPool, index: u16, offset: usize) -> Result<LoadableConstant> {
    Ok(match pool.get(index)? {
        Constant::Integer(value) => LoadableConstant::Integer(*value),
        Constant::Float(value) => LoadableConstant::Float(value.to_bits()),
        Constant::Long(value) => LoadableConstant::Long(*value),
        Constant::Double(value) => LoadableConstant::Double(value.to_bits()),
        Constant::String { string_index } => {
            LoadableConstant::String(exact(pool.utf8_constant(*string_index)?))
        }
        Constant::Class { .. } => LoadableConstant::Class(class(pool, index, offset)?),
        Constant::MethodType { descriptor_index } => {
            let descriptor = pool.utf8(*descriptor_index)?;
            require_method_descriptor(descriptor, offset)?;
            LoadableConstant::MethodType(descriptor.to_owned())
        }
        Constant::MethodHandle {
            reference_kind,
            reference_index,
        } => LoadableConstant::MethodHandle(method_handle(
            pool,
            *reference_kind,
            *reference_index,
            offset,
        )?),
        Constant::Dynamic { .. } => LoadableConstant::Dynamic(dynamic(pool, index, false, offset)?),
        constant => {
            return Err(Error::invalid_bytecode(
                offset,
                format!("{} is not an ldc-loadable constant", constant.tag_name()),
            ));
        }
    })
}

fn class(pool: &ConstantPool, index: u16, offset: usize) -> Result<ClassSymbol> {
    let Constant::Class { name_index } = pool.get(index)? else {
        return Err(Error::invalid_bytecode(
            offset,
            "type instruction does not reference a Class constant",
        ));
    };
    Ok(ClassSymbol {
        name: exact(pool.utf8_constant(*name_index)?),
    })
}

fn field(pool: &ConstantPool, index: u16, offset: usize) -> Result<FieldSymbol> {
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
    let (name, descriptor) = name_and_type(pool, *name_and_type_index)?;
    require_field_descriptor(&descriptor, offset)?;
    Ok(FieldSymbol {
        owner: class(pool, *class_index, offset)?,
        name,
        descriptor,
    })
}

fn method(
    pool: &ConstantPool,
    index: u16,
    offset: usize,
    expected: Option<MethodReferenceKind>,
) -> Result<MethodSymbol> {
    let (class_index, name_and_type_index, kind) = match pool.get(index)? {
        Constant::MethodRef {
            class_index,
            name_and_type_index,
        } => (
            *class_index,
            *name_and_type_index,
            MethodReferenceKind::Class,
        ),
        Constant::InterfaceMethodRef {
            class_index,
            name_and_type_index,
        } => (
            *class_index,
            *name_and_type_index,
            MethodReferenceKind::Interface,
        ),
        constant => {
            return Err(Error::invalid_bytecode(
                offset,
                format!("invocation references {} constant", constant.tag_name()),
            ));
        }
    };
    if expected.is_some_and(|expected| expected != kind) {
        return Err(Error::invalid_bytecode(
            offset,
            format!("invocation requires a {expected:?} method reference, found {kind:?}"),
        ));
    }
    let (name, descriptor) = name_and_type(pool, name_and_type_index)?;
    require_method_descriptor(&descriptor, offset)?;
    Ok(MethodSymbol {
        owner: class(pool, class_index, offset)?,
        name,
        descriptor,
        kind,
    })
}

fn method_handle(
    pool: &ConstantPool,
    kind: crate::classfile::MethodHandleKind,
    target: u16,
    offset: usize,
) -> Result<MethodHandleSymbol> {
    let constant = pool.get(target)?;
    let target = match kind {
        MethodHandleKind::GetField
        | MethodHandleKind::GetStatic
        | MethodHandleKind::PutField
        | MethodHandleKind::PutStatic => match constant {
            Constant::FieldRef { .. } => {
                MethodHandleTargetSymbol::Field(field(pool, target, offset)?)
            }
            _ => return Err(incompatible_handle_target(kind, constant, offset)),
        },
        MethodHandleKind::InvokeVirtual | MethodHandleKind::NewInvokeSpecial => match constant {
            Constant::MethodRef { .. } => MethodHandleTargetSymbol::Method(method(
                pool,
                target,
                offset,
                Some(MethodReferenceKind::Class),
            )?),
            _ => return Err(incompatible_handle_target(kind, constant, offset)),
        },
        MethodHandleKind::InvokeInterface => match constant {
            Constant::InterfaceMethodRef { .. } => MethodHandleTargetSymbol::Method(method(
                pool,
                target,
                offset,
                Some(MethodReferenceKind::Interface),
            )?),
            _ => return Err(incompatible_handle_target(kind, constant, offset)),
        },
        MethodHandleKind::InvokeStatic | MethodHandleKind::InvokeSpecial => match constant {
            Constant::MethodRef { .. } | Constant::InterfaceMethodRef { .. } => {
                MethodHandleTargetSymbol::Method(method(pool, target, offset, None)?)
            }
            _ => return Err(incompatible_handle_target(kind, constant, offset)),
        },
    };
    if let MethodHandleTargetSymbol::Method(method) = &target {
        let constructor = method.name.text == crate::classfile::INSTANCE_INITIALIZER_NAME;
        if (kind == MethodHandleKind::NewInvokeSpecial) != constructor
            || method.name.text == crate::classfile::CLASS_INITIALIZER_NAME
        {
            return Err(Error::invalid_bytecode(
                offset,
                format!("method handle {} has invalid target name", kind.name()),
            ));
        }
    }
    Ok(MethodHandleSymbol { kind, target })
}

fn dynamic(
    pool: &ConstantPool,
    index: u16,
    call_site: bool,
    offset: usize,
) -> Result<DynamicSymbol> {
    let (bootstrap_method, name_and_type_index) = match (pool.get(index)?, call_site) {
        (
            Constant::InvokeDynamic {
                bootstrap_method_attr_index,
                name_and_type_index,
            },
            true,
        )
        | (
            Constant::Dynamic {
                bootstrap_method_attr_index,
                name_and_type_index,
            },
            false,
        ) => (*bootstrap_method_attr_index, *name_and_type_index),
        (constant, _) => {
            return Err(Error::invalid_bytecode(
                offset,
                format!("dynamic reference selects {} constant", constant.tag_name()),
            ));
        }
    };
    let (name, descriptor) = name_and_type(pool, name_and_type_index)?;
    if call_site {
        require_method_descriptor(&descriptor, offset)?;
    } else {
        require_field_descriptor(&descriptor, offset)?;
    }
    Ok(DynamicSymbol {
        bootstrap_method,
        name,
        descriptor,
    })
}

fn name_and_type(pool: &ConstantPool, index: u16) -> Result<(ExactString, String)> {
    let Constant::NameAndType {
        name_index,
        descriptor_index,
    } = pool.get(index)?
    else {
        return Err(Error::invalid_class(
            0,
            "member reference lacks a NameAndType constant",
        ));
    };
    Ok((
        exact(pool.utf8_constant(*name_index)?),
        pool.utf8(*descriptor_index)?.to_owned(),
    ))
}

fn exact(value: &Utf8Constant) -> ExactString {
    ExactString {
        text: value.as_str().to_owned(),
        utf16_units: value.utf16_units().to_vec(),
    }
}

fn incompatible_handle_target(kind: MethodHandleKind, constant: &Constant, offset: usize) -> Error {
    Error::invalid_bytecode(
        offset,
        format!(
            "method handle {} targets incompatible {} constant",
            kind.name(),
            constant.tag_name()
        ),
    )
}

fn require_field_descriptor(value: &str, offset: usize) -> Result<()> {
    descriptor::parse_field(value).map(|_| ()).map_err(|error| {
        Error::invalid_bytecode(
            offset,
            format!("invalid referenced field descriptor: {error}"),
        )
    })
}

fn require_method_descriptor(value: &str, offset: usize) -> Result<()> {
    descriptor::parse_method(value)
        .map(|_| ())
        .map_err(|error| {
            Error::invalid_bytecode(
                offset,
                format!("invalid referenced method descriptor: {error}"),
            )
        })
}

fn constant_index(instruction: &Instruction) -> Result<u16> {
    let Operand::Constant(index) = instruction.operand else {
        return Err(Error::invalid_bytecode(
            instruction.offset,
            "constant-pool index is missing",
        ));
    };
    Ok(index)
}

fn invocation_index(instruction: &Instruction) -> Result<u16> {
    match instruction.operand {
        Operand::Constant(index) | Operand::InvokeInterface { index, .. } => Ok(index),
        _ => Err(Error::invalid_bytecode(
            instruction.offset,
            "invocation constant-pool index is missing",
        )),
    }
}

fn type_index(instruction: &Instruction) -> Result<u16> {
    match instruction.operand {
        Operand::Constant(index) | Operand::MultiArray { index, .. } => Ok(index),
        _ => Err(Error::invalid_bytecode(
            instruction.offset,
            "class constant-pool index is missing",
        )),
    }
}

//! Semantic validation for constant-pool entries and bootstrap references.

use crate::descriptor::{self, JavaType};
use crate::{Error, Result};

use super::super::{
    Attribute, ClassFile, Constant, ConstantPool, KnownAttribute, MethodHandleKind,
};
use super::{validate_internal_or_array_name, validate_unqualified_name};

pub(super) fn validate_constant_pool(class: &ClassFile) -> Result<()> {
    let pool = &class.constant_pool;
    for (index, constant) in pool.iter() {
        validate_constant_version(index, constant, class.major_version)?;
        match constant {
            Constant::Class { name_index } => {
                validate_internal_or_array_name(pool.utf8(*name_index)?, true)?;
            }
            Constant::FieldRef {
                name_and_type_index,
                ..
            }
            | Constant::Dynamic {
                name_and_type_index,
                ..
            } => validate_name_and_type(pool, *name_and_type_index, false)?,
            Constant::MethodRef {
                name_and_type_index,
                ..
            }
            | Constant::InterfaceMethodRef {
                name_and_type_index,
                ..
            }
            | Constant::InvokeDynamic {
                name_and_type_index,
                ..
            } => validate_name_and_type(pool, *name_and_type_index, true)?,
            Constant::NameAndType {
                name_index,
                descriptor_index,
            } => {
                let name = pool.utf8(*name_index)?;
                let descriptor = pool.utf8(*descriptor_index)?;
                validate_unqualified_name(name, descriptor.starts_with('('))?;
                if descriptor.starts_with('(') {
                    descriptor::parse_method(descriptor)?;
                } else {
                    descriptor::parse_field(descriptor)?;
                }
            }
            Constant::MethodHandle {
                reference_kind,
                reference_index,
            } => validate_method_handle(
                pool,
                *reference_kind,
                *reference_index,
                class.major_version,
            )?,
            Constant::MethodType { descriptor_index } => {
                descriptor::parse_method(pool.utf8(*descriptor_index)?)?;
            }
            Constant::Unusable
            | Constant::Utf8(_)
            | Constant::Integer(_)
            | Constant::Float(_)
            | Constant::Long(_)
            | Constant::Double(_)
            | Constant::String { .. }
            | Constant::Module { .. }
            | Constant::Package { .. } => {}
        }
    }
    Ok(())
}

pub(super) fn bootstrap_method_count(class: &ClassFile) -> Result<Option<usize>> {
    let mut counts = class
        .attributes
        .iter()
        .filter_map(|attribute| match attribute {
            Attribute::Known(KnownAttribute::BootstrapMethods(attribute)) => {
                Some(attribute.methods.len())
            }
            _ => None,
        });
    let count = counts.next();
    if counts.next().is_some() {
        Err(invalid("multiple BootstrapMethods attributes"))
    } else {
        Ok(count)
    }
}

pub(super) fn validate_dynamic_bootstraps(class: &ClassFile, count: Option<usize>) -> Result<()> {
    for (index, constant) in class.constant_pool.iter() {
        let bootstrap = match constant {
            Constant::Dynamic {
                bootstrap_method_attr_index,
                ..
            }
            | Constant::InvokeDynamic {
                bootstrap_method_attr_index,
                ..
            } => usize::from(*bootstrap_method_attr_index),
            _ => continue,
        };
        if count.is_none_or(|count| bootstrap >= count) {
            return Err(invalid(format!(
                "constant #{index} references missing bootstrap method {bootstrap}"
            )));
        }
    }
    Ok(())
}

pub(super) fn dynamic_has_category_two_descriptor(
    pool: &ConstantPool,
    constant: &Constant,
) -> bool {
    let Constant::Dynamic {
        name_and_type_index,
        ..
    } = constant
    else {
        return false;
    };
    pool.name_and_type(*name_and_type_index)
        .ok()
        .and_then(|(_, descriptor)| descriptor::parse_field(descriptor).ok())
        .is_some_and(|kind| matches!(kind, JavaType::Long | JavaType::Double))
}

fn validate_constant_version(index: u16, constant: &Constant, major: u16) -> Result<()> {
    let required = match constant {
        Constant::MethodHandle { .. }
        | Constant::MethodType { .. }
        | Constant::InvokeDynamic { .. } => 51,
        Constant::Module { .. } | Constant::Package { .. } => 53,
        Constant::Dynamic { .. } => 55,
        _ => return Ok(()),
    };
    if major < required {
        Err(invalid(format!(
            "constant #{index} ({}) requires class-file major {required}",
            constant.tag_name()
        )))
    } else {
        Ok(())
    }
}

fn validate_method_handle(
    pool: &ConstantPool,
    kind: MethodHandleKind,
    index: u16,
    major: u16,
) -> Result<()> {
    let constant = pool.get(index)?;
    let valid_tag = match kind {
        MethodHandleKind::GetField
        | MethodHandleKind::GetStatic
        | MethodHandleKind::PutField
        | MethodHandleKind::PutStatic => matches!(constant, Constant::FieldRef { .. }),
        MethodHandleKind::InvokeVirtual | MethodHandleKind::NewInvokeSpecial => {
            matches!(constant, Constant::MethodRef { .. })
        }
        MethodHandleKind::InvokeStatic | MethodHandleKind::InvokeSpecial => {
            matches!(constant, Constant::MethodRef { .. })
                || (major >= 52 && matches!(constant, Constant::InterfaceMethodRef { .. }))
        }
        MethodHandleKind::InvokeInterface => {
            matches!(constant, Constant::InterfaceMethodRef { .. })
        }
    };
    if !valid_tag {
        return Err(invalid(format!(
            "method handle {} references incompatible {} constant #{index}",
            kind.name(),
            constant.tag_name()
        )));
    }
    if let Constant::MethodRef {
        name_and_type_index,
        ..
    }
    | Constant::InterfaceMethodRef {
        name_and_type_index,
        ..
    } = constant
    {
        let (name, _) = pool.name_and_type(*name_and_type_index)?;
        if (kind == MethodHandleKind::NewInvokeSpecial) != (name == "<init>") || name == "<clinit>"
        {
            return Err(invalid(format!(
                "method handle {} has invalid target name `{name}`",
                kind.name()
            )));
        }
    }
    Ok(())
}

fn validate_name_and_type(pool: &ConstantPool, index: u16, method: bool) -> Result<()> {
    let (name, descriptor) = pool.name_and_type(index)?;
    validate_unqualified_name(name, method)?;
    if method {
        descriptor::parse_method(descriptor)?;
    } else {
        descriptor::parse_field(descriptor)?;
    }
    Ok(())
}

fn invalid(message: impl Into<String>) -> Error {
    Error::invalid_class(0, message)
}

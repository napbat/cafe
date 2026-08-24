//! Binary assembler for structured JVM class files.

use crate::{Error, Result};

use super::attribute::write_attributes;
use super::io::Writer;
use super::modified_utf8;
use super::{
    AttributeLocation, CLASS_MAGIC, ClassFile, Constant, ConstantPool, ConstantSlotWidth,
    FIRST_USABLE_CONSTANT_POOL_INDEX, FieldInfo, MethodInfo,
};

/// Assembles a complete structured class file into JVM binary bytes.
///
/// Unknown attributes are emitted from their exact raw payloads, so a parsed
/// class can be modified and reassembled without understanding every attribute.
///
/// # Errors
///
/// Returns an error if constant-pool references are invalid, bytecode is
/// malformed, or a count or payload exceeds its class-file encoding limit.
pub fn assemble_class(class: &ClassFile) -> Result<Vec<u8>> {
    class.validate()?;

    let mut output = Writer::new();
    output.write_u32(CLASS_MAGIC);
    output.write_u16(class.minor_version);
    output.write_u16(class.major_version);
    write_constant_pool(&mut output, &class.constant_pool)?;
    output.write_u16(class.access_flags.bits());
    output.write_u16(class.this_class);
    output.write_u16(class.super_class);
    write_indices(&mut output, &class.interfaces, "interfaces")?;

    output.write_u16(count_u16(class.fields.len(), "fields")?);
    for field in &class.fields {
        write_field(&mut output, field, &class.constant_pool)?;
    }

    output.write_u16(count_u16(class.methods.len(), "methods")?);
    for method in &class.methods {
        write_method(&mut output, method, &class.constant_pool)?;
    }
    write_attributes(
        &mut output,
        &class.attributes,
        &class.constant_pool,
        AttributeLocation::Class,
    )?;
    Ok(output.finish())
}

fn write_constant_pool(output: &mut Writer, pool: &ConstantPool) -> Result<()> {
    let entries = pool.raw_entries();
    output.write_u16(count_u16(entries.len(), "constant-pool slots")?);
    let mut index = usize::from(FIRST_USABLE_CONSTANT_POOL_INDEX);
    while index < entries.len() {
        let constant = &entries[index];
        let tag = constant.tag().ok_or_else(|| {
            Error::invalid_assembly(format!(
                "unexpected unusable constant-pool slot at index #{index}"
            ))
        })?;
        output.write_u8(tag.byte());
        write_constant(output, constant)?;

        let slot_width = constant.slot_width();
        if slot_width == ConstantSlotWidth::Double {
            let second_slot = entries
                .get(index + ConstantSlotWidth::Single.slot_count())
                .ok_or_else(|| {
                    Error::invalid_assembly(format!(
                        "constant-pool entry #{index} is missing its reserved second slot"
                    ))
                })?;
            if !matches!(second_slot, Constant::Unusable) {
                return Err(Error::invalid_assembly(format!(
                    "constant-pool entry #{index} must be followed by an unusable slot"
                )));
            }
        }
        index += slot_width.slot_count();
    }
    Ok(())
}

fn write_constant(output: &mut Writer, constant: &Constant) -> Result<()> {
    match constant {
        Constant::Unusable => {
            return Err(Error::invalid_assembly(
                "cannot write an unusable constant-pool slot directly",
            ));
        }
        Constant::Utf8(value) => {
            let encoded = modified_utf8::encode(value.utf16_units());
            output.write_u16(count_u16(encoded.len(), "modified UTF-8 bytes")?);
            output.write_bytes(&encoded);
        }
        Constant::Integer(value) => output.write_bytes(&value.to_be_bytes()),
        Constant::Float(value) => output.write_u32(value.to_bits()),
        Constant::Long(value) => output.write_bytes(&value.to_be_bytes()),
        Constant::Double(value) => output.write_u64(value.to_bits()),
        Constant::Class { name_index }
        | Constant::Module { name_index }
        | Constant::Package { name_index } => output.write_u16(*name_index),
        Constant::String { string_index } => output.write_u16(*string_index),
        Constant::FieldRef {
            class_index,
            name_and_type_index,
        }
        | Constant::MethodRef {
            class_index,
            name_and_type_index,
        }
        | Constant::InterfaceMethodRef {
            class_index,
            name_and_type_index,
        } => {
            output.write_u16(*class_index);
            output.write_u16(*name_and_type_index);
        }
        Constant::NameAndType {
            name_index,
            descriptor_index,
        } => {
            output.write_u16(*name_index);
            output.write_u16(*descriptor_index);
        }
        Constant::MethodHandle {
            reference_kind,
            reference_index,
        } => {
            output.write_u8(reference_kind.byte());
            output.write_u16(*reference_index);
        }
        Constant::MethodType { descriptor_index } => output.write_u16(*descriptor_index),
        Constant::Dynamic {
            bootstrap_method_attr_index,
            name_and_type_index,
        }
        | Constant::InvokeDynamic {
            bootstrap_method_attr_index,
            name_and_type_index,
        } => {
            output.write_u16(*bootstrap_method_attr_index);
            output.write_u16(*name_and_type_index);
        }
    }
    Ok(())
}

fn write_field(output: &mut Writer, field: &FieldInfo, pool: &ConstantPool) -> Result<()> {
    output.write_u16(field.access_flags.bits());
    output.write_u16(field.name_index);
    output.write_u16(field.descriptor_index);
    write_attributes(output, &field.attributes, pool, AttributeLocation::Field)
}

fn write_method(output: &mut Writer, method: &MethodInfo, pool: &ConstantPool) -> Result<()> {
    output.write_u16(method.access_flags.bits());
    output.write_u16(method.name_index);
    output.write_u16(method.descriptor_index);
    write_attributes(output, &method.attributes, pool, AttributeLocation::Method)
}

fn write_indices(output: &mut Writer, indices: &[u16], label: &str) -> Result<()> {
    output.write_u16(count_u16(indices.len(), label)?);
    for &index in indices {
        output.write_u16(index);
    }
    Ok(())
}

fn count_u16(value: usize, label: &str) -> Result<u16> {
    u16::try_from(value)
        .map_err(|_| Error::invalid_assembly(format!("{label} count {value} exceeds u16")))
}

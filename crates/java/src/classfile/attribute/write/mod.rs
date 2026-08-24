//! Binary assembler for standard and custom JVM attributes.

mod annotations;

use crate::bytecode;
use crate::{Error, Result};

use self::annotations::{
    write_annotation_default, write_annotations, write_parameter_annotations,
    write_type_annotations,
};
use super::super::io::Writer;
use super::super::{
    Attribute, AttributeLocation, CATCH_ALL_EXCEPTION_INDEX, CODE_ATTRIBUTE_NAME, CodeAttribute,
    Constant, ConstantPool, MAX_CODE_LENGTH, OPTIONAL_CONSTANT_POOL_INDEX, RawAttribute,
};
use super::{KnownAttribute, ModuleAttribute, StackMapFrame, StackMapFrameTag, VerificationType};

pub(crate) fn write_attributes(
    output: &mut Writer,
    attributes: &[Attribute],
    pool: &ConstantPool,
    location: AttributeLocation,
) -> Result<()> {
    output.write_u16(count_u16(attributes.len(), "attributes")?);
    for attribute in attributes {
        match attribute {
            Attribute::Code(code) => {
                if location != AttributeLocation::Method {
                    return Err(Error::invalid_assembly(
                        "Code attribute is only valid on a method",
                    ));
                }
                write_code(output, code, pool)?;
            }
            Attribute::Known(attribute) => {
                validate_known_location(attribute, location)?;
                verify_attribute_name(pool, attribute.name_index(), attribute.name())?;
                let mut info = Writer::new();
                write_known_payload(&mut info, attribute, pool)?;
                output.write_u16(attribute.name_index());
                output.write_u32(count_u32(info.len(), "attribute bytes")?);
                output.write_bytes(&info.finish());
            }
            Attribute::Raw(attribute) => write_raw(output, attribute, pool)?,
        }
    }
    Ok(())
}

pub(crate) fn validate_known_model(attribute: &KnownAttribute, pool: &ConstantPool) -> Result<()> {
    verify_attribute_name(pool, attribute.name_index(), attribute.name())?;
    write_known_payload(&mut Writer::new(), attribute, pool)
}

fn write_code(output: &mut Writer, code: &CodeAttribute, pool: &ConstantPool) -> Result<()> {
    verify_attribute_name(pool, code.name_index, CODE_ATTRIBUTE_NAME)?;
    if code.code.len() > MAX_CODE_LENGTH {
        return Err(Error::invalid_assembly(format!(
            "method code exceeds {MAX_CODE_LENGTH} bytes: {}",
            code.code.len()
        )));
    }
    bytecode::decode_code(code)?;
    let mut info = Writer::new();
    info.write_u16(code.max_stack);
    info.write_u16(code.max_locals);
    info.write_u32(count_u32(code.code.len(), "method code bytes")?);
    info.write_bytes(&code.code);
    info.write_u16(count_u16(code.exception_table.len(), "exception handlers")?);
    for handler in &code.exception_table {
        if handler.catch_type != CATCH_ALL_EXCEPTION_INDEX {
            expect_class(pool, handler.catch_type)?;
        }
        info.write_u16(handler.start_pc);
        info.write_u16(handler.end_pc);
        info.write_u16(handler.handler_pc);
        info.write_u16(handler.catch_type);
    }
    write_attributes(&mut info, &code.attributes, pool, AttributeLocation::Code)?;
    output.write_u16(code.name_index);
    output.write_u32(count_u32(info.len(), "Code attribute bytes")?);
    output.write_bytes(&info.finish());
    Ok(())
}

#[allow(clippy::too_many_lines)]
fn write_known_payload(
    output: &mut Writer,
    attribute: &KnownAttribute,
    pool: &ConstantPool,
) -> Result<()> {
    match attribute {
        KnownAttribute::ConstantValue(attribute) => {
            expect_tag(pool, attribute.index, "constant value", |constant| {
                matches!(
                    constant,
                    Constant::Integer(_)
                        | Constant::Float(_)
                        | Constant::Long(_)
                        | Constant::Double(_)
                        | Constant::String { .. }
                )
            })?;
            output.write_u16(attribute.index);
        }
        KnownAttribute::StackMapTable(attribute) => {
            output.write_u16(count_u16(attribute.frames.len(), "stack-map frames")?);
            for frame in &attribute.frames {
                write_stack_map_frame(output, frame, pool)?;
            }
        }
        KnownAttribute::Exceptions(attribute)
        | KnownAttribute::NestMembers(attribute)
        | KnownAttribute::PermittedSubclasses(attribute) => {
            expect_each_class(pool, &attribute.indices)?;
            write_u16_list(output, &attribute.indices, "class indices")?;
        }
        KnownAttribute::InnerClasses(attribute) => {
            output.write_u16(count_u16(attribute.classes.len(), "inner classes")?);
            for class in &attribute.classes {
                expect_class(pool, class.inner_class_info_index)?;
                expect_optional(pool, class.outer_class_info_index, "Class", |constant| {
                    matches!(constant, Constant::Class { .. })
                })?;
                expect_optional(pool, class.inner_name_index, "Utf8", |constant| {
                    matches!(constant, Constant::Utf8(_))
                })?;
                output.write_u16(class.inner_class_info_index);
                output.write_u16(class.outer_class_info_index);
                output.write_u16(class.inner_name_index);
                output.write_u16(class.access_flags.bits());
            }
        }
        KnownAttribute::EnclosingMethod(attribute) => {
            expect_class(pool, attribute.class_index)?;
            expect_optional(pool, attribute.method_index, "NameAndType", |constant| {
                matches!(constant, Constant::NameAndType { .. })
            })?;
            output.write_u16(attribute.class_index);
            output.write_u16(attribute.method_index);
        }
        KnownAttribute::Synthetic(_) | KnownAttribute::Deprecated(_) => {}
        KnownAttribute::Signature(attribute) | KnownAttribute::SourceFile(attribute) => {
            expect_utf8(pool, attribute.index)?;
            output.write_u16(attribute.index);
        }
        KnownAttribute::SourceDebugExtension(attribute) => output.write_bytes(&attribute.bytes),
        KnownAttribute::LineNumberTable(attribute) => {
            output.write_u16(count_u16(attribute.lines.len(), "line numbers")?);
            for line in &attribute.lines {
                output.write_u16(line.start_pc);
                output.write_u16(line.line_number);
            }
        }
        KnownAttribute::LocalVariableTable(attribute) => {
            output.write_u16(count_u16(attribute.variables.len(), "local variables")?);
            for variable in &attribute.variables {
                expect_utf8(pool, variable.name_index)?;
                expect_utf8(pool, variable.descriptor_index)?;
                output.write_u16(variable.start_pc);
                output.write_u16(variable.length);
                output.write_u16(variable.name_index);
                output.write_u16(variable.descriptor_index);
                output.write_u16(variable.slot);
            }
        }
        KnownAttribute::LocalVariableTypeTable(attribute) => {
            output.write_u16(count_u16(
                attribute.variables.len(),
                "generic local variables",
            )?);
            for variable in &attribute.variables {
                expect_utf8(pool, variable.name_index)?;
                expect_utf8(pool, variable.signature_index)?;
                output.write_u16(variable.start_pc);
                output.write_u16(variable.length);
                output.write_u16(variable.name_index);
                output.write_u16(variable.signature_index);
                output.write_u16(variable.slot);
            }
        }
        KnownAttribute::RuntimeVisibleAnnotations(attribute)
        | KnownAttribute::RuntimeInvisibleAnnotations(attribute) => {
            write_annotations(output, attribute, pool)?;
        }
        KnownAttribute::RuntimeVisibleParameterAnnotations(attribute)
        | KnownAttribute::RuntimeInvisibleParameterAnnotations(attribute) => {
            write_parameter_annotations(output, attribute, pool)?;
        }
        KnownAttribute::RuntimeVisibleTypeAnnotations(attribute)
        | KnownAttribute::RuntimeInvisibleTypeAnnotations(attribute) => {
            write_type_annotations(output, attribute, pool)?;
        }
        KnownAttribute::AnnotationDefault(attribute) => {
            write_annotation_default(output, attribute, pool)?;
        }
        KnownAttribute::BootstrapMethods(attribute) => {
            output.write_u16(count_u16(attribute.methods.len(), "bootstrap methods")?);
            for method in &attribute.methods {
                expect_tag(pool, method.method_ref, "MethodHandle", |constant| {
                    matches!(constant, Constant::MethodHandle { .. })
                })?;
                output.write_u16(method.method_ref);
                write_u16_list(output, &method.arguments, "bootstrap arguments")?;
                for &argument in &method.arguments {
                    expect_loadable_constant(pool, argument)?;
                }
            }
        }
        KnownAttribute::MethodParameters(attribute) => {
            output.write_u8(count_u8(attribute.parameters.len(), "method parameters")?);
            for parameter in &attribute.parameters {
                expect_optional(pool, parameter.name_index, "Utf8", |constant| {
                    matches!(constant, Constant::Utf8(_))
                })?;
                output.write_u16(parameter.name_index);
                output.write_u16(parameter.access_flags.bits());
            }
        }
        KnownAttribute::Module(attribute) => write_module(output, attribute, pool)?,
        KnownAttribute::ModulePackages(attribute) => {
            expect_each(pool, &attribute.indices, "Package", |constant| {
                matches!(constant, Constant::Package { .. })
            })?;
            write_u16_list(output, &attribute.indices, "module packages")?;
        }
        KnownAttribute::ModuleMainClass(attribute) | KnownAttribute::NestHost(attribute) => {
            expect_class(pool, attribute.index)?;
            output.write_u16(attribute.index);
        }
        KnownAttribute::Record(attribute) => {
            output.write_u16(count_u16(attribute.components.len(), "record components")?);
            for component in &attribute.components {
                expect_utf8(pool, component.name_index)?;
                expect_utf8(pool, component.descriptor_index)?;
                output.write_u16(component.name_index);
                output.write_u16(component.descriptor_index);
                write_attributes(
                    output,
                    &component.attributes,
                    pool,
                    AttributeLocation::RecordComponent,
                )?;
            }
        }
    }
    Ok(())
}

fn write_stack_map_frame(
    output: &mut Writer,
    frame: &StackMapFrame,
    pool: &ConstantPool,
) -> Result<()> {
    match frame {
        StackMapFrame::Same { offset_delta } => {
            let tag = StackMapFrameTag::same(*offset_delta).ok_or_else(|| {
                Error::invalid_assembly(
                    "same stack-map frame offset is outside its compact encoded range",
                )
            })?;
            output.write_u8(tag.byte());
        }
        StackMapFrame::SameLocalsOneStack {
            offset_delta,
            stack,
        } => {
            let tag = StackMapFrameTag::same_locals_one_stack(*offset_delta).ok_or_else(|| {
                Error::invalid_assembly(
                    "same-locals stack-map frame offset is outside its compact encoded range",
                )
            })?;
            output.write_u8(tag.byte());
            write_verification_type(output, *stack, pool)?;
        }
        StackMapFrame::SameLocalsOneStackExtended {
            offset_delta,
            stack,
        } => {
            output.write_u8(StackMapFrameTag::SameLocalsOneStackExtended.byte());
            output.write_u16(*offset_delta);
            write_verification_type(output, *stack, pool)?;
        }
        StackMapFrame::Chop {
            offset_delta,
            absent_locals,
        } => {
            let tag = StackMapFrameTag::chop(*absent_locals).ok_or_else(|| {
                Error::invalid_assembly(
                    "chop stack-map frame local count is outside its encoded range",
                )
            })?;
            output.write_u8(tag.byte());
            output.write_u16(*offset_delta);
        }
        StackMapFrame::SameExtended { offset_delta } => {
            output.write_u8(StackMapFrameTag::SameExtended.byte());
            output.write_u16(*offset_delta);
        }
        StackMapFrame::Append {
            offset_delta,
            locals,
        } => {
            let tag = StackMapFrameTag::append(locals.len()).ok_or_else(|| {
                Error::invalid_assembly(
                    "append stack-map frame local count is outside its encoded range",
                )
            })?;
            output.write_u8(tag.byte());
            output.write_u16(*offset_delta);
            for &local in locals {
                write_verification_type(output, local, pool)?;
            }
        }
        StackMapFrame::Full {
            offset_delta,
            locals,
            stack,
        } => {
            output.write_u8(StackMapFrameTag::Full.byte());
            output.write_u16(*offset_delta);
            write_verification_types(output, locals, pool, "full-frame locals")?;
            write_verification_types(output, stack, pool, "full-frame stack")?;
        }
    }
    Ok(())
}

fn write_verification_types(
    output: &mut Writer,
    values: &[VerificationType],
    pool: &ConstantPool,
    label: &str,
) -> Result<()> {
    output.write_u16(count_u16(values.len(), label)?);
    for &value in values {
        write_verification_type(output, value, pool)?;
    }
    Ok(())
}

fn write_verification_type(
    output: &mut Writer,
    value: VerificationType,
    pool: &ConstantPool,
) -> Result<()> {
    if let VerificationType::Object(index) = value {
        expect_class(pool, index)?;
    }
    output.write_u8(value.kind().tag());
    match value {
        VerificationType::Object(index) | VerificationType::Uninitialized(index) => {
            output.write_u16(index);
        }
        VerificationType::Top
        | VerificationType::Integer
        | VerificationType::Float
        | VerificationType::Double
        | VerificationType::Long
        | VerificationType::Null
        | VerificationType::UninitializedThis => {}
    }
    Ok(())
}

fn write_module(output: &mut Writer, module: &ModuleAttribute, pool: &ConstantPool) -> Result<()> {
    expect_module(pool, module.module_name_index)?;
    expect_optional(pool, module.module_version_index, "Utf8", |constant| {
        matches!(constant, Constant::Utf8(_))
    })?;
    output.write_u16(module.module_name_index);
    output.write_u16(module.module_flags.bits());
    output.write_u16(module.module_version_index);
    output.write_u16(count_u16(module.requires.len(), "module requires")?);
    for requirement in &module.requires {
        expect_module(pool, requirement.module_index)?;
        expect_optional(pool, requirement.version_index, "Utf8", |constant| {
            matches!(constant, Constant::Utf8(_))
        })?;
        output.write_u16(requirement.module_index);
        output.write_u16(requirement.flags.bits());
        output.write_u16(requirement.version_index);
    }
    output.write_u16(count_u16(module.exports.len(), "module exports")?);
    for export in &module.exports {
        expect_package(pool, export.package_index)?;
        output.write_u16(export.package_index);
        output.write_u16(export.flags.bits());
        write_u16_list(output, &export.to_modules, "qualified export modules")?;
        expect_each_module(pool, &export.to_modules)?;
    }
    output.write_u16(count_u16(module.opens.len(), "module opens")?);
    for open in &module.opens {
        expect_package(pool, open.package_index)?;
        output.write_u16(open.package_index);
        output.write_u16(open.flags.bits());
        write_u16_list(output, &open.to_modules, "qualified open modules")?;
        expect_each_module(pool, &open.to_modules)?;
    }
    write_u16_list(output, &module.uses, "module uses")?;
    expect_each_class(pool, &module.uses)?;
    output.write_u16(count_u16(module.provides.len(), "module provides")?);
    for provide in &module.provides {
        if provide.implementation_indices.is_empty() {
            return Err(Error::invalid_assembly(
                "module provides directive has no implementation",
            ));
        }
        expect_class(pool, provide.service_index)?;
        expect_each_class(pool, &provide.implementation_indices)?;
        output.write_u16(provide.service_index);
        write_u16_list(
            output,
            &provide.implementation_indices,
            "module provider implementations",
        )?;
    }
    Ok(())
}

fn write_raw(output: &mut Writer, attribute: &RawAttribute, pool: &ConstantPool) -> Result<()> {
    verify_attribute_name(pool, attribute.name_index, &attribute.name)?;
    output.write_u16(attribute.name_index);
    output.write_u32(count_u32(attribute.info.len(), "attribute bytes")?);
    output.write_bytes(&attribute.info);
    Ok(())
}

fn verify_attribute_name(pool: &ConstantPool, index: u16, expected: &str) -> Result<()> {
    let actual = pool.utf8(index)?;
    if actual == expected {
        Ok(())
    } else {
        Err(Error::invalid_assembly(format!(
            "attribute name index #{index} resolves to `{actual}`, expected `{expected}`"
        )))
    }
}

fn validate_known_location(attribute: &KnownAttribute, location: AttributeLocation) -> Result<()> {
    if attribute.is_valid_at(location) {
        Ok(())
    } else {
        Err(Error::invalid_assembly(format!(
            "{} attribute is not valid at {location:?} location",
            attribute.name()
        )))
    }
}

fn write_u16_list(output: &mut Writer, values: &[u16], label: &str) -> Result<()> {
    output.write_u16(count_u16(values.len(), label)?);
    for &value in values {
        output.write_u16(value);
    }
    Ok(())
}

fn count_u8(count: usize, label: &str) -> Result<u8> {
    u8::try_from(count)
        .map_err(|_| Error::invalid_assembly(format!("{label} count {count} exceeds u8")))
}

fn count_u16(count: usize, label: &str) -> Result<u16> {
    u16::try_from(count)
        .map_err(|_| Error::invalid_assembly(format!("{label} count {count} exceeds u16")))
}

fn count_u32(count: usize, label: &str) -> Result<u32> {
    u32::try_from(count)
        .map_err(|_| Error::invalid_assembly(format!("{label} count {count} exceeds u32")))
}

fn expect_loadable_constant(pool: &ConstantPool, index: u16) -> Result<()> {
    expect_tag(pool, index, "loadable constant", |constant| {
        matches!(
            constant,
            Constant::Integer(_)
                | Constant::Float(_)
                | Constant::Long(_)
                | Constant::Double(_)
                | Constant::String { .. }
                | Constant::Class { .. }
                | Constant::MethodHandle { .. }
                | Constant::MethodType { .. }
                | Constant::Dynamic { .. }
        )
    })
}

fn expect_utf8(pool: &ConstantPool, index: u16) -> Result<()> {
    expect_tag(pool, index, "Utf8", |constant| {
        matches!(constant, Constant::Utf8(_))
    })
}

fn expect_class(pool: &ConstantPool, index: u16) -> Result<()> {
    expect_tag(pool, index, "Class", |constant| {
        matches!(constant, Constant::Class { .. })
    })
}

fn expect_module(pool: &ConstantPool, index: u16) -> Result<()> {
    expect_tag(pool, index, "Module", |constant| {
        matches!(constant, Constant::Module { .. })
    })
}

fn expect_package(pool: &ConstantPool, index: u16) -> Result<()> {
    expect_tag(pool, index, "Package", |constant| {
        matches!(constant, Constant::Package { .. })
    })
}

fn expect_each_class(pool: &ConstantPool, indices: &[u16]) -> Result<()> {
    expect_each(pool, indices, "Class", |constant| {
        matches!(constant, Constant::Class { .. })
    })
}

fn expect_each_module(pool: &ConstantPool, indices: &[u16]) -> Result<()> {
    expect_each(pool, indices, "Module", |constant| {
        matches!(constant, Constant::Module { .. })
    })
}

fn expect_each(
    pool: &ConstantPool,
    indices: &[u16],
    expected: &str,
    predicate: impl Fn(&Constant) -> bool + Copy,
) -> Result<()> {
    for &index in indices {
        expect_tag(pool, index, expected, predicate)?;
    }
    Ok(())
}

fn expect_optional(
    pool: &ConstantPool,
    index: u16,
    expected: &str,
    predicate: impl Fn(&Constant) -> bool,
) -> Result<()> {
    if index == OPTIONAL_CONSTANT_POOL_INDEX {
        Ok(())
    } else {
        expect_tag(pool, index, expected, predicate)
    }
}

fn expect_tag(
    pool: &ConstantPool,
    index: u16,
    expected: &str,
    predicate: impl Fn(&Constant) -> bool,
) -> Result<()> {
    let constant = pool.get(index)?;
    if predicate(constant) {
        Ok(())
    } else {
        Err(Error::invalid_assembly(format!(
            "constant-pool index #{index} is {}, expected {expected}",
            constant.tag_name()
        )))
    }
}

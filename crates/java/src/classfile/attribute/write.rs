//! Binary assembler for standard and custom JVM attributes.

use crate::bytecode;
use crate::{Error, Result};

use super::super::io::Writer;
use super::super::{
    Attribute, AttributeLocation, CATCH_ALL_EXCEPTION_INDEX, CodeAttribute, Constant, ConstantPool,
    MAX_CODE_LENGTH, RawAttribute,
};
use super::{
    Annotation, AnnotationConstantKind, ElementValue, KnownAttribute, LocalVariableTarget,
    ModuleAttribute, StackMapFrame, TypeAnnotation, TypeAnnotationTarget, TypePathEntry,
    VerificationType,
};

const MAX_ANNOTATION_DEPTH: usize = 128;

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
    verify_attribute_name(pool, code.name_index, "Code")?;
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
            write_annotations(output, &attribute.annotations, pool, 0)?;
        }
        KnownAttribute::RuntimeVisibleParameterAnnotations(attribute)
        | KnownAttribute::RuntimeInvisibleParameterAnnotations(attribute) => {
            output.write_u8(count_u8(
                attribute.parameters.len(),
                "annotated parameters",
            )?);
            for annotations in &attribute.parameters {
                write_annotations(output, annotations, pool, 0)?;
            }
        }
        KnownAttribute::RuntimeVisibleTypeAnnotations(attribute)
        | KnownAttribute::RuntimeInvisibleTypeAnnotations(attribute) => {
            output.write_u16(count_u16(attribute.annotations.len(), "type annotations")?);
            for annotation in &attribute.annotations {
                write_type_annotation(output, annotation, pool)?;
            }
        }
        KnownAttribute::AnnotationDefault(attribute) => {
            write_element_value(output, &attribute.value, pool, 0)?;
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
        StackMapFrame::Same { offset_delta } if *offset_delta <= 63 => {
            output.write_u8(*offset_delta);
        }
        StackMapFrame::SameLocalsOneStack {
            offset_delta,
            stack,
        } if *offset_delta <= 63 => {
            output.write_u8(64 + *offset_delta);
            write_verification_type(output, *stack, pool)?;
        }
        StackMapFrame::SameLocalsOneStackExtended {
            offset_delta,
            stack,
        } => {
            output.write_u8(247);
            output.write_u16(*offset_delta);
            write_verification_type(output, *stack, pool)?;
        }
        StackMapFrame::Chop {
            offset_delta,
            absent_locals: absent @ 1..=3,
        } => {
            output.write_u8(251 - *absent);
            output.write_u16(*offset_delta);
        }
        StackMapFrame::SameExtended { offset_delta } => {
            output.write_u8(251);
            output.write_u16(*offset_delta);
        }
        StackMapFrame::Append {
            offset_delta,
            locals,
        } if (1..=3).contains(&locals.len()) => {
            output.write_u8(251 + count_u8(locals.len(), "appended locals")?);
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
            output.write_u8(255);
            output.write_u16(*offset_delta);
            write_verification_types(output, locals, pool, "full-frame locals")?;
            write_verification_types(output, stack, pool, "full-frame stack")?;
        }
        StackMapFrame::Same { .. }
        | StackMapFrame::SameLocalsOneStack { .. }
        | StackMapFrame::Chop { .. }
        | StackMapFrame::Append { .. } => {
            return Err(Error::invalid_assembly(
                "stack-map compact frame fields are outside their encoded range",
            ));
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
    match value {
        VerificationType::Top => output.write_u8(0),
        VerificationType::Integer => output.write_u8(1),
        VerificationType::Float => output.write_u8(2),
        VerificationType::Double => output.write_u8(3),
        VerificationType::Long => output.write_u8(4),
        VerificationType::Null => output.write_u8(5),
        VerificationType::UninitializedThis => output.write_u8(6),
        VerificationType::Object(index) => {
            expect_class(pool, index)?;
            output.write_u8(7);
            output.write_u16(index);
        }
        VerificationType::Uninitialized(offset) => {
            output.write_u8(8);
            output.write_u16(offset);
        }
    }
    Ok(())
}

fn write_annotations(
    output: &mut Writer,
    annotations: &[Annotation],
    pool: &ConstantPool,
    depth: usize,
) -> Result<()> {
    output.write_u16(count_u16(annotations.len(), "annotations")?);
    for annotation in annotations {
        write_annotation(output, annotation, pool, depth)?;
    }
    Ok(())
}

fn write_annotation(
    output: &mut Writer,
    annotation: &Annotation,
    pool: &ConstantPool,
    depth: usize,
) -> Result<()> {
    ensure_annotation_depth(depth)?;
    expect_utf8(pool, annotation.type_index)?;
    output.write_u16(annotation.type_index);
    output.write_u16(count_u16(annotation.elements.len(), "annotation elements")?);
    for element in &annotation.elements {
        expect_utf8(pool, element.name_index)?;
        output.write_u16(element.name_index);
        write_element_value(output, &element.value, pool, depth + 1)?;
    }
    Ok(())
}

fn write_element_value(
    output: &mut Writer,
    value: &ElementValue,
    pool: &ConstantPool,
    depth: usize,
) -> Result<()> {
    ensure_annotation_depth(depth)?;
    match value {
        ElementValue::Constant {
            kind,
            constant_index,
        } => {
            expect_annotation_constant(pool, *constant_index, *kind)?;
            output.write_u8(kind.tag());
            output.write_u16(*constant_index);
        }
        ElementValue::Enum {
            type_name_index,
            constant_name_index,
        } => {
            expect_utf8(pool, *type_name_index)?;
            expect_utf8(pool, *constant_name_index)?;
            output.write_u8(b'e');
            output.write_u16(*type_name_index);
            output.write_u16(*constant_name_index);
        }
        ElementValue::Class(index) => {
            expect_utf8(pool, *index)?;
            output.write_u8(b'c');
            output.write_u16(*index);
        }
        ElementValue::Annotation(annotation) => {
            output.write_u8(b'@');
            write_annotation(output, annotation, pool, depth + 1)?;
        }
        ElementValue::Array(values) => {
            output.write_u8(b'[');
            output.write_u16(count_u16(values.len(), "annotation array values")?);
            for value in values {
                write_element_value(output, value, pool, depth + 1)?;
            }
        }
    }
    Ok(())
}

fn write_type_annotation(
    output: &mut Writer,
    annotation: &TypeAnnotation,
    pool: &ConstantPool,
) -> Result<()> {
    write_type_target(output, &annotation.target)?;
    output.write_u8(count_u8(annotation.path.len(), "type path")?);
    for entry in &annotation.path {
        match entry {
            TypePathEntry::Array => {
                output.write_u8(0);
                output.write_u8(0);
            }
            TypePathEntry::Nested => {
                output.write_u8(1);
                output.write_u8(0);
            }
            TypePathEntry::WildcardBound => {
                output.write_u8(2);
                output.write_u8(0);
            }
            TypePathEntry::TypeArgument(index) => {
                output.write_u8(3);
                output.write_u8(*index);
            }
        }
    }
    write_annotation(output, &annotation.annotation, pool, 0)
}

#[allow(clippy::too_many_lines)]
fn write_type_target(output: &mut Writer, target: &TypeAnnotationTarget) -> Result<()> {
    match target {
        TypeAnnotationTarget::ClassTypeParameter(index) => write_u8_target(output, 0x00, *index),
        TypeAnnotationTarget::MethodTypeParameter(index) => write_u8_target(output, 0x01, *index),
        TypeAnnotationTarget::ClassExtends(index) => write_u16_target(output, 0x10, *index),
        TypeAnnotationTarget::ClassTypeParameterBound {
            parameter_index,
            bound_index,
        } => write_bound_target(output, 0x11, *parameter_index, *bound_index),
        TypeAnnotationTarget::MethodTypeParameterBound {
            parameter_index,
            bound_index,
        } => write_bound_target(output, 0x12, *parameter_index, *bound_index),
        TypeAnnotationTarget::Field => output.write_u8(0x13),
        TypeAnnotationTarget::MethodReturn => output.write_u8(0x14),
        TypeAnnotationTarget::MethodReceiver => output.write_u8(0x15),
        TypeAnnotationTarget::MethodFormalParameter(index) => {
            write_u8_target(output, 0x16, *index);
        }
        TypeAnnotationTarget::Throws(index) => write_u16_target(output, 0x17, *index),
        TypeAnnotationTarget::LocalVariable(targets) => {
            write_local_variable_targets(output, 0x40, targets)?;
        }
        TypeAnnotationTarget::ResourceVariable(targets) => {
            write_local_variable_targets(output, 0x41, targets)?;
        }
        TypeAnnotationTarget::ExceptionParameter(index) => write_u16_target(output, 0x42, *index),
        TypeAnnotationTarget::InstanceOf(offset) => write_u16_target(output, 0x43, *offset),
        TypeAnnotationTarget::New(offset) => write_u16_target(output, 0x44, *offset),
        TypeAnnotationTarget::ConstructorReference(offset) => {
            write_u16_target(output, 0x45, *offset);
        }
        TypeAnnotationTarget::MethodReference(offset) => write_u16_target(output, 0x46, *offset),
        TypeAnnotationTarget::Cast {
            offset,
            type_argument_index,
        } => write_type_argument_target(output, 0x47, *offset, *type_argument_index),
        TypeAnnotationTarget::ConstructorInvocationTypeArgument {
            offset,
            type_argument_index,
        } => write_type_argument_target(output, 0x48, *offset, *type_argument_index),
        TypeAnnotationTarget::MethodInvocationTypeArgument {
            offset,
            type_argument_index,
        } => write_type_argument_target(output, 0x49, *offset, *type_argument_index),
        TypeAnnotationTarget::ConstructorReferenceTypeArgument {
            offset,
            type_argument_index,
        } => write_type_argument_target(output, 0x4a, *offset, *type_argument_index),
        TypeAnnotationTarget::MethodReferenceTypeArgument {
            offset,
            type_argument_index,
        } => write_type_argument_target(output, 0x4b, *offset, *type_argument_index),
    }
    Ok(())
}

fn write_u8_target(output: &mut Writer, tag: u8, index: u8) {
    output.write_u8(tag);
    output.write_u8(index);
}

fn write_u16_target(output: &mut Writer, tag: u8, index: u16) {
    output.write_u8(tag);
    output.write_u16(index);
}

fn write_bound_target(output: &mut Writer, tag: u8, parameter: u8, bound: u8) {
    output.write_u8(tag);
    output.write_u8(parameter);
    output.write_u8(bound);
}

fn write_type_argument_target(output: &mut Writer, tag: u8, offset: u16, argument: u8) {
    output.write_u8(tag);
    output.write_u16(offset);
    output.write_u8(argument);
}

fn write_local_variable_targets(
    output: &mut Writer,
    tag: u8,
    targets: &[LocalVariableTarget],
) -> Result<()> {
    output.write_u8(tag);
    output.write_u16(count_u16(targets.len(), "local-variable targets")?);
    for target in targets {
        output.write_u16(target.start_pc);
        output.write_u16(target.length);
        output.write_u16(target.index);
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

fn ensure_annotation_depth(depth: usize) -> Result<()> {
    if depth > MAX_ANNOTATION_DEPTH {
        Err(Error::invalid_assembly(
            "annotation nesting exceeds the supported safety limit",
        ))
    } else {
        Ok(())
    }
}

fn expect_annotation_constant(
    pool: &ConstantPool,
    index: u16,
    kind: AnnotationConstantKind,
) -> Result<()> {
    expect_tag(pool, index, "annotation constant", |constant| match kind {
        AnnotationConstantKind::Byte
        | AnnotationConstantKind::Char
        | AnnotationConstantKind::Int
        | AnnotationConstantKind::Short
        | AnnotationConstantKind::Boolean => matches!(constant, Constant::Integer(_)),
        AnnotationConstantKind::Double => matches!(constant, Constant::Double(_)),
        AnnotationConstantKind::Float => matches!(constant, Constant::Float(_)),
        AnnotationConstantKind::Long => matches!(constant, Constant::Long(_)),
        AnnotationConstantKind::String => matches!(constant, Constant::Utf8(_)),
    })
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
    if index == 0 {
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

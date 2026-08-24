//! Assembler for JVM annotation attributes and their nested values.

use crate::{Error, Result};

use super::super::super::io::Writer;
use super::super::super::{Constant, ConstantPool};
use super::super::{
    Annotation, AnnotationConstantKind, AnnotationDefaultAttribute, AnnotationsAttribute,
    ElementValue, LocalVariableTarget, ParameterAnnotationsAttribute, TypeAnnotation,
    TypeAnnotationTarget, TypeAnnotationsAttribute,
};
use super::{count_u8, count_u16, expect_tag, expect_utf8};

const MAX_ANNOTATION_DEPTH: usize = 128;
const ROOT_ANNOTATION_DEPTH: usize = 0;
const ANNOTATION_NESTING_INCREMENT: usize = 1;

pub(super) fn write_annotations(
    output: &mut Writer,
    attribute: &AnnotationsAttribute,
    pool: &ConstantPool,
) -> Result<()> {
    write_annotation_list(output, &attribute.annotations, pool, ROOT_ANNOTATION_DEPTH)
}

pub(super) fn write_parameter_annotations(
    output: &mut Writer,
    attribute: &ParameterAnnotationsAttribute,
    pool: &ConstantPool,
) -> Result<()> {
    output.write_u8(count_u8(
        attribute.parameters.len(),
        "annotated parameters",
    )?);
    for annotations in &attribute.parameters {
        write_annotation_list(output, annotations, pool, ROOT_ANNOTATION_DEPTH)?;
    }
    Ok(())
}

pub(super) fn write_type_annotations(
    output: &mut Writer,
    attribute: &TypeAnnotationsAttribute,
    pool: &ConstantPool,
) -> Result<()> {
    output.write_u16(count_u16(attribute.annotations.len(), "type annotations")?);
    for annotation in &attribute.annotations {
        write_type_annotation(output, annotation, pool)?;
    }
    Ok(())
}

pub(super) fn write_annotation_default(
    output: &mut Writer,
    attribute: &AnnotationDefaultAttribute,
    pool: &ConstantPool,
) -> Result<()> {
    write_element_value(output, &attribute.value, pool, ROOT_ANNOTATION_DEPTH)
}

fn write_annotation_list(
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
        write_element_value(
            output,
            &element.value,
            pool,
            depth + ANNOTATION_NESTING_INCREMENT,
        )?;
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
    let tag = value.kind().tag();
    match value {
        ElementValue::Constant {
            kind,
            constant_index,
        } => {
            expect_annotation_constant(pool, *constant_index, *kind)?;
            output.write_u8(tag);
            output.write_u16(*constant_index);
        }
        ElementValue::Enum {
            type_name_index,
            constant_name_index,
        } => {
            expect_utf8(pool, *type_name_index)?;
            expect_utf8(pool, *constant_name_index)?;
            output.write_u8(tag);
            output.write_u16(*type_name_index);
            output.write_u16(*constant_name_index);
        }
        ElementValue::Class(index) => {
            expect_utf8(pool, *index)?;
            output.write_u8(tag);
            output.write_u16(*index);
        }
        ElementValue::Annotation(annotation) => {
            output.write_u8(tag);
            write_annotation(
                output,
                annotation,
                pool,
                depth + ANNOTATION_NESTING_INCREMENT,
            )?;
        }
        ElementValue::Array(values) => {
            output.write_u8(tag);
            output.write_u16(count_u16(values.len(), "annotation array values")?);
            for value in values {
                write_element_value(output, value, pool, depth + ANNOTATION_NESTING_INCREMENT)?;
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
        let (kind, argument) = entry.encoded();
        output.write_u8(kind.tag());
        output.write_u8(argument);
    }
    write_annotation(output, &annotation.annotation, pool, ROOT_ANNOTATION_DEPTH)
}

#[allow(clippy::too_many_lines)]
fn write_type_target(output: &mut Writer, target: &TypeAnnotationTarget) -> Result<()> {
    output.write_u8(target.kind().tag());
    match target {
        TypeAnnotationTarget::ClassTypeParameter(index)
        | TypeAnnotationTarget::MethodTypeParameter(index)
        | TypeAnnotationTarget::MethodFormalParameter(index) => output.write_u8(*index),
        TypeAnnotationTarget::ClassExtends(index)
        | TypeAnnotationTarget::Throws(index)
        | TypeAnnotationTarget::ExceptionParameter(index)
        | TypeAnnotationTarget::InstanceOf(index)
        | TypeAnnotationTarget::New(index)
        | TypeAnnotationTarget::ConstructorReference(index)
        | TypeAnnotationTarget::MethodReference(index) => output.write_u16(*index),
        TypeAnnotationTarget::ClassTypeParameterBound {
            parameter_index,
            bound_index,
        }
        | TypeAnnotationTarget::MethodTypeParameterBound {
            parameter_index,
            bound_index,
        } => {
            output.write_u8(*parameter_index);
            output.write_u8(*bound_index);
        }
        TypeAnnotationTarget::Field
        | TypeAnnotationTarget::MethodReturn
        | TypeAnnotationTarget::MethodReceiver => {}
        TypeAnnotationTarget::LocalVariable(targets)
        | TypeAnnotationTarget::ResourceVariable(targets) => {
            write_local_variable_targets(output, targets)?;
        }
        TypeAnnotationTarget::Cast {
            offset,
            type_argument_index,
        }
        | TypeAnnotationTarget::ConstructorInvocationTypeArgument {
            offset,
            type_argument_index,
        }
        | TypeAnnotationTarget::MethodInvocationTypeArgument {
            offset,
            type_argument_index,
        }
        | TypeAnnotationTarget::ConstructorReferenceTypeArgument {
            offset,
            type_argument_index,
        }
        | TypeAnnotationTarget::MethodReferenceTypeArgument {
            offset,
            type_argument_index,
        } => {
            output.write_u16(*offset);
            output.write_u8(*type_argument_index);
        }
    }
    Ok(())
}

fn write_local_variable_targets(
    output: &mut Writer,
    targets: &[LocalVariableTarget],
) -> Result<()> {
    output.write_u16(count_u16(targets.len(), "local-variable targets")?);
    for target in targets {
        output.write_u16(target.start_pc);
        output.write_u16(target.length);
        output.write_u16(target.index);
    }
    Ok(())
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

fn ensure_annotation_depth(depth: usize) -> Result<()> {
    if depth > MAX_ANNOTATION_DEPTH {
        Err(Error::invalid_assembly(
            "annotation nesting exceeds the supported safety limit",
        ))
    } else {
        Ok(())
    }
}

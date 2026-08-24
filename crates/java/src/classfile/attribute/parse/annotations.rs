//! Parser for JVM annotation attributes and their nested values.

use crate::{Error, Result};

use super::super::super::io::Reader;
use super::super::super::{Constant, ConstantPool};
use super::super::{
    Annotation, AnnotationConstantKind, AnnotationDefaultAttribute, AnnotationElement,
    AnnotationsAttribute, ElementValue, ElementValueKind, LocalVariableTarget,
    ParameterAnnotationsAttribute, TypeAnnotation, TypeAnnotationTarget, TypeAnnotationTargetKind,
    TypeAnnotationsAttribute, TypePathEntry,
};
use super::{expect_tag, expect_utf8};

const MAX_ANNOTATION_DEPTH: usize = 128;
const ROOT_ANNOTATION_DEPTH: usize = 0;
const ANNOTATION_NESTING_INCREMENT: usize = 1;

pub(super) fn parse_annotations(
    name_index: u16,
    reader: &mut Reader<'_>,
    pool: &ConstantPool,
) -> Result<AnnotationsAttribute> {
    Ok(AnnotationsAttribute {
        name_index,
        annotations: parse_annotation_list(reader, pool, ROOT_ANNOTATION_DEPTH)?,
    })
}

pub(super) fn parse_parameter_annotations(
    name_index: u16,
    reader: &mut Reader<'_>,
    pool: &ConstantPool,
) -> Result<ParameterAnnotationsAttribute> {
    let count = usize::from(reader.read_u8()?);
    let parameters = (0..count)
        .map(|_| parse_annotation_list(reader, pool, ROOT_ANNOTATION_DEPTH))
        .collect::<Result<_>>()?;
    Ok(ParameterAnnotationsAttribute {
        name_index,
        parameters,
    })
}

pub(super) fn parse_type_annotations(
    name_index: u16,
    reader: &mut Reader<'_>,
    pool: &ConstantPool,
) -> Result<TypeAnnotationsAttribute> {
    let count = usize::from(reader.read_u16()?);
    let annotations = (0..count)
        .map(|_| parse_type_annotation(reader, pool))
        .collect::<Result<_>>()?;
    Ok(TypeAnnotationsAttribute {
        name_index,
        annotations,
    })
}

pub(super) fn parse_annotation_default(
    name_index: u16,
    reader: &mut Reader<'_>,
    pool: &ConstantPool,
) -> Result<AnnotationDefaultAttribute> {
    Ok(AnnotationDefaultAttribute {
        name_index,
        value: parse_element_value(reader, pool, ROOT_ANNOTATION_DEPTH)?,
    })
}

fn parse_annotation_list(
    reader: &mut Reader<'_>,
    pool: &ConstantPool,
    depth: usize,
) -> Result<Vec<Annotation>> {
    let count = usize::from(reader.read_u16()?);
    (0..count)
        .map(|_| parse_annotation(reader, pool, depth))
        .collect()
}

fn parse_annotation(
    reader: &mut Reader<'_>,
    pool: &ConstantPool,
    depth: usize,
) -> Result<Annotation> {
    ensure_annotation_depth(reader, depth)?;
    let type_index = reader.read_u16()?;
    expect_utf8(pool, type_index)?;
    let count = usize::from(reader.read_u16()?);
    let mut elements = Vec::with_capacity(count);
    for _ in 0..count {
        let name_index = reader.read_u16()?;
        expect_utf8(pool, name_index)?;
        elements.push(AnnotationElement {
            name_index,
            value: parse_element_value(reader, pool, depth + ANNOTATION_NESTING_INCREMENT)?,
        });
    }
    Ok(Annotation {
        type_index,
        elements,
    })
}

fn parse_element_value(
    reader: &mut Reader<'_>,
    pool: &ConstantPool,
    depth: usize,
) -> Result<ElementValue> {
    ensure_annotation_depth(reader, depth)?;
    let offset = reader.absolute_position();
    let tag = reader.read_u8()?;
    let Some(kind) = ElementValueKind::from_tag(tag) else {
        return Err(Error::invalid_class(
            offset,
            format!("invalid annotation element tag 0x{tag:02x}"),
        ));
    };
    Ok(match kind {
        ElementValueKind::Constant(kind) => {
            let constant_index = reader.read_u16()?;
            expect_annotation_constant(pool, constant_index, kind)?;
            ElementValue::Constant {
                kind,
                constant_index,
            }
        }
        ElementValueKind::Enum => {
            let type_name_index = reader.read_u16()?;
            let constant_name_index = reader.read_u16()?;
            expect_utf8(pool, type_name_index)?;
            expect_utf8(pool, constant_name_index)?;
            ElementValue::Enum {
                type_name_index,
                constant_name_index,
            }
        }
        ElementValueKind::Class => {
            let index = reader.read_u16()?;
            expect_utf8(pool, index)?;
            ElementValue::Class(index)
        }
        ElementValueKind::Annotation => ElementValue::Annotation(Box::new(parse_annotation(
            reader,
            pool,
            depth + ANNOTATION_NESTING_INCREMENT,
        )?)),
        ElementValueKind::Array => {
            let count = usize::from(reader.read_u16()?);
            let values = (0..count)
                .map(|_| parse_element_value(reader, pool, depth + ANNOTATION_NESTING_INCREMENT))
                .collect::<Result<_>>()?;
            ElementValue::Array(values)
        }
    })
}

fn parse_type_annotation(reader: &mut Reader<'_>, pool: &ConstantPool) -> Result<TypeAnnotation> {
    let target = parse_type_target(reader)?;
    let path_count = usize::from(reader.read_u8()?);
    let mut path = Vec::with_capacity(path_count);
    for _ in 0..path_count {
        let kind_offset = reader.absolute_position();
        let kind = reader.read_u8()?;
        let argument = reader.read_u8()?;
        let Some(entry) = TypePathEntry::from_encoded(kind, argument) else {
            return Err(Error::invalid_class(
                kind_offset,
                format!("invalid type path entry ({kind}, {argument})"),
            ));
        };
        path.push(entry);
    }
    Ok(TypeAnnotation {
        target,
        path,
        annotation: parse_annotation(reader, pool, ROOT_ANNOTATION_DEPTH)?,
    })
}

#[allow(clippy::too_many_lines)]
fn parse_type_target(reader: &mut Reader<'_>) -> Result<TypeAnnotationTarget> {
    let offset = reader.absolute_position();
    let tag = reader.read_u8()?;
    let Some(kind) = TypeAnnotationTargetKind::from_tag(tag) else {
        return Err(Error::invalid_class(
            offset,
            format!("invalid type-annotation target tag 0x{tag:02x}"),
        ));
    };
    Ok(match kind {
        TypeAnnotationTargetKind::ClassTypeParameter => {
            TypeAnnotationTarget::ClassTypeParameter(reader.read_u8()?)
        }
        TypeAnnotationTargetKind::MethodTypeParameter => {
            TypeAnnotationTarget::MethodTypeParameter(reader.read_u8()?)
        }
        TypeAnnotationTargetKind::ClassExtends => {
            TypeAnnotationTarget::ClassExtends(reader.read_u16()?)
        }
        TypeAnnotationTargetKind::ClassTypeParameterBound => {
            TypeAnnotationTarget::ClassTypeParameterBound {
                parameter_index: reader.read_u8()?,
                bound_index: reader.read_u8()?,
            }
        }
        TypeAnnotationTargetKind::MethodTypeParameterBound => {
            TypeAnnotationTarget::MethodTypeParameterBound {
                parameter_index: reader.read_u8()?,
                bound_index: reader.read_u8()?,
            }
        }
        TypeAnnotationTargetKind::Field => TypeAnnotationTarget::Field,
        TypeAnnotationTargetKind::MethodReturn => TypeAnnotationTarget::MethodReturn,
        TypeAnnotationTargetKind::MethodReceiver => TypeAnnotationTarget::MethodReceiver,
        TypeAnnotationTargetKind::MethodFormalParameter => {
            TypeAnnotationTarget::MethodFormalParameter(reader.read_u8()?)
        }
        TypeAnnotationTargetKind::Throws => TypeAnnotationTarget::Throws(reader.read_u16()?),
        TypeAnnotationTargetKind::LocalVariable => {
            TypeAnnotationTarget::LocalVariable(parse_local_variable_targets(reader)?)
        }
        TypeAnnotationTargetKind::ResourceVariable => {
            TypeAnnotationTarget::ResourceVariable(parse_local_variable_targets(reader)?)
        }
        TypeAnnotationTargetKind::ExceptionParameter => {
            TypeAnnotationTarget::ExceptionParameter(reader.read_u16()?)
        }
        TypeAnnotationTargetKind::InstanceOf => {
            TypeAnnotationTarget::InstanceOf(reader.read_u16()?)
        }
        TypeAnnotationTargetKind::New => TypeAnnotationTarget::New(reader.read_u16()?),
        TypeAnnotationTargetKind::ConstructorReference => {
            TypeAnnotationTarget::ConstructorReference(reader.read_u16()?)
        }
        TypeAnnotationTargetKind::MethodReference => {
            TypeAnnotationTarget::MethodReference(reader.read_u16()?)
        }
        TypeAnnotationTargetKind::Cast => TypeAnnotationTarget::Cast {
            offset: reader.read_u16()?,
            type_argument_index: reader.read_u8()?,
        },
        TypeAnnotationTargetKind::ConstructorInvocationTypeArgument => {
            TypeAnnotationTarget::ConstructorInvocationTypeArgument {
                offset: reader.read_u16()?,
                type_argument_index: reader.read_u8()?,
            }
        }
        TypeAnnotationTargetKind::MethodInvocationTypeArgument => {
            TypeAnnotationTarget::MethodInvocationTypeArgument {
                offset: reader.read_u16()?,
                type_argument_index: reader.read_u8()?,
            }
        }
        TypeAnnotationTargetKind::ConstructorReferenceTypeArgument => {
            TypeAnnotationTarget::ConstructorReferenceTypeArgument {
                offset: reader.read_u16()?,
                type_argument_index: reader.read_u8()?,
            }
        }
        TypeAnnotationTargetKind::MethodReferenceTypeArgument => {
            TypeAnnotationTarget::MethodReferenceTypeArgument {
                offset: reader.read_u16()?,
                type_argument_index: reader.read_u8()?,
            }
        }
    })
}

fn parse_local_variable_targets(reader: &mut Reader<'_>) -> Result<Vec<LocalVariableTarget>> {
    let count = usize::from(reader.read_u16()?);
    let mut targets = Vec::with_capacity(count);
    for _ in 0..count {
        targets.push(LocalVariableTarget {
            start_pc: reader.read_u16()?,
            length: reader.read_u16()?,
            index: reader.read_u16()?,
        });
    }
    Ok(targets)
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

fn ensure_annotation_depth(reader: &Reader<'_>, depth: usize) -> Result<()> {
    if depth > MAX_ANNOTATION_DEPTH {
        Err(Error::invalid_class(
            reader.absolute_position(),
            "annotation nesting exceeds the supported safety limit",
        ))
    } else {
        Ok(())
    }
}

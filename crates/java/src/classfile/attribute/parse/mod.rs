//! Binary parser for all standard JVM attributes.

mod module;

use crate::{Error, Result};

use self::module::parse_module;
use super::super::io::Reader;
use super::super::{
    Attribute, AttributeLocation, CATCH_ALL_EXCEPTION_INDEX, CodeAttribute, Constant, ConstantPool,
    ExceptionHandler, InnerClassAccessFlags, MAX_CODE_LENGTH, MethodParameterAccessFlags,
    RawAttribute,
};
use super::{
    Annotation, AnnotationConstantKind, AnnotationDefaultAttribute, AnnotationElement,
    AnnotationsAttribute, BootstrapMethod, BootstrapMethodsAttribute, BytesAttribute, ElementValue,
    EnclosingMethodAttribute, IndexAttribute, IndexListAttribute, InnerClass,
    InnerClassesAttribute, KnownAttribute, KnownAttributeKind as StandardKind, LineNumber,
    LineNumberTableAttribute, LocalVariable, LocalVariableTableAttribute, LocalVariableTarget,
    LocalVariableType, LocalVariableTypeTableAttribute, MarkerAttribute, MethodParameter,
    MethodParametersAttribute, ParameterAnnotationsAttribute, RecordAttribute, RecordComponent,
    StackMapFrame, StackMapTableAttribute, TypeAnnotation, TypeAnnotationTarget,
    TypeAnnotationsAttribute, TypePathEntry, VerificationType,
};

const MAX_ANNOTATION_DEPTH: usize = 128;

pub(crate) fn parse_attributes(
    reader: &mut Reader<'_>,
    pool: &ConstantPool,
    location: AttributeLocation,
) -> Result<Vec<Attribute>> {
    let count = usize::from(reader.read_u16()?);
    let mut attributes = Vec::with_capacity(count);
    for _ in 0..count {
        let name_index = reader.read_u16()?;
        let name = pool.utf8(name_index)?.to_owned();
        let length = usize::try_from(reader.read_u32()?).map_err(|_| {
            Error::invalid_class(
                reader.absolute_position(),
                "attribute length does not fit usize",
            )
        })?;
        let info_offset = reader.absolute_position();
        let info = reader.read_bytes(length)?;
        attributes.push(parse_attribute(
            name_index,
            &name,
            info,
            info_offset,
            pool,
            location,
        )?);
    }
    validate_attribute_multiplicity(&attributes, location)?;
    Ok(attributes)
}

fn parse_attribute(
    name_index: u16,
    name: &str,
    info: &[u8],
    info_offset: usize,
    pool: &ConstantPool,
    location: AttributeLocation,
) -> Result<Attribute> {
    if name == "Code" {
        require_location(
            name,
            location,
            location == AttributeLocation::Method,
            info_offset,
        )?;
        return Ok(Attribute::Code(parse_code(
            name_index,
            info,
            info_offset,
            pool,
        )?));
    }
    let Some(kind) = StandardKind::from_name(name) else {
        return Ok(Attribute::Raw(RawAttribute {
            name_index,
            name: name.to_owned(),
            info: info.to_vec(),
        }));
    };
    require_location(name, location, kind.is_valid_at(location), info_offset)?;
    let mut reader = Reader::with_base(info, info_offset);
    let known = parse_known(kind, name_index, &mut reader, pool)?;
    reader.finish(name)?;
    Ok(Attribute::Known(known))
}

#[allow(clippy::too_many_lines)]
fn parse_known(
    kind: StandardKind,
    name_index: u16,
    reader: &mut Reader<'_>,
    pool: &ConstantPool,
) -> Result<KnownAttribute> {
    let attribute = match kind {
        StandardKind::ConstantValue => {
            let index = reader.read_u16()?;
            expect_constant_value(pool, index)?;
            KnownAttribute::ConstantValue(IndexAttribute { name_index, index })
        }
        StandardKind::StackMapTable => {
            KnownAttribute::StackMapTable(parse_stack_map_table(name_index, reader, pool)?)
        }
        StandardKind::Exceptions => {
            let indices = read_u16_list(reader)?;
            expect_each_class(pool, &indices)?;
            KnownAttribute::Exceptions(IndexListAttribute {
                name_index,
                indices,
            })
        }
        StandardKind::InnerClasses => {
            KnownAttribute::InnerClasses(parse_inner_classes(name_index, reader, pool)?)
        }
        StandardKind::EnclosingMethod => {
            let class_index = reader.read_u16()?;
            expect_class(pool, class_index)?;
            let method_index = reader.read_u16()?;
            if method_index != 0 {
                expect_tag(pool, method_index, "NameAndType", |constant| {
                    matches!(constant, Constant::NameAndType { .. })
                })?;
            }
            KnownAttribute::EnclosingMethod(EnclosingMethodAttribute {
                name_index,
                class_index,
                method_index,
            })
        }
        StandardKind::Synthetic => KnownAttribute::Synthetic(MarkerAttribute { name_index }),
        StandardKind::Signature => {
            let index = reader.read_u16()?;
            expect_utf8(pool, index)?;
            KnownAttribute::Signature(IndexAttribute { name_index, index })
        }
        StandardKind::SourceFile => {
            let index = reader.read_u16()?;
            expect_utf8(pool, index)?;
            KnownAttribute::SourceFile(IndexAttribute { name_index, index })
        }
        StandardKind::SourceDebugExtension => {
            let bytes = reader.read_bytes(reader.remaining())?.to_vec();
            KnownAttribute::SourceDebugExtension(BytesAttribute { name_index, bytes })
        }
        StandardKind::LineNumberTable => {
            KnownAttribute::LineNumberTable(parse_line_numbers(name_index, reader)?)
        }
        StandardKind::LocalVariableTable => {
            KnownAttribute::LocalVariableTable(parse_local_variables(name_index, reader, pool)?)
        }
        StandardKind::LocalVariableTypeTable => KnownAttribute::LocalVariableTypeTable(
            parse_local_variable_types(name_index, reader, pool)?,
        ),
        StandardKind::Deprecated => KnownAttribute::Deprecated(MarkerAttribute { name_index }),
        StandardKind::RuntimeVisibleAnnotations => {
            KnownAttribute::RuntimeVisibleAnnotations(AnnotationsAttribute {
                name_index,
                annotations: parse_annotations(reader, pool, 0)?,
            })
        }
        StandardKind::RuntimeInvisibleAnnotations => {
            KnownAttribute::RuntimeInvisibleAnnotations(AnnotationsAttribute {
                name_index,
                annotations: parse_annotations(reader, pool, 0)?,
            })
        }
        StandardKind::RuntimeVisibleParameterAnnotations => {
            KnownAttribute::RuntimeVisibleParameterAnnotations(parse_parameter_annotations(
                name_index, reader, pool,
            )?)
        }
        StandardKind::RuntimeInvisibleParameterAnnotations => {
            KnownAttribute::RuntimeInvisibleParameterAnnotations(parse_parameter_annotations(
                name_index, reader, pool,
            )?)
        }
        StandardKind::RuntimeVisibleTypeAnnotations => {
            KnownAttribute::RuntimeVisibleTypeAnnotations(parse_type_annotations(
                name_index, reader, pool,
            )?)
        }
        StandardKind::RuntimeInvisibleTypeAnnotations => {
            KnownAttribute::RuntimeInvisibleTypeAnnotations(parse_type_annotations(
                name_index, reader, pool,
            )?)
        }
        StandardKind::AnnotationDefault => {
            KnownAttribute::AnnotationDefault(AnnotationDefaultAttribute {
                name_index,
                value: parse_element_value(reader, pool, 0)?,
            })
        }
        StandardKind::BootstrapMethods => {
            KnownAttribute::BootstrapMethods(parse_bootstrap_methods(name_index, reader, pool)?)
        }
        StandardKind::MethodParameters => {
            KnownAttribute::MethodParameters(parse_method_parameters(name_index, reader, pool)?)
        }
        StandardKind::Module => KnownAttribute::Module(parse_module(name_index, reader, pool)?),
        StandardKind::ModulePackages => {
            let indices = read_u16_list(reader)?;
            expect_each(pool, &indices, "Package", |constant| {
                matches!(constant, Constant::Package { .. })
            })?;
            KnownAttribute::ModulePackages(IndexListAttribute {
                name_index,
                indices,
            })
        }
        StandardKind::ModuleMainClass => {
            let index = reader.read_u16()?;
            expect_class(pool, index)?;
            KnownAttribute::ModuleMainClass(IndexAttribute { name_index, index })
        }
        StandardKind::NestHost => {
            let index = reader.read_u16()?;
            expect_class(pool, index)?;
            KnownAttribute::NestHost(IndexAttribute { name_index, index })
        }
        StandardKind::NestMembers => {
            let indices = read_u16_list(reader)?;
            expect_each_class(pool, &indices)?;
            KnownAttribute::NestMembers(IndexListAttribute {
                name_index,
                indices,
            })
        }
        StandardKind::Record => KnownAttribute::Record(parse_record(name_index, reader, pool)?),
        StandardKind::PermittedSubclasses => {
            let indices = read_u16_list(reader)?;
            expect_each_class(pool, &indices)?;
            KnownAttribute::PermittedSubclasses(IndexListAttribute {
                name_index,
                indices,
            })
        }
    };
    Ok(attribute)
}

fn parse_code(
    name_index: u16,
    info: &[u8],
    info_offset: usize,
    pool: &ConstantPool,
) -> Result<CodeAttribute> {
    let mut reader = Reader::with_base(info, info_offset);
    let max_stack = reader.read_u16()?;
    let max_locals = reader.read_u16()?;
    let code_length = usize::try_from(reader.read_u32()?).map_err(|_| {
        Error::invalid_class(reader.absolute_position(), "code length does not fit usize")
    })?;
    if code_length > MAX_CODE_LENGTH {
        return Err(Error::invalid_class(
            reader.absolute_position(),
            format!("Code attribute is too large: {code_length} bytes"),
        ));
    }
    let code = reader.read_bytes(code_length)?.to_vec();
    let exception_count = usize::from(reader.read_u16()?);
    let mut exception_table = Vec::with_capacity(exception_count);
    for _ in 0..exception_count {
        let handler = ExceptionHandler {
            start_pc: reader.read_u16()?,
            end_pc: reader.read_u16()?,
            handler_pc: reader.read_u16()?,
            catch_type: reader.read_u16()?,
        };
        if handler.catch_type != CATCH_ALL_EXCEPTION_INDEX {
            expect_class(pool, handler.catch_type)?;
        }
        exception_table.push(handler);
    }
    let attributes = parse_attributes(&mut reader, pool, AttributeLocation::Code)?;
    reader.finish("Code attribute")?;
    Ok(CodeAttribute {
        name_index,
        max_stack,
        max_locals,
        code,
        exception_table,
        attributes,
    })
}

fn parse_stack_map_table(
    name_index: u16,
    reader: &mut Reader<'_>,
    pool: &ConstantPool,
) -> Result<StackMapTableAttribute> {
    let count = usize::from(reader.read_u16()?);
    let mut frames = Vec::with_capacity(count);
    for _ in 0..count {
        let frame_type = reader.read_u8()?;
        let frame = match frame_type {
            0..=63 => StackMapFrame::Same {
                offset_delta: frame_type,
            },
            64..=127 => StackMapFrame::SameLocalsOneStack {
                offset_delta: frame_type - 64,
                stack: parse_verification_type(reader, pool)?,
            },
            247 => StackMapFrame::SameLocalsOneStackExtended {
                offset_delta: reader.read_u16()?,
                stack: parse_verification_type(reader, pool)?,
            },
            248..=250 => StackMapFrame::Chop {
                offset_delta: reader.read_u16()?,
                absent_locals: 251 - frame_type,
            },
            251 => StackMapFrame::SameExtended {
                offset_delta: reader.read_u16()?,
            },
            252..=254 => {
                let offset_delta = reader.read_u16()?;
                let locals = parse_verification_types(reader, pool, usize::from(frame_type - 251))?;
                StackMapFrame::Append {
                    offset_delta,
                    locals,
                }
            }
            255 => StackMapFrame::Full {
                offset_delta: reader.read_u16()?,
                locals: parse_counted_verification_types(reader, pool)?,
                stack: parse_counted_verification_types(reader, pool)?,
            },
            _ => {
                return Err(Error::invalid_class(
                    reader.absolute_position().saturating_sub(1),
                    format!("reserved stack-map frame type {frame_type}"),
                ));
            }
        };
        frames.push(frame);
    }
    Ok(StackMapTableAttribute { name_index, frames })
}

fn parse_counted_verification_types(
    reader: &mut Reader<'_>,
    pool: &ConstantPool,
) -> Result<Vec<VerificationType>> {
    let count = usize::from(reader.read_u16()?);
    parse_verification_types(reader, pool, count)
}

fn parse_verification_types(
    reader: &mut Reader<'_>,
    pool: &ConstantPool,
    count: usize,
) -> Result<Vec<VerificationType>> {
    (0..count)
        .map(|_| parse_verification_type(reader, pool))
        .collect()
}

fn parse_verification_type(
    reader: &mut Reader<'_>,
    pool: &ConstantPool,
) -> Result<VerificationType> {
    let offset = reader.absolute_position();
    Ok(match reader.read_u8()? {
        0 => VerificationType::Top,
        1 => VerificationType::Integer,
        2 => VerificationType::Float,
        3 => VerificationType::Double,
        4 => VerificationType::Long,
        5 => VerificationType::Null,
        6 => VerificationType::UninitializedThis,
        7 => {
            let index = reader.read_u16()?;
            expect_class(pool, index)?;
            VerificationType::Object(index)
        }
        8 => VerificationType::Uninitialized(reader.read_u16()?),
        tag => {
            return Err(Error::invalid_class(
                offset,
                format!("invalid verification-type tag {tag}"),
            ));
        }
    })
}

fn parse_inner_classes(
    name_index: u16,
    reader: &mut Reader<'_>,
    pool: &ConstantPool,
) -> Result<InnerClassesAttribute> {
    let count = usize::from(reader.read_u16()?);
    let mut classes = Vec::with_capacity(count);
    for _ in 0..count {
        let inner_class_info_index = reader.read_u16()?;
        expect_class(pool, inner_class_info_index)?;
        let outer_class_info_index = reader.read_u16()?;
        expect_optional(pool, outer_class_info_index, "Class", |constant| {
            matches!(constant, Constant::Class { .. })
        })?;
        let inner_name_index = reader.read_u16()?;
        expect_optional(pool, inner_name_index, "Utf8", |constant| {
            matches!(constant, Constant::Utf8(_))
        })?;
        classes.push(InnerClass {
            inner_class_info_index,
            outer_class_info_index,
            inner_name_index,
            access_flags: InnerClassAccessFlags::from_bits_retain(reader.read_u16()?),
        });
    }
    Ok(InnerClassesAttribute {
        name_index,
        classes,
    })
}

fn parse_line_numbers(
    name_index: u16,
    reader: &mut Reader<'_>,
) -> Result<LineNumberTableAttribute> {
    let count = usize::from(reader.read_u16()?);
    let mut lines = Vec::with_capacity(count);
    for _ in 0..count {
        lines.push(LineNumber {
            start_pc: reader.read_u16()?,
            line_number: reader.read_u16()?,
        });
    }
    Ok(LineNumberTableAttribute { name_index, lines })
}

fn parse_local_variables(
    name_index: u16,
    reader: &mut Reader<'_>,
    pool: &ConstantPool,
) -> Result<LocalVariableTableAttribute> {
    let count = usize::from(reader.read_u16()?);
    let mut variables = Vec::with_capacity(count);
    for _ in 0..count {
        let variable = LocalVariable {
            start_pc: reader.read_u16()?,
            length: reader.read_u16()?,
            name_index: reader.read_u16()?,
            descriptor_index: reader.read_u16()?,
            slot: reader.read_u16()?,
        };
        expect_utf8(pool, variable.name_index)?;
        expect_utf8(pool, variable.descriptor_index)?;
        variables.push(variable);
    }
    Ok(LocalVariableTableAttribute {
        name_index,
        variables,
    })
}

fn parse_local_variable_types(
    name_index: u16,
    reader: &mut Reader<'_>,
    pool: &ConstantPool,
) -> Result<LocalVariableTypeTableAttribute> {
    let count = usize::from(reader.read_u16()?);
    let mut variables = Vec::with_capacity(count);
    for _ in 0..count {
        let variable = LocalVariableType {
            start_pc: reader.read_u16()?,
            length: reader.read_u16()?,
            name_index: reader.read_u16()?,
            signature_index: reader.read_u16()?,
            slot: reader.read_u16()?,
        };
        expect_utf8(pool, variable.name_index)?;
        expect_utf8(pool, variable.signature_index)?;
        variables.push(variable);
    }
    Ok(LocalVariableTypeTableAttribute {
        name_index,
        variables,
    })
}

fn parse_annotations(
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
            value: parse_element_value(reader, pool, depth + 1)?,
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
    if let Some(kind) = AnnotationConstantKind::from_tag(tag) {
        let constant_index = reader.read_u16()?;
        expect_annotation_constant(pool, constant_index, kind)?;
        return Ok(ElementValue::Constant {
            kind,
            constant_index,
        });
    }
    Ok(match tag {
        b'e' => {
            let type_name_index = reader.read_u16()?;
            let constant_name_index = reader.read_u16()?;
            expect_utf8(pool, type_name_index)?;
            expect_utf8(pool, constant_name_index)?;
            ElementValue::Enum {
                type_name_index,
                constant_name_index,
            }
        }
        b'c' => {
            let index = reader.read_u16()?;
            expect_utf8(pool, index)?;
            ElementValue::Class(index)
        }
        b'@' => ElementValue::Annotation(Box::new(parse_annotation(reader, pool, depth + 1)?)),
        b'[' => {
            let count = usize::from(reader.read_u16()?);
            let values = (0..count)
                .map(|_| parse_element_value(reader, pool, depth + 1))
                .collect::<Result<_>>()?;
            ElementValue::Array(values)
        }
        _ => {
            return Err(Error::invalid_class(
                offset,
                format!("invalid annotation element tag 0x{tag:02x}"),
            ));
        }
    })
}

fn parse_parameter_annotations(
    name_index: u16,
    reader: &mut Reader<'_>,
    pool: &ConstantPool,
) -> Result<ParameterAnnotationsAttribute> {
    let count = usize::from(reader.read_u8()?);
    let parameters = (0..count)
        .map(|_| parse_annotations(reader, pool, 0))
        .collect::<Result<_>>()?;
    Ok(ParameterAnnotationsAttribute {
        name_index,
        parameters,
    })
}

fn parse_type_annotations(
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

fn parse_type_annotation(reader: &mut Reader<'_>, pool: &ConstantPool) -> Result<TypeAnnotation> {
    let target = parse_type_target(reader)?;
    let path_count = usize::from(reader.read_u8()?);
    let mut path = Vec::with_capacity(path_count);
    for _ in 0..path_count {
        let kind_offset = reader.absolute_position();
        let kind = reader.read_u8()?;
        let argument = reader.read_u8()?;
        path.push(match (kind, argument) {
            (0, 0) => TypePathEntry::Array,
            (1, 0) => TypePathEntry::Nested,
            (2, 0) => TypePathEntry::WildcardBound,
            (3, index) => TypePathEntry::TypeArgument(index),
            _ => {
                return Err(Error::invalid_class(
                    kind_offset,
                    format!("invalid type path entry ({kind}, {argument})"),
                ));
            }
        });
    }
    Ok(TypeAnnotation {
        target,
        path,
        annotation: parse_annotation(reader, pool, 0)?,
    })
}

#[allow(clippy::too_many_lines)]
fn parse_type_target(reader: &mut Reader<'_>) -> Result<TypeAnnotationTarget> {
    let offset = reader.absolute_position();
    Ok(match reader.read_u8()? {
        0x00 => TypeAnnotationTarget::ClassTypeParameter(reader.read_u8()?),
        0x01 => TypeAnnotationTarget::MethodTypeParameter(reader.read_u8()?),
        0x10 => TypeAnnotationTarget::ClassExtends(reader.read_u16()?),
        0x11 => TypeAnnotationTarget::ClassTypeParameterBound {
            parameter_index: reader.read_u8()?,
            bound_index: reader.read_u8()?,
        },
        0x12 => TypeAnnotationTarget::MethodTypeParameterBound {
            parameter_index: reader.read_u8()?,
            bound_index: reader.read_u8()?,
        },
        0x13 => TypeAnnotationTarget::Field,
        0x14 => TypeAnnotationTarget::MethodReturn,
        0x15 => TypeAnnotationTarget::MethodReceiver,
        0x16 => TypeAnnotationTarget::MethodFormalParameter(reader.read_u8()?),
        0x17 => TypeAnnotationTarget::Throws(reader.read_u16()?),
        0x40 => TypeAnnotationTarget::LocalVariable(parse_local_variable_targets(reader)?),
        0x41 => TypeAnnotationTarget::ResourceVariable(parse_local_variable_targets(reader)?),
        0x42 => TypeAnnotationTarget::ExceptionParameter(reader.read_u16()?),
        0x43 => TypeAnnotationTarget::InstanceOf(reader.read_u16()?),
        0x44 => TypeAnnotationTarget::New(reader.read_u16()?),
        0x45 => TypeAnnotationTarget::ConstructorReference(reader.read_u16()?),
        0x46 => TypeAnnotationTarget::MethodReference(reader.read_u16()?),
        0x47 => TypeAnnotationTarget::Cast {
            offset: reader.read_u16()?,
            type_argument_index: reader.read_u8()?,
        },
        0x48 => TypeAnnotationTarget::ConstructorInvocationTypeArgument {
            offset: reader.read_u16()?,
            type_argument_index: reader.read_u8()?,
        },
        0x49 => TypeAnnotationTarget::MethodInvocationTypeArgument {
            offset: reader.read_u16()?,
            type_argument_index: reader.read_u8()?,
        },
        0x4a => TypeAnnotationTarget::ConstructorReferenceTypeArgument {
            offset: reader.read_u16()?,
            type_argument_index: reader.read_u8()?,
        },
        0x4b => TypeAnnotationTarget::MethodReferenceTypeArgument {
            offset: reader.read_u16()?,
            type_argument_index: reader.read_u8()?,
        },
        tag => {
            return Err(Error::invalid_class(
                offset,
                format!("invalid type-annotation target tag 0x{tag:02x}"),
            ));
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

fn parse_bootstrap_methods(
    name_index: u16,
    reader: &mut Reader<'_>,
    pool: &ConstantPool,
) -> Result<BootstrapMethodsAttribute> {
    let count = usize::from(reader.read_u16()?);
    let mut methods = Vec::with_capacity(count);
    for _ in 0..count {
        let method_ref = reader.read_u16()?;
        expect_tag(pool, method_ref, "MethodHandle", |constant| {
            matches!(constant, Constant::MethodHandle { .. })
        })?;
        let arguments = read_u16_list(reader)?;
        for &index in &arguments {
            expect_loadable_constant(pool, index)?;
        }
        methods.push(BootstrapMethod {
            method_ref,
            arguments,
        });
    }
    Ok(BootstrapMethodsAttribute {
        name_index,
        methods,
    })
}

fn parse_method_parameters(
    name_index: u16,
    reader: &mut Reader<'_>,
    pool: &ConstantPool,
) -> Result<MethodParametersAttribute> {
    let count = usize::from(reader.read_u8()?);
    let mut parameters = Vec::with_capacity(count);
    for _ in 0..count {
        let parameter = MethodParameter {
            name_index: reader.read_u16()?,
            access_flags: MethodParameterAccessFlags::from_bits_retain(reader.read_u16()?),
        };
        expect_optional(pool, parameter.name_index, "Utf8", |constant| {
            matches!(constant, Constant::Utf8(_))
        })?;
        parameters.push(parameter);
    }
    Ok(MethodParametersAttribute {
        name_index,
        parameters,
    })
}

fn parse_record(
    name_index: u16,
    reader: &mut Reader<'_>,
    pool: &ConstantPool,
) -> Result<RecordAttribute> {
    let count = usize::from(reader.read_u16()?);
    let mut components = Vec::with_capacity(count);
    for _ in 0..count {
        let component_name_index = reader.read_u16()?;
        let descriptor_index = reader.read_u16()?;
        expect_utf8(pool, component_name_index)?;
        expect_utf8(pool, descriptor_index)?;
        components.push(RecordComponent {
            name_index: component_name_index,
            descriptor_index,
            attributes: parse_attributes(reader, pool, AttributeLocation::RecordComponent)?,
        });
    }
    Ok(RecordAttribute {
        name_index,
        components,
    })
}

fn read_u16_list(reader: &mut Reader<'_>) -> Result<Vec<u16>> {
    let count = usize::from(reader.read_u16()?);
    (0..count).map(|_| reader.read_u16()).collect()
}

fn expect_constant_value(pool: &ConstantPool, index: u16) -> Result<()> {
    expect_tag(pool, index, "constant value", |constant| {
        matches!(
            constant,
            Constant::Integer(_)
                | Constant::Float(_)
                | Constant::Long(_)
                | Constant::Double(_)
                | Constant::String { .. }
        )
    })
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

fn expect_each_class(pool: &ConstantPool, indices: &[u16]) -> Result<()> {
    expect_each(pool, indices, "Class", |constant| {
        matches!(constant, Constant::Class { .. })
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
        Err(Error::invalid_class(
            0,
            format!(
                "constant-pool index #{index} is {}, expected {expected}",
                constant.tag_name()
            ),
        ))
    }
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

fn require_location(
    name: &str,
    actual: AttributeLocation,
    allowed: bool,
    offset: usize,
) -> Result<()> {
    if allowed {
        Ok(())
    } else {
        Err(Error::invalid_class(
            offset,
            format!("{name} attribute is not valid at {actual:?} location"),
        ))
    }
}

fn validate_attribute_multiplicity(
    attributes: &[Attribute],
    location: AttributeLocation,
) -> Result<()> {
    let code_count = attributes
        .iter()
        .filter(|attribute| matches!(attribute, Attribute::Code(_)))
        .count();
    if code_count > 1 {
        return Err(Error::invalid_class(
            0,
            format!("multiple Code attributes at {location:?} location"),
        ));
    }
    Ok(())
}

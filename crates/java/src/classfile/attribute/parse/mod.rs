//! Binary parser for all standard JVM attributes.

mod annotations;
mod module;

use crate::{Error, Result};

use self::annotations::{
    parse_annotation_default, parse_annotations, parse_parameter_annotations,
    parse_type_annotations,
};
use self::module::parse_module;
use super::super::io::Reader;
use super::super::{
    Attribute, AttributeLocation, CATCH_ALL_EXCEPTION_INDEX, CODE_ATTRIBUTE_NAME, CodeAttribute,
    Constant, ConstantPool, ExceptionHandler, InnerClassAccessFlags, MAX_CODE_LENGTH,
    MODEL_VALIDATION_OFFSET, MethodParameterAccessFlags, OPTIONAL_CONSTANT_POOL_INDEX,
    RawAttribute,
};
use super::{
    BootstrapMethod, BootstrapMethodsAttribute, BytesAttribute, EnclosingMethodAttribute,
    IndexAttribute, IndexListAttribute, InnerClass, InnerClassesAttribute, KnownAttribute,
    KnownAttributeKind as StandardKind, LineNumber, LineNumberTableAttribute, LocalVariable,
    LocalVariableTableAttribute, LocalVariableType, LocalVariableTypeTableAttribute,
    MarkerAttribute, MethodParameter, MethodParametersAttribute, RecordAttribute, RecordComponent,
    StackMapFrame, StackMapFrameTag, StackMapTableAttribute, VerificationType,
    VerificationTypeKind,
};

const MAX_CODE_ATTRIBUTES_PER_LOCATION: usize = 1;

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
    if name == CODE_ATTRIBUTE_NAME {
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
            if method_index != OPTIONAL_CONSTANT_POOL_INDEX {
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
            KnownAttribute::RuntimeVisibleAnnotations(parse_annotations(name_index, reader, pool)?)
        }
        StandardKind::RuntimeInvisibleAnnotations => KnownAttribute::RuntimeInvisibleAnnotations(
            parse_annotations(name_index, reader, pool)?,
        ),
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
            KnownAttribute::AnnotationDefault(parse_annotation_default(name_index, reader, pool)?)
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
        let frame_type_byte = reader.read_u8()?;
        let Some(frame_type) = StackMapFrameTag::from_byte(frame_type_byte) else {
            return Err(Error::invalid_class(
                reader
                    .absolute_position()
                    .saturating_sub(std::mem::size_of::<u8>()),
                format!("reserved stack-map frame type {frame_type_byte}"),
            ));
        };
        let frame = match frame_type {
            StackMapFrameTag::Same(offset_delta) => StackMapFrame::Same { offset_delta },
            StackMapFrameTag::SameLocalsOneStack(offset_delta) => {
                StackMapFrame::SameLocalsOneStack {
                    offset_delta,
                    stack: parse_verification_type(reader, pool)?,
                }
            }
            StackMapFrameTag::SameLocalsOneStackExtended => {
                StackMapFrame::SameLocalsOneStackExtended {
                    offset_delta: reader.read_u16()?,
                    stack: parse_verification_type(reader, pool)?,
                }
            }
            StackMapFrameTag::Chop(absent_locals) => StackMapFrame::Chop {
                offset_delta: reader.read_u16()?,
                absent_locals,
            },
            StackMapFrameTag::SameExtended => StackMapFrame::SameExtended {
                offset_delta: reader.read_u16()?,
            },
            StackMapFrameTag::Append(local_count) => {
                let offset_delta = reader.read_u16()?;
                let locals = parse_verification_types(reader, pool, usize::from(local_count))?;
                StackMapFrame::Append {
                    offset_delta,
                    locals,
                }
            }
            StackMapFrameTag::Full => StackMapFrame::Full {
                offset_delta: reader.read_u16()?,
                locals: parse_counted_verification_types(reader, pool)?,
                stack: parse_counted_verification_types(reader, pool)?,
            },
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
    let tag = reader.read_u8()?;
    let Some(kind) = VerificationTypeKind::from_tag(tag) else {
        return Err(Error::invalid_class(
            offset,
            format!("invalid verification-type tag {tag}"),
        ));
    };
    Ok(match kind {
        VerificationTypeKind::Top => VerificationType::Top,
        VerificationTypeKind::Integer => VerificationType::Integer,
        VerificationTypeKind::Float => VerificationType::Float,
        VerificationTypeKind::Double => VerificationType::Double,
        VerificationTypeKind::Long => VerificationType::Long,
        VerificationTypeKind::Null => VerificationType::Null,
        VerificationTypeKind::UninitializedThis => VerificationType::UninitializedThis,
        VerificationTypeKind::Object => {
            let index = reader.read_u16()?;
            expect_class(pool, index)?;
            VerificationType::Object(index)
        }
        VerificationTypeKind::Uninitialized => VerificationType::Uninitialized(reader.read_u16()?),
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
        Err(Error::invalid_class(
            MODEL_VALIDATION_OFFSET,
            format!(
                "constant-pool index #{index} is {}, expected {expected}",
                constant.tag_name()
            ),
        ))
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
    if code_count > MAX_CODE_ATTRIBUTES_PER_LOCATION {
        return Err(Error::invalid_class(
            MODEL_VALIDATION_OFFSET,
            format!("multiple Code attributes at {location:?} location"),
        ));
    }
    Ok(())
}

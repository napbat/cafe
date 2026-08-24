//! Encoded values, arrays, and annotations.

use crate::{Error, Result};

use super::Context;
use crate::file::io::Cursor;
use crate::file::layout::{Alignment, ItemWidth};
use crate::file::model::{
    AnnotationElement, ENCODED_VALUE_ARGUMENT_SHIFT, ENCODED_VALUE_TAG_MASK,
    ENCODED_VALUE_WIDTH_BIAS, EncodedAnnotation, EncodedValue, EncodedValueTag, FieldIndex,
    MAX_ENCODED_VALUE_DEPTH, MethodHandleIndex, MethodIndex, PrototypeIndex, StringIndex,
    TypeIndex,
};

const BYTE_WIDTH_BITS: usize = u8::BITS as usize;
const FLOAT_WIDTH_BYTES: usize = size_of::<u32>();
const DOUBLE_WIDTH_BYTES: usize = size_of::<u64>();

pub(super) fn array_at(
    context: &Context<'_>,
    encoded_offset: u32,
    what: &str,
) -> Result<Vec<EncodedValue>> {
    let offset = context.offset(encoded_offset, Alignment::Byte, what)?;
    let mut cursor = context.reader.cursor(offset)?;
    array(context, &mut cursor, 0)
}

pub(super) fn array(
    context: &Context<'_>,
    cursor: &mut Cursor<'_>,
    depth: usize,
) -> Result<Vec<EncodedValue>> {
    require_depth(cursor.position(), depth)?;
    let count_offset = cursor.position();
    let count = cursor.uleb128()?;
    let count = context.count(count, ItemWidth::BYTE, cursor.position(), "encoded array")?;
    let mut values = Vec::with_capacity(count);
    for _ in 0..count {
        values.push(encoded(context, cursor, depth + 1)?);
    }
    if cursor.position() < count_offset {
        return Err(Error::invalid_dex(
            count_offset,
            "encoded array cursor overflowed",
        ));
    }
    Ok(values)
}

pub(super) fn annotation(
    context: &Context<'_>,
    cursor: &mut Cursor<'_>,
    depth: usize,
) -> Result<EncodedAnnotation> {
    require_depth(cursor.position(), depth)?;
    let start = cursor.position();
    let annotation_type = cursor.uleb128()?;
    context.index(
        annotation_type,
        context.header.type_ids.size,
        start,
        "annotation type",
    )?;
    let count = cursor.uleb128()?;
    let count = context.count(
        count,
        ItemWidth::ANNOTATION_ELEMENT_MINIMUM,
        cursor.position(),
        "annotation elements",
    )?;
    let mut elements = Vec::with_capacity(count);
    let mut previous_name = None;
    for _ in 0..count {
        let item_offset = cursor.position();
        let name = cursor.uleb128()?;
        context.index(
            name,
            context.header.string_ids.size,
            item_offset,
            "annotation element name",
        )?;
        if previous_name.is_some_and(|previous| name <= previous) {
            return Err(Error::invalid_dex(
                item_offset,
                "annotation element names are not strictly increasing",
            ));
        }
        previous_name = Some(name);
        elements.push(AnnotationElement {
            name: StringIndex(name),
            value: encoded(context, cursor, depth + 1)?,
        });
    }
    Ok(EncodedAnnotation {
        annotation_type: TypeIndex(annotation_type),
        elements,
    })
}

#[allow(clippy::too_many_lines)]
fn encoded(context: &Context<'_>, cursor: &mut Cursor<'_>, depth: usize) -> Result<EncodedValue> {
    require_depth(cursor.position(), depth)?;
    let start = cursor.position();
    let header = cursor.u8()?;
    let raw_tag = header & ENCODED_VALUE_TAG_MASK;
    let tag = EncodedValueTag::from_byte(raw_tag).ok_or_else(|| {
        Error::invalid_dex(start, format!("unknown encoded-value type 0x{raw_tag:02x}"))
    })?;
    let argument = header >> ENCODED_VALUE_ARGUMENT_SHIFT;
    let length = usize::from(argument + ENCODED_VALUE_WIDTH_BIAS);
    let value =
        match tag {
            EncodedValueTag::Byte => {
                require_argument(start, tag, argument)?;
                EncodedValue::Byte(i8::from_ne_bytes([cursor.u8()?]))
            }
            EncodedValueTag::Short => {
                require_argument(start, tag, argument)?;
                EncodedValue::Short(signed(cursor.bytes(length)?).try_into().map_err(|_| {
                    Error::invalid_dex(start, "encoded short exceeds its native width")
                })?)
            }
            EncodedValueTag::Char => {
                require_argument(start, tag, argument)?;
                EncodedValue::Char(unsigned(cursor.bytes(length)?).try_into().map_err(|_| {
                    Error::invalid_dex(start, "encoded char exceeds its native width")
                })?)
            }
            EncodedValueTag::Int => {
                require_argument(start, tag, argument)?;
                EncodedValue::Int(signed(cursor.bytes(length)?).try_into().map_err(|_| {
                    Error::invalid_dex(start, "encoded int exceeds its native width")
                })?)
            }
            EncodedValueTag::Long => {
                require_argument(start, tag, argument)?;
                EncodedValue::Long(signed(cursor.bytes(length)?))
            }
            EncodedValueTag::Float => {
                require_argument(start, tag, argument)?;
                let bits = unsigned(cursor.bytes(length)?)
                    << ((FLOAT_WIDTH_BYTES - length) * BYTE_WIDTH_BITS);
                EncodedValue::Float(u32::try_from(bits).map_err(|_| {
                    Error::invalid_dex(start, "encoded float exceeds its native width")
                })?)
            }
            EncodedValueTag::Double => {
                require_argument(start, tag, argument)?;
                let bits = unsigned(cursor.bytes(length)?)
                    << ((DOUBLE_WIDTH_BYTES - length) * BYTE_WIDTH_BITS);
                EncodedValue::Double(bits)
            }
            EncodedValueTag::MethodType => EncodedValue::MethodType(PrototypeIndex(index(
                context,
                cursor,
                start,
                tag,
                argument,
                context.header.proto_ids.size,
                "method-type",
            )?)),
            EncodedValueTag::MethodHandle => EncodedValue::MethodHandle(MethodHandleIndex(index(
                context,
                cursor,
                start,
                tag,
                argument,
                context
                    .map_item(crate::file::MapItemType::MethodHandle)
                    .map_or(0, |item| item.size),
                "method-handle",
            )?)),
            EncodedValueTag::String => EncodedValue::String(StringIndex(index(
                context,
                cursor,
                start,
                tag,
                argument,
                context.header.string_ids.size,
                "string",
            )?)),
            EncodedValueTag::Type => EncodedValue::Type(TypeIndex(index(
                context,
                cursor,
                start,
                tag,
                argument,
                context.header.type_ids.size,
                "type",
            )?)),
            EncodedValueTag::Field => EncodedValue::Field(FieldIndex(index(
                context,
                cursor,
                start,
                tag,
                argument,
                context.header.field_ids.size,
                "field",
            )?)),
            EncodedValueTag::Method => EncodedValue::Method(MethodIndex(index(
                context,
                cursor,
                start,
                tag,
                argument,
                context.header.method_ids.size,
                "method",
            )?)),
            EncodedValueTag::Enum => EncodedValue::Enum(FieldIndex(index(
                context,
                cursor,
                start,
                tag,
                argument,
                context.header.field_ids.size,
                "enum field",
            )?)),
            EncodedValueTag::Array => {
                require_argument(start, tag, argument)?;
                EncodedValue::Array(array(context, cursor, depth + 1)?)
            }
            EncodedValueTag::Annotation => {
                require_argument(start, tag, argument)?;
                EncodedValue::Annotation(annotation(context, cursor, depth + 1)?)
            }
            EncodedValueTag::Null => {
                require_argument(start, tag, argument)?;
                EncodedValue::Null
            }
            EncodedValueTag::Boolean => {
                require_argument(start, tag, argument)?;
                EncodedValue::Boolean(argument == tag.maximum_argument())
            }
        };
    Ok(value)
}

fn index(
    context: &Context<'_>,
    cursor: &mut Cursor<'_>,
    start: usize,
    tag: EncodedValueTag,
    argument: u8,
    limit: u32,
    what: &str,
) -> Result<u32> {
    require_argument(start, tag, argument)?;
    let value = u32::try_from(unsigned(
        cursor.bytes(usize::from(argument + ENCODED_VALUE_WIDTH_BIAS))?,
    ))
    .map_err(|_| Error::invalid_dex(start, format!("encoded {what} index exceeds 32 bits")))?;
    context.index(value, limit, start, what)
}

fn require_argument(offset: usize, tag: EncodedValueTag, actual: u8) -> Result<()> {
    let maximum = tag.maximum_argument();
    if actual <= maximum {
        Ok(())
    } else {
        Err(Error::invalid_dex(
            offset,
            format!(
                "encoded-value type 0x{:02x} argument {actual} exceeds {maximum}",
                tag.byte()
            ),
        ))
    }
}

fn require_depth(offset: usize, depth: usize) -> Result<()> {
    if depth <= MAX_ENCODED_VALUE_DEPTH {
        Ok(())
    } else {
        Err(Error::invalid_dex(
            offset,
            format!("encoded-value nesting exceeds {MAX_ENCODED_VALUE_DEPTH} levels"),
        ))
    }
}

fn unsigned(bytes: &[u8]) -> u64 {
    bytes.iter().enumerate().fold(0u64, |value, (index, byte)| {
        value | (u64::from(*byte) << (index * BYTE_WIDTH_BITS))
    })
}

fn signed(bytes: &[u8]) -> i64 {
    let unsigned = unsigned(bytes);
    let bits = bytes.len() * BYTE_WIDTH_BITS;
    let extended = if bits < u64::BITS as usize && unsigned & (1_u64 << (bits - 1)) != 0 {
        unsigned | (u64::MAX << bits)
    } else {
        unsigned
    };
    i64::from_ne_bytes(extended.to_ne_bytes())
}

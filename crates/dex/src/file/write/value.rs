//! Canonical encoded-value, array, and annotation output.

use crate::file::io::Writer;
use crate::file::model::{
    ENCODED_VALUE_NESTING_INCREMENT, ENCODED_VALUE_TAG_MASK, EncodedAnnotation, EncodedValue,
    EncodedValueArgument, EncodedValueTag, MAX_ENCODED_VALUE_DEPTH,
};
use crate::{Error, Result};

const SIGN_BIT_MASK: u8 = 0x80;
const FIRST_VALUE_BYTE_INDEX: usize = 0;
const MINIMUM_ENCODED_PAYLOAD_WIDTH: usize = size_of::<u8>();
const LAST_BYTE_DISTANCE: usize = 1;
const PENULTIMATE_BYTE_DISTANCE: usize = 2;
const POSITION_TO_LENGTH_BIAS: usize = 1;

pub(super) fn array(writer: &mut Writer, values: &[EncodedValue], depth: usize) -> Result<()> {
    require_depth(depth)?;
    writer.uleb128(u32::try_from(values.len()).map_err(|_| {
        Error::invalid_assembly("encoded-array element count exceeds 32-bit address space")
    })?);
    for value in values {
        encoded(writer, value, depth + ENCODED_VALUE_NESTING_INCREMENT)?;
    }
    Ok(())
}

pub(super) fn annotation(
    writer: &mut Writer,
    annotation: &EncodedAnnotation,
    depth: usize,
) -> Result<()> {
    require_depth(depth)?;
    writer.uleb128(annotation.annotation_type.get());
    writer.uleb128(u32::try_from(annotation.elements.len()).map_err(|_| {
        Error::invalid_assembly("annotation element count exceeds 32-bit address space")
    })?);
    let mut previous = None;
    for element in &annotation.elements {
        if previous.is_some_and(|previous| element.name.get() <= previous) {
            return Err(Error::invalid_assembly(
                "annotation element names are not strictly increasing",
            ));
        }
        previous = Some(element.name.get());
        writer.uleb128(element.name.get());
        encoded(
            writer,
            &element.value,
            depth + ENCODED_VALUE_NESTING_INCREMENT,
        )?;
    }
    Ok(())
}

fn encoded(writer: &mut Writer, value: &EncodedValue, depth: usize) -> Result<()> {
    require_depth(depth)?;
    match value {
        EncodedValue::Byte(value) => {
            header(writer, EncodedValueTag::Byte, EncodedValueArgument::ZERO);
            writer.u8(value.to_ne_bytes()[FIRST_VALUE_BYTE_INDEX]);
        }
        EncodedValue::Short(value) => {
            signed_value(writer, EncodedValueTag::Short, i64::from(*value))?;
        }
        EncodedValue::Char(value) => {
            unsigned_value(writer, EncodedValueTag::Char, u64::from(*value))?;
        }
        EncodedValue::Int(value) => {
            signed_value(writer, EncodedValueTag::Int, i64::from(*value))?;
        }
        EncodedValue::Long(value) => signed_value(writer, EncodedValueTag::Long, *value)?,
        EncodedValue::Float(bits) => {
            right_extended(writer, EncodedValueTag::Float, &bits.to_le_bytes())?;
        }
        EncodedValue::Double(bits) => {
            right_extended(writer, EncodedValueTag::Double, &bits.to_le_bytes())?;
        }
        EncodedValue::MethodType(index) => {
            unsigned_value(writer, EncodedValueTag::MethodType, u64::from(index.get()))?;
        }
        EncodedValue::MethodHandle(index) => {
            unsigned_value(
                writer,
                EncodedValueTag::MethodHandle,
                u64::from(index.get()),
            )?;
        }
        EncodedValue::String(index) => {
            unsigned_value(writer, EncodedValueTag::String, u64::from(index.get()))?;
        }
        EncodedValue::Type(index) => {
            unsigned_value(writer, EncodedValueTag::Type, u64::from(index.get()))?;
        }
        EncodedValue::Field(index) => {
            unsigned_value(writer, EncodedValueTag::Field, u64::from(index.get()))?;
        }
        EncodedValue::Method(index) => {
            unsigned_value(writer, EncodedValueTag::Method, u64::from(index.get()))?;
        }
        EncodedValue::Enum(index) => {
            unsigned_value(writer, EncodedValueTag::Enum, u64::from(index.get()))?;
        }
        EncodedValue::Array(values) => {
            header(writer, EncodedValueTag::Array, EncodedValueArgument::ZERO);
            array(writer, values, depth + ENCODED_VALUE_NESTING_INCREMENT)?;
        }
        EncodedValue::Annotation(annotation) => {
            header(
                writer,
                EncodedValueTag::Annotation,
                EncodedValueArgument::ZERO,
            );
            self::annotation(writer, annotation, depth + ENCODED_VALUE_NESTING_INCREMENT)?;
        }
        EncodedValue::Null => header(writer, EncodedValueTag::Null, EncodedValueArgument::ZERO),
        EncodedValue::Boolean(value) => {
            header(
                writer,
                EncodedValueTag::Boolean,
                EncodedValueArgument::from_boolean(*value),
            );
        }
    }
    Ok(())
}

fn signed_value(writer: &mut Writer, tag: EncodedValueTag, value: i64) -> Result<()> {
    let bytes = value.to_le_bytes();
    let mut length = bytes.len();
    while length > MINIMUM_ENCODED_PAYLOAD_WIDTH {
        let high = bytes[length - LAST_BYTE_DISTANCE];
        let next_sign = bytes[length - PENULTIMATE_BYTE_DISTANCE] & SIGN_BIT_MASK != 0;
        if (high == u8::MIN && !next_sign) || (high == u8::MAX && next_sign) {
            length -= LAST_BYTE_DISTANCE;
        } else {
            break;
        }
    }
    write_bytes(writer, tag, &bytes[..length])
}

fn unsigned_value(writer: &mut Writer, tag: EncodedValueTag, value: u64) -> Result<()> {
    let bytes = value.to_le_bytes();
    let length = bytes
        .iter()
        .rposition(|byte| *byte != u8::MIN)
        .map_or(MINIMUM_ENCODED_PAYLOAD_WIDTH, |position| {
            position + POSITION_TO_LENGTH_BIAS
        });
    write_bytes(writer, tag, &bytes[..length])
}

fn right_extended(writer: &mut Writer, tag: EncodedValueTag, bytes: &[u8]) -> Result<()> {
    let first = bytes
        .iter()
        .position(|byte| *byte != u8::MIN)
        .unwrap_or(bytes.len() - LAST_BYTE_DISTANCE);
    write_bytes(writer, tag, &bytes[first..])
}

fn write_bytes(writer: &mut Writer, tag: EncodedValueTag, bytes: &[u8]) -> Result<()> {
    let argument = encoded_argument(tag, bytes.len())?;
    header(writer, tag, argument);
    writer.bytes(bytes);
    Ok(())
}

fn header(writer: &mut Writer, tag: EncodedValueTag, argument: EncodedValueArgument) {
    writer.u8((tag.byte() & ENCODED_VALUE_TAG_MASK) | argument.header_bits());
}

fn encoded_argument(tag: EncodedValueTag, length: usize) -> Result<EncodedValueArgument> {
    let maximum = tag.maximum_argument();
    EncodedValueArgument::from_payload_width(length)
        .filter(|argument| *argument <= maximum)
        .ok_or_else(|| {
            Error::invalid_assembly(format!(
                "encoded-value type 0x{:02x} width exceeds its typed maximum {}",
                tag.byte(),
                maximum.payload_width()
            ))
        })
}

fn require_depth(depth: usize) -> Result<()> {
    if depth <= MAX_ENCODED_VALUE_DEPTH {
        Ok(())
    } else {
        Err(Error::invalid_assembly(
            "encoded-value nesting exceeds its typed maximum",
        ))
    }
}

use std::io::Read;

use flate2::read::ZlibDecoder;

use crate::classfile::encode_modified_utf8;
use crate::{Error, Result};

use super::{COMPRESSED_RESOURCE_HEADER_SIZE, COMPRESSED_RESOURCE_MAGIC, JimageEntry, JimageFile};

const ZIP_DECOMPRESSOR_NAME: &str = "zip";
const COMPACT_CP_DECOMPRESSOR_NAME: &str = "compact-cp";
const EXTERNALIZED_STRING_TAG: u8 = 23;
const EXTERNALIZED_DESCRIPTOR_TAG: u8 = 25;
const CONSTANT_UTF8_TAG: u8 = 1;
const CONSTANT_INTEGER_TAG: u8 = 3;
const CONSTANT_FLOAT_TAG: u8 = 4;
const CONSTANT_LONG_TAG: u8 = 5;
const CONSTANT_DOUBLE_TAG: u8 = 6;
const CONSTANT_CLASS_TAG: u8 = 7;
const CONSTANT_STRING_TAG: u8 = 8;
const CONSTANT_FIELD_REF_TAG: u8 = 9;
const CONSTANT_METHOD_REF_TAG: u8 = 10;
const CONSTANT_INTERFACE_METHOD_REF_TAG: u8 = 11;
const CONSTANT_NAME_AND_TYPE_TAG: u8 = 12;
const CONSTANT_METHOD_HANDLE_TAG: u8 = 15;
const CONSTANT_METHOD_TYPE_TAG: u8 = 16;
const CONSTANT_DYNAMIC_TAG: u8 = 17;
const CONSTANT_INVOKE_DYNAMIC_TAG: u8 = 18;
const CONSTANT_MODULE_TAG: u8 = 19;
const CONSTANT_PACKAGE_TAG: u8 = 20;
const CLASS_PREFIX_SIZE: usize = 8;
const COMPRESSED_INDEX_FLAG: u8 = 0x80;
const COMPRESSED_INDEX_LENGTH_SHIFT: u32 = 5;
const COMPRESSED_INDEX_LENGTH_MASK: u8 = 0x03;
const COMPRESSED_INDEX_VALUE_MASK: u8 = 0x1f;

struct LayerHeader {
    compressed_size: usize,
    uncompressed_size: usize,
    name_offset: u32,
}

pub(super) fn resource(image: &JimageFile, entry: &JimageEntry, raw: &[u8]) -> Result<Vec<u8>> {
    let mut content = raw.to_vec();
    let mut layers = 0_usize;
    while let Some(header) = layer_header(image, entry, &content)? {
        layers = layers
            .checked_add(1)
            .ok_or_else(|| invalid(entry, 0, "compression-layer count overflow"))?;
        if layers > 64 {
            return Err(invalid(entry, 0, "too many nested compression layers"));
        }
        let payload = &content[COMPRESSED_RESOURCE_HEADER_SIZE..];
        if payload.len() != header.compressed_size {
            return Err(invalid(
                entry,
                COMPRESSED_RESOURCE_HEADER_SIZE,
                format!(
                    "layer declares {} compressed bytes but contains {}",
                    header.compressed_size,
                    payload.len()
                ),
            ));
        }
        let name_units = image.string_units(header.name_offset)?;
        let name = String::from_utf16_lossy(&name_units);
        content = match name.as_str() {
            ZIP_DECOMPRESSOR_NAME => decompress_zip(entry, payload, header.uncompressed_size)?,
            COMPACT_CP_DECOMPRESSOR_NAME => {
                decompress_compact_cp(image, entry, payload, header.uncompressed_size)?
            }
            _ => {
                return Err(Error::UnsupportedJimageCompression {
                    entry: entry.name.clone(),
                    decompressor: name,
                });
            }
        };
    }
    let expected = usize::try_from(entry.uncompressed_size)
        .map_err(|_| invalid(entry, 0, "uncompressed resource size does not fit memory"))?;
    if content.len() != expected {
        return Err(invalid(
            entry,
            0,
            format!(
                "resource declares {expected} uncompressed bytes but produced {}",
                content.len()
            ),
        ));
    }
    Ok(content)
}

fn layer_header(
    image: &JimageFile,
    entry: &JimageEntry,
    content: &[u8],
) -> Result<Option<LayerHeader>> {
    if content.len() < size_of::<u32>() {
        return Ok(None);
    }
    let magic = image.header().endian.read_u32(
        content[..size_of::<u32>()]
            .try_into()
            .expect("slice width checked"),
    );
    if magic != COMPRESSED_RESOURCE_MAGIC {
        return Ok(None);
    }
    if content.len() < COMPRESSED_RESOURCE_HEADER_SIZE {
        return Err(invalid(entry, 0, "truncated compressed-resource header"));
    }
    let endian = image.header().endian;
    let compressed_size =
        usize::try_from(endian.read_u64(content[4..12].try_into().expect("slice width checked")))
            .map_err(|_| invalid(entry, 4, "compressed layer size does not fit memory"))?;
    let uncompressed_size =
        usize::try_from(endian.read_u64(content[12..20].try_into().expect("slice width checked")))
            .map_err(|_| invalid(entry, 12, "uncompressed layer size does not fit memory"))?;
    let name_offset = endian.read_u32(content[20..24].try_into().expect("slice width checked"));
    let terminal = content[28];
    if terminal > 1 {
        return Err(invalid(entry, 28, "invalid compression terminal flag"));
    }
    Ok(Some(LayerHeader {
        compressed_size,
        uncompressed_size,
        name_offset,
    }))
}

fn decompress_zip(entry: &JimageEntry, payload: &[u8], expected: usize) -> Result<Vec<u8>> {
    let mut output = Vec::with_capacity(expected);
    ZlibDecoder::new(payload)
        .read_to_end(&mut output)
        .map_err(|error| invalid(entry, COMPRESSED_RESOURCE_HEADER_SIZE, error.to_string()))?;
    if output.len() != expected {
        return Err(invalid(
            entry,
            COMPRESSED_RESOURCE_HEADER_SIZE,
            format!(
                "zip layer declares {expected} bytes but produced {}",
                output.len()
            ),
        ));
    }
    Ok(output)
}

fn decompress_compact_cp(
    image: &JimageFile,
    entry: &JimageEntry,
    payload: &[u8],
    expected: usize,
) -> Result<Vec<u8>> {
    let mut reader = BigEndianReader::new(entry, payload);
    let mut output = Vec::with_capacity(expected);
    output.extend_from_slice(reader.take(CLASS_PREFIX_SIZE)?);
    let constant_count = reader.u16()?;
    output.extend_from_slice(&constant_count.to_be_bytes());
    let mut index = 1_u16;
    while index < constant_count {
        let tag = reader.u8()?;
        match tag {
            CONSTANT_UTF8_TAG => {
                output.push(tag);
                let length = usize::from(reader.u16()?);
                write_utf_bytes(entry, &mut output, reader.take(length)?)?;
            }
            EXTERNALIZED_STRING_TAG => {
                let string_offset = reader.compressed_index()?;
                let units = image.string_units(string_offset)?;
                write_utf_units(entry, &mut output, &units)?;
            }
            EXTERNALIZED_DESCRIPTOR_TAG => {
                let units = reconstruct_descriptor(image, &mut reader)?;
                write_utf_units(entry, &mut output, &units)?;
            }
            CONSTANT_LONG_TAG | CONSTANT_DOUBLE_TAG => {
                output.push(tag);
                output.extend_from_slice(reader.take(constant_size(tag).expect("known tag"))?);
                index = index
                    .checked_add(1)
                    .ok_or_else(|| invalid(entry, reader.position, "constant index overflow"))?;
            }
            _ => {
                let size = constant_size(tag).ok_or_else(|| {
                    invalid(
                        entry,
                        reader.position.saturating_sub(1),
                        format!("invalid compact constant-pool tag {tag}"),
                    )
                })?;
                output.push(tag);
                output.extend_from_slice(reader.take(size)?);
            }
        }
        index = index
            .checked_add(1)
            .ok_or_else(|| invalid(entry, reader.position, "constant index overflow"))?;
    }
    output.extend_from_slice(reader.remaining());
    if output.len() != expected {
        return Err(invalid(
            entry,
            0,
            format!(
                "compact-cp layer declares {expected} bytes but produced {}",
                output.len()
            ),
        ));
    }
    Ok(output)
}

fn reconstruct_descriptor(
    image: &JimageFile,
    reader: &mut BigEndianReader<'_>,
) -> Result<Vec<u16>> {
    let format_offset = reader.compressed_index()?;
    let format = image.string_units(format_offset)?;
    let flow_length = usize::try_from(reader.compressed_index()?).map_err(|_| {
        invalid(
            reader.entry,
            reader.position,
            "descriptor flow size overflow",
        )
    })?;
    let flow = reader.take(flow_length)?;
    let mut flow_reader = BigEndianReader::new(reader.entry, flow);
    let mut offsets = Vec::new();
    while !flow_reader.remaining().is_empty() {
        offsets.push(flow_reader.compressed_index()?);
    }
    let expected_offsets = format
        .iter()
        .filter(|&&unit| unit == u16::from(b'L'))
        .count()
        .checked_mul(2)
        .ok_or_else(|| {
            invalid(
                reader.entry,
                reader.position,
                "descriptor argument overflow",
            )
        })?;
    if offsets.len() != expected_offsets {
        return Err(invalid(
            reader.entry,
            reader.position,
            format!(
                "descriptor format requires {expected_offsets} string offsets but has {}",
                offsets.len()
            ),
        ));
    }
    let mut offset_iter = offsets.into_iter();
    let mut result = Vec::new();
    for unit in format {
        result.push(unit);
        if unit == u16::from(b'L') {
            let package = image.string_units(offset_iter.next().expect("count checked"))?;
            if !package.is_empty() {
                result.extend_from_slice(&package);
                result.push(u16::from(b'/'));
            }
            let class = image.string_units(offset_iter.next().expect("count checked"))?;
            result.extend_from_slice(&class);
        }
    }
    Ok(result)
}

fn write_utf_units(entry: &JimageEntry, output: &mut Vec<u8>, units: &[u16]) -> Result<()> {
    let encoded = encode_modified_utf8(units);
    write_utf_bytes(entry, output, &encoded)
}

fn write_utf_bytes(entry: &JimageEntry, output: &mut Vec<u8>, encoded: &[u8]) -> Result<()> {
    let length = u16::try_from(encoded.len()).map_err(|_| {
        invalid(
            entry,
            0,
            "reconstructed modified UTF-8 constant exceeds u16",
        )
    })?;
    output.push(CONSTANT_UTF8_TAG);
    output.extend_from_slice(&length.to_be_bytes());
    output.extend_from_slice(encoded);
    Ok(())
}

const fn constant_size(tag: u8) -> Option<usize> {
    match tag {
        CONSTANT_LONG_TAG | CONSTANT_DOUBLE_TAG => Some(8),
        CONSTANT_CLASS_TAG
        | CONSTANT_STRING_TAG
        | CONSTANT_METHOD_TYPE_TAG
        | CONSTANT_MODULE_TAG
        | CONSTANT_PACKAGE_TAG => Some(2),
        CONSTANT_INTEGER_TAG
        | CONSTANT_FLOAT_TAG
        | CONSTANT_FIELD_REF_TAG
        | CONSTANT_METHOD_REF_TAG
        | CONSTANT_INTERFACE_METHOD_REF_TAG
        | CONSTANT_NAME_AND_TYPE_TAG
        | CONSTANT_DYNAMIC_TAG
        | CONSTANT_INVOKE_DYNAMIC_TAG => Some(4),
        CONSTANT_METHOD_HANDLE_TAG => Some(3),
        _ => None,
    }
}

struct BigEndianReader<'a> {
    entry: &'a JimageEntry,
    bytes: &'a [u8],
    position: usize,
}

impl<'a> BigEndianReader<'a> {
    const fn new(entry: &'a JimageEntry, bytes: &'a [u8]) -> Self {
        Self {
            entry,
            bytes,
            position: 0,
        }
    }

    fn u8(&mut self) -> Result<u8> {
        let byte = *self
            .bytes
            .get(self.position)
            .ok_or_else(|| invalid(self.entry, self.position, "truncated compact-cp payload"))?;
        self.position += 1;
        Ok(byte)
    }

    fn u16(&mut self) -> Result<u16> {
        let raw: [u8; 2] = self.take(2)?.try_into().expect("slice width checked");
        Ok(u16::from_be_bytes(raw))
    }

    fn compressed_index(&mut self) -> Result<u32> {
        let header = self.u8()?;
        let length = if header & COMPRESSED_INDEX_FLAG == 0 {
            size_of::<u32>()
        } else {
            usize::from((header >> COMPRESSED_INDEX_LENGTH_SHIFT) & COMPRESSED_INDEX_LENGTH_MASK)
        };
        if length == 0 || length > size_of::<u32>() {
            return Err(invalid(
                self.entry,
                self.position - 1,
                "invalid compressed index width",
            ));
        }
        let mut value = if header & COMPRESSED_INDEX_FLAG == 0 {
            u32::from(header)
        } else {
            u32::from(header & COMPRESSED_INDEX_VALUE_MASK)
        };
        for _ in 1..length {
            value = (value << u8::BITS) | u32::from(self.u8()?);
        }
        Ok(value)
    }

    fn take(&mut self, length: usize) -> Result<&'a [u8]> {
        let end = self
            .position
            .checked_add(length)
            .ok_or_else(|| invalid(self.entry, self.position, "compact-cp range overflow"))?;
        let result = self
            .bytes
            .get(self.position..end)
            .ok_or_else(|| invalid(self.entry, self.position, "truncated compact-cp payload"))?;
        self.position = end;
        Ok(result)
    }

    fn remaining(&mut self) -> &'a [u8] {
        let remaining = &self.bytes[self.position..];
        self.position = self.bytes.len();
        remaining
    }
}

fn invalid(entry: &JimageEntry, offset: usize, message: impl Into<String>) -> Error {
    Error::invalid_jimage(
        offset,
        format!("resource `{}`: {}", entry.name, message.into()),
    )
}

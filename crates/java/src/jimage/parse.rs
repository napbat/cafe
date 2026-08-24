use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use crate::classfile::decode_modified_utf8;
use crate::{Error, Result};

use super::{
    JIMAGE_HEADER_SIZE, JIMAGE_MAGIC, JIMAGE_MAJOR_VERSION, JIMAGE_MINOR_VERSION, JimageEndian,
    JimageEntry, JimageFile, JimageHeader,
};

const WORD_SIZE: usize = size_of::<u32>();
const ATTRIBUTE_MODULE: usize = 1;
const ATTRIBUTE_PARENT: usize = 2;
const ATTRIBUTE_BASE: usize = 3;
const ATTRIBUTE_EXTENSION: usize = 4;
const ATTRIBUTE_OFFSET: usize = 5;
const ATTRIBUTE_COMPRESSED: usize = 6;
const ATTRIBUTE_UNCOMPRESSED: usize = 7;
const ATTRIBUTE_PREVIEW_FLAGS: usize = 8;
const ATTRIBUTE_COUNT: usize = ATTRIBUTE_PREVIEW_FLAGS + 1;
const ATTRIBUTE_KIND_SHIFT: u8 = 3;
const ATTRIBUTE_LENGTH_MASK: u8 = 0x07;
const ATTRIBUTE_END_MAX: u8 = ATTRIBUTE_LENGTH_MASK;

pub(super) fn parse(bytes: Vec<u8>) -> Result<JimageFile> {
    let endian = detect_endian(&bytes)?;
    let version = read_u32(&bytes, WORD_SIZE, endian)?;
    let header = JimageHeader {
        endian,
        major_version: u16::try_from(version >> 16).expect("upper half fits"),
        minor_version: u16::try_from(version & u32::from(u16::MAX)).expect("lower half fits"),
        flags: read_u32(&bytes, WORD_SIZE * 2, endian)?,
        resource_count: read_u32(&bytes, WORD_SIZE * 3, endian)?,
        table_length: read_u32(&bytes, WORD_SIZE * 4, endian)?,
        locations_size: read_u32(&bytes, WORD_SIZE * 5, endian)?,
        strings_size: read_u32(&bytes, WORD_SIZE * 6, endian)?,
    };
    if (header.major_version, header.minor_version) != (JIMAGE_MAJOR_VERSION, JIMAGE_MINOR_VERSION)
    {
        return Err(Error::invalid_jimage(
            WORD_SIZE,
            format!(
                "unsupported version {}.{}",
                header.major_version, header.minor_version
            ),
        ));
    }

    let table_bytes = usize::try_from(header.table_length)
        .map_err(|_| Error::invalid_jimage(WORD_SIZE * 4, "table length does not fit memory"))?
        .checked_mul(WORD_SIZE)
        .ok_or_else(|| Error::invalid_jimage(WORD_SIZE * 4, "table size overflow"))?;
    let offsets_offset = JIMAGE_HEADER_SIZE
        .checked_add(table_bytes)
        .ok_or_else(|| Error::invalid_jimage(0, "redirect table range overflow"))?;
    let locations_offset = offsets_offset
        .checked_add(table_bytes)
        .ok_or_else(|| Error::invalid_jimage(0, "offset table range overflow"))?;
    let locations_size = usize::try_from(header.locations_size)
        .map_err(|_| Error::invalid_jimage(WORD_SIZE * 5, "location size does not fit memory"))?;
    let strings_offset = locations_offset
        .checked_add(locations_size)
        .ok_or_else(|| Error::invalid_jimage(0, "location section range overflow"))?;
    let strings_size = usize::try_from(header.strings_size)
        .map_err(|_| Error::invalid_jimage(WORD_SIZE * 6, "string size does not fit memory"))?;
    let index_size = strings_offset
        .checked_add(strings_size)
        .ok_or_else(|| Error::invalid_jimage(0, "string section range overflow"))?;
    if index_size > bytes.len() {
        return Err(Error::invalid_jimage(
            bytes.len(),
            format!(
                "index declares {index_size} bytes but image has {}",
                bytes.len()
            ),
        ));
    }

    let mut seen_locations = BTreeSet::new();
    let mut entries = Vec::new();
    for slot in 0..usize::try_from(header.table_length).expect("u32 fits usize") {
        let offset_position = offsets_offset + slot * WORD_SIZE;
        let location_offset = read_u32(&bytes, offset_position, endian)?;
        if location_offset == 0 || !seen_locations.insert(location_offset) {
            continue;
        }
        entries.push(parse_location(
            &bytes,
            locations_offset,
            locations_size,
            strings_offset,
            strings_size,
            location_offset,
            index_size,
        )?);
    }

    let expected_count = usize::try_from(header.resource_count)
        .map_err(|_| Error::invalid_jimage(WORD_SIZE * 3, "resource count does not fit memory"))?;
    if entries.len() != expected_count {
        return Err(Error::invalid_jimage(
            offsets_offset,
            format!(
                "header declares {expected_count} resources but the index contains {} unique locations",
                entries.len()
            ),
        ));
    }
    entries.sort_by(|left, right| left.name.cmp(&right.name));
    let mut by_name = BTreeMap::new();
    for (index, entry) in entries.iter().enumerate() {
        if by_name.insert(entry.name.clone(), index).is_some() {
            return Err(Error::invalid_jimage(
                locations_offset,
                format!("duplicate resource name `{}`", entry.name),
            ));
        }
    }

    Ok(JimageFile {
        bytes: Arc::from(bytes),
        header,
        index_size,
        strings_offset,
        strings_size,
        entries,
        by_name,
    })
}

fn detect_endian(bytes: &[u8]) -> Result<JimageEndian> {
    let magic_bytes: [u8; WORD_SIZE] = bytes
        .get(..WORD_SIZE)
        .ok_or_else(|| Error::invalid_jimage(0, "truncated JIMAGE header"))?
        .try_into()
        .expect("slice width checked");
    if u32::from_le_bytes(magic_bytes) == JIMAGE_MAGIC {
        Ok(JimageEndian::Little)
    } else if u32::from_be_bytes(magic_bytes) == JIMAGE_MAGIC {
        Ok(JimageEndian::Big)
    } else {
        Err(Error::invalid_jimage(0, "bad JIMAGE magic"))
    }
}

fn parse_location(
    bytes: &[u8],
    locations_offset: usize,
    locations_size: usize,
    strings_offset: usize,
    strings_size: usize,
    relative_offset: u32,
    index_size: usize,
) -> Result<JimageEntry> {
    let relative = usize::try_from(relative_offset).map_err(|_| {
        Error::invalid_jimage(locations_offset, "location offset does not fit memory")
    })?;
    let attributes = parse_location_attributes(bytes, locations_offset, locations_size, relative)?;
    let module = string(
        bytes,
        strings_offset,
        strings_size,
        attributes[ATTRIBUTE_MODULE],
    )?;
    let parent = string(
        bytes,
        strings_offset,
        strings_size,
        attributes[ATTRIBUTE_PARENT],
    )?;
    let base = string(
        bytes,
        strings_offset,
        strings_size,
        attributes[ATTRIBUTE_BASE],
    )?;
    let extension = string(
        bytes,
        strings_offset,
        strings_size,
        attributes[ATTRIBUTE_EXTENSION],
    )?;
    let (name, path) = resource_name(
        locations_offset + relative,
        &module,
        &parent,
        &base,
        &extension,
    )?;
    validate_resource_range(
        bytes,
        locations_offset + relative,
        index_size,
        &name,
        &attributes,
    )?;

    Ok(JimageEntry {
        name,
        module,
        path,
        offset: attributes[ATTRIBUTE_OFFSET],
        compressed_size: attributes[ATTRIBUTE_COMPRESSED],
        uncompressed_size: attributes[ATTRIBUTE_UNCOMPRESSED],
        preview_flags: attributes[ATTRIBUTE_PREVIEW_FLAGS],
    })
}

fn parse_location_attributes(
    bytes: &[u8],
    locations_offset: usize,
    locations_size: usize,
    relative: usize,
) -> Result<[u64; ATTRIBUTE_COUNT]> {
    if relative >= locations_size {
        return Err(Error::invalid_jimage(
            locations_offset,
            format!("location offset {relative} is outside the location section"),
        ));
    }
    let mut cursor = locations_offset + relative;
    let location_end = locations_offset + locations_size;
    let mut attributes = [0_u64; ATTRIBUTE_COUNT];
    let mut present = [false; ATTRIBUTE_COUNT];
    loop {
        let descriptor = *bytes
            .get(cursor)
            .filter(|_| cursor < location_end)
            .ok_or_else(|| Error::invalid_jimage(cursor, "unterminated location attributes"))?;
        cursor += 1;
        if descriptor <= ATTRIBUTE_END_MAX {
            break;
        }
        let kind = usize::from(descriptor >> ATTRIBUTE_KIND_SHIFT);
        if kind >= ATTRIBUTE_COUNT {
            return Err(Error::invalid_jimage(
                cursor - 1,
                format!("unknown location attribute kind {kind}"),
            ));
        }
        if present[kind] {
            return Err(Error::invalid_jimage(
                cursor - 1,
                format!("duplicate location attribute kind {kind}"),
            ));
        }
        let width = usize::from(descriptor & ATTRIBUTE_LENGTH_MASK) + 1;
        let end = cursor
            .checked_add(width)
            .filter(|&end| end <= location_end)
            .ok_or_else(|| Error::invalid_jimage(cursor, "truncated location attribute value"))?;
        attributes[kind] = bytes[cursor..end]
            .iter()
            .fold(0_u64, |value, &byte| (value << 8) | u64::from(byte));
        present[kind] = true;
        cursor = end;
    }

    if !present[ATTRIBUTE_BASE] || !present[ATTRIBUTE_UNCOMPRESSED] {
        return Err(Error::invalid_jimage(
            locations_offset + relative,
            "resource location lacks a base name or uncompressed size",
        ));
    }
    Ok(attributes)
}

fn resource_name(
    offset: usize,
    module: &str,
    parent: &str,
    base: &str,
    extension: &str,
) -> Result<(String, String)> {
    if base.is_empty() {
        return Err(Error::invalid_jimage(offset, "resource base name is empty"));
    }
    let mut path = String::new();
    if !parent.is_empty() {
        path.push_str(parent);
        path.push('/');
    }
    path.push_str(base);
    if !extension.is_empty() {
        path.push('.');
        path.push_str(extension);
    }
    let name = if module.is_empty() {
        format!("/{path}")
    } else {
        format!("/{module}/{path}")
    };

    Ok((name, path))
}

fn validate_resource_range(
    bytes: &[u8],
    location_offset: usize,
    index_size: usize,
    name: &str,
    attributes: &[u64; ATTRIBUTE_COUNT],
) -> Result<()> {
    let stored_size = if attributes[ATTRIBUTE_COMPRESSED] == 0 {
        attributes[ATTRIBUTE_UNCOMPRESSED]
    } else {
        attributes[ATTRIBUTE_COMPRESSED]
    };
    let start = index_size
        .checked_add(usize::try_from(attributes[ATTRIBUTE_OFFSET]).map_err(|_| {
            Error::invalid_jimage(location_offset, "resource offset does not fit memory")
        })?)
        .ok_or_else(|| Error::invalid_jimage(index_size, "resource offset overflow"))?;
    let end = start
        .checked_add(
            usize::try_from(stored_size)
                .map_err(|_| Error::invalid_jimage(start, "resource size does not fit memory"))?,
        )
        .ok_or_else(|| Error::invalid_jimage(start, "resource range overflow"))?;
    if end > bytes.len() {
        return Err(Error::invalid_jimage(
            start,
            format!("resource `{name}` range exceeds the image"),
        ));
    }

    Ok(())
}

fn string(bytes: &[u8], strings_offset: usize, strings_size: usize, offset: u64) -> Result<String> {
    let offset = u32::try_from(offset)
        .map_err(|_| Error::invalid_jimage(strings_offset, "string offset exceeds 32 bits"))?;
    let units = string_units(bytes, strings_offset, strings_size, offset)?;
    Ok(String::from_utf16_lossy(&units))
}

pub(super) fn string_units(
    bytes: &[u8],
    strings_offset: usize,
    strings_size: usize,
    offset: u32,
) -> Result<Vec<u16>> {
    let relative = usize::try_from(offset).expect("u32 fits usize");
    if relative >= strings_size {
        return Err(Error::invalid_jimage(
            strings_offset,
            format!("string offset {offset} is outside the string table"),
        ));
    }
    let start = strings_offset + relative;
    let section_end = strings_offset + strings_size;
    let nul = bytes[start..section_end]
        .iter()
        .position(|&byte| byte == 0)
        .map(|distance| start + distance)
        .ok_or_else(|| Error::invalid_jimage(start, "unterminated string-table entry"))?;
    decode_modified_utf8(&bytes[start..nul], start)
        .map(|decoded| decoded.units)
        .map_err(|error| Error::invalid_jimage(start, error.to_string()))
}

fn read_u32(bytes: &[u8], offset: usize, endian: JimageEndian) -> Result<u32> {
    let raw: [u8; WORD_SIZE] = bytes
        .get(offset..offset + WORD_SIZE)
        .ok_or_else(|| Error::invalid_jimage(offset, "truncated 32-bit field"))?
        .try_into()
        .expect("slice width checked");
    Ok(endian.read_u32(raw))
}

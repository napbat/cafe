//! Split-section `CompactDex` artifact and method inventory.

use std::collections::BTreeSet;

use super::{CompactCodeItem, CompactDexHeader, CompactDexVersion, CompactOffsetTable};
use crate::file::DexSourceFormat;
use crate::file::header::{HeaderField, Section};
use crate::file::integrity;
use crate::file::io::Reader;
use crate::file::layout::ItemWidth;
use crate::{Error, Result};

const CLASS_DATA_OFFSET_IN_DEFINITION: usize = 24;
const CLASS_DEFINITION_WIDTH: usize = 32;
const COMPACT_CHECKSUM_FACTOR: u32 = 31;
const CHECKSUM_FIELD_WIDTH: usize = 4;
const DATA_SIZE_OFFSET: usize = 0x68;
const DATA_OFFSET_OFFSET: usize = 0x6c;

/// Exact `CompactDex` main and shared-data sections.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompactDexSections {
    /// Main section containing the header and identifier tables.
    pub main: Vec<u8>,
    /// Separately addressed data section.
    pub data: Vec<u8>,
}

/// Native method and code-item coordinates discovered from class data.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct CompactMethodLocation {
    /// Native `method_id` index.
    pub method_index: u32,
    /// Exact encoded access flags.
    pub access_flags: u32,
    /// Data-section offset of the compact code item, or zero for no body.
    pub code_offset: u32,
}

/// Parsed, editable `CompactDex` split artifact.
#[derive(Debug, Clone)]
pub struct CompactDexFile {
    header: CompactDexHeader,
    main: Vec<u8>,
    data: Vec<u8>,
    original: Option<CompactDexSections>,
    dirty: bool,
}

impl CompactDexFile {
    /// Parses a contiguous representation whose header points at an embedded
    /// data section.
    ///
    /// # Errors
    ///
    /// Returns an error when the data section is external or any header,
    /// section, table, integrity, or range constraint is malformed.
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        let header = CompactDexHeader::parse(bytes)?;
        let main_end = usize::try_from(header.file_size).map_err(|_| {
            Error::invalid_dex(HeaderField::FileSize.offset(), "file size is too large")
        })?;
        let data_start = usize::try_from(header.data.offset)
            .map_err(|_| Error::invalid_dex(DATA_OFFSET_OFFSET, "data offset is too large"))?;
        let data_size = usize::try_from(header.data.size)
            .map_err(|_| Error::invalid_dex(DATA_SIZE_OFFSET, "data size is too large"))?;
        if data_size != 0 && data_start == 0 {
            return Err(Error::invalid_dex(
                DATA_OFFSET_OFFSET,
                "CompactDex data is external; use CompactDexFile::parse_sections",
            ));
        }
        let data_end = data_start
            .checked_add(data_size)
            .ok_or_else(|| Error::invalid_dex(data_start, "data section range overflowed"))?;
        let main = bytes
            .get(..main_end)
            .ok_or_else(|| Error::invalid_dex(0, "truncated CompactDex main section"))?;
        let data = if data_size == 0 {
            &[][..]
        } else {
            bytes.get(data_start..data_end).ok_or_else(|| {
                Error::invalid_dex(data_start, "truncated CompactDex data section")
            })?
        };
        Self::parse_sections(main, data)
    }

    /// Parses explicit main and shared-data sections, as stored by VDEX.
    ///
    /// # Errors
    ///
    /// Returns an error for unsupported versions, malformed coordinates,
    /// invalid checksums, or truncated identifier and data sections.
    pub fn parse_sections(main: &[u8], data: &[u8]) -> Result<Self> {
        let header = CompactDexHeader::parse(main)?;
        validate_sections(&header, main, data)?;
        validate_checksum(&header, main, data)?;
        let sections = CompactDexSections {
            main: main.to_vec(),
            data: data.to_vec(),
        };
        Ok(Self {
            header,
            main: sections.main.clone(),
            data: sections.data.clone(),
            original: Some(sections),
            dirty: false,
        })
    }

    /// Creates a `CompactDex` artifact from a typed header, main payload bytes,
    /// and a shared data section.
    ///
    /// Header sizes and integrity fields are regenerated.
    ///
    /// # Errors
    ///
    /// Returns an error when lengths exceed 32 bits or coordinates do not fit
    /// the supplied sections.
    pub fn from_parts(
        mut header: CompactDexHeader,
        main_payload: impl AsRef<[u8]>,
        data: Vec<u8>,
    ) -> Result<Self> {
        let header_size = usize::try_from(super::COMPACT_DEX_HEADER_SIZE)
            .map_err(|_| Error::invalid_assembly("CompactDex header size is not representable"))?;
        let mut main = vec![0; header_size];
        main.extend_from_slice(main_payload.as_ref());
        header.version = CompactDexVersion::V001;
        header.header_size = super::COMPACT_DEX_HEADER_SIZE;
        header.file_size = u32::try_from(main.len())
            .map_err(|_| Error::invalid_assembly("CompactDex main section exceeds 32 bits"))?;
        header.data.size = u32::try_from(data.len())
            .map_err(|_| Error::invalid_assembly("CompactDex data section exceeds 32 bits"))?;
        header.checksum = 0;
        header.write_into(&mut main)?;
        header.checksum = calculate_checksum(&main, &data)?;
        header.write_into(&mut main)?;
        validate_sections(&header, &main, &data)?;
        Ok(Self {
            header,
            main,
            data,
            original: None,
            dirty: true,
        })
    }

    /// Returns the explicit physical source identity.
    #[must_use]
    pub const fn source_format(&self) -> DexSourceFormat {
        DexSourceFormat::Compact(self.header.version)
    }

    /// Returns the parsed `CompactDex` header.
    #[must_use]
    pub const fn header(&self) -> &CompactDexHeader {
        &self.header
    }

    /// Returns an editable header and marks the artifact dirty.
    pub fn header_mut(&mut self) -> &mut CompactDexHeader {
        self.dirty = true;
        &mut self.header
    }

    /// Returns the exact main section.
    #[must_use]
    pub fn main_section(&self) -> &[u8] {
        &self.main
    }

    /// Returns editable bytes after the fixed header.
    pub fn main_payload_mut(&mut self) -> &mut [u8] {
        self.dirty = true;
        &mut self.main[super::COMPACT_DEX_HEADER_SIZE as usize..]
    }

    /// Returns the exact shared data section.
    #[must_use]
    pub fn data_section(&self) -> &[u8] {
        &self.data
    }

    /// Returns editable shared-data bytes.
    pub fn data_section_mut(&mut self) -> &mut [u8] {
        self.dirty = true;
        &mut self.data
    }

    /// Returns whether no mutating API has been used since parsing.
    #[must_use]
    pub const fn is_pristine(&self) -> bool {
        !self.dirty
    }

    /// Assembles both physical sections, preserving pristine input exactly.
    ///
    /// # Errors
    ///
    /// Returns an error when an edited header or section is inconsistent.
    pub fn to_sections(&self) -> Result<CompactDexSections> {
        if !self.dirty
            && let Some(original) = &self.original
        {
            return Ok(original.clone());
        }
        let mut header = self.header.clone();
        let mut main = self.main.clone();
        header.file_size = u32::try_from(main.len())
            .map_err(|_| Error::invalid_assembly("CompactDex main section exceeds 32 bits"))?;
        header.data.size = u32::try_from(self.data.len())
            .map_err(|_| Error::invalid_assembly("CompactDex data section exceeds 32 bits"))?;
        header.checksum = 0;
        header.write_into(&mut main)?;
        header.checksum = calculate_checksum(&main, &self.data)?;
        header.write_into(&mut main)?;
        validate_sections(&header, &main, &self.data)?;
        Ok(CompactDexSections {
            main,
            data: self.data.clone(),
        })
    }

    /// Applies a checked edit and restores the previous artifact if complete
    /// assembly or reparsing fails.
    ///
    /// # Errors
    ///
    /// Returns the edit or validation error.
    pub fn try_edit<T>(&mut self, edit: impl FnOnce(&mut Self) -> Result<T>) -> Result<T> {
        let previous = self.clone();
        let result = edit(self).and_then(|value| {
            let sections = self.to_sections()?;
            Self::parse_sections(&sections.main, &sections.data)?;
            Ok(value)
        });
        if result.is_err() {
            *self = previous;
        }
        result
    }

    /// Decodes the compact debug-info offset for one native method index.
    ///
    /// # Errors
    ///
    /// Returns an error when the compressed table is malformed.
    pub fn debug_info_offset(&self, method_index: u32) -> Result<u32> {
        if method_index >= self.header.method_ids.size {
            return Err(Error::invalid_dex(
                self.header.debug_info_offsets_pos as usize,
                format!("method index {method_index} is outside the debug-offset table"),
            ));
        }
        let start = usize::try_from(self.header.debug_info_offsets_pos).map_err(|_| {
            Error::invalid_dex(0, "debug-offset data position does not fit platform")
        })?;
        let data = self.data.get(start..).ok_or_else(|| {
            Error::invalid_dex(start, "debug-offset data begins outside the data section")
        })?;
        let table = CompactOffsetTable::parse(
            data,
            self.header.debug_info_base,
            self.header.debug_info_offsets_table_offset,
            self.header.method_ids.size as usize,
            self.header.endian,
        )?;
        table.get(method_index).ok_or_else(|| {
            Error::invalid_dex(start, "debug-offset table is missing the requested method")
        })
    }

    /// Inventories every encoded method and its optional compact code item.
    ///
    /// # Errors
    ///
    /// Returns an error for malformed class-data LEBs, duplicate method
    /// indices, invalid table indices, or out-of-range code coordinates.
    pub fn methods(&self) -> Result<Vec<CompactMethodLocation>> {
        let reader = Reader::new(&self.main, self.header.endian);
        let data_reader = Reader::new(&self.data, self.header.endian);
        let class_offset = checked_section(
            self.header.class_defs,
            CLASS_DEFINITION_WIDTH,
            self.main.len(),
            "class definitions",
        )?;
        let mut methods = Vec::new();
        let mut seen = BTreeSet::new();
        for class_index in 0..self.header.class_defs.size as usize {
            let item = class_offset + class_index * CLASS_DEFINITION_WIDTH;
            let class_data_offset = reader.u32(item + CLASS_DATA_OFFSET_IN_DEFINITION)?;
            if class_data_offset == 0 {
                continue;
            }
            let offset = usize::try_from(class_data_offset)
                .map_err(|_| Error::invalid_dex(item, "class-data offset is too large"))?;
            let mut cursor = data_reader.cursor(offset)?;
            let static_fields = cursor.uleb128()?;
            let instance_fields = cursor.uleb128()?;
            let direct_methods = cursor.uleb128()?;
            let virtual_methods = cursor.uleb128()?;
            skip_fields(&mut cursor, static_fields)?;
            skip_fields(&mut cursor, instance_fields)?;
            read_methods(
                &mut cursor,
                direct_methods,
                self.header.method_ids.size,
                &mut seen,
                &mut methods,
            )?;
            read_methods(
                &mut cursor,
                virtual_methods,
                self.header.method_ids.size,
                &mut seen,
                &mut methods,
            )?;
        }
        methods.sort_unstable();
        Ok(methods)
    }

    /// Decodes one compact code item into canonical DEX instructions and
    /// exception metadata.
    ///
    /// # Errors
    ///
    /// Returns an error for malformed compact headers, instructions, tries,
    /// handlers, or type references.
    pub fn decode_code_item(&self, code_offset: u32) -> Result<CompactCodeItem> {
        super::decode_code_item(
            &self.data,
            code_offset,
            self.header.endian,
            self.header.type_ids.size,
        )
    }
}

fn validate_sections(header: &CompactDexHeader, main: &[u8], data: &[u8]) -> Result<()> {
    if usize::try_from(header.file_size).ok() != Some(main.len()) {
        return Err(Error::invalid_dex(
            HeaderField::FileSize.offset(),
            format!(
                "CompactDex main size {} does not match {} bytes",
                header.file_size,
                main.len()
            ),
        ));
    }
    if usize::try_from(header.data.size).ok() != Some(data.len()) {
        return Err(Error::invalid_dex(
            DATA_SIZE_OFFSET,
            format!(
                "CompactDex data size {} does not match {} bytes",
                header.data.size,
                data.len()
            ),
        ));
    }
    validate_fixed_sections(header, main.len())?;
    if header.owned_data_begin > header.owned_data_end
        || usize::try_from(header.owned_data_end).map_or(true, |end| end > data.len())
    {
        return Err(Error::invalid_dex(
            0x80,
            "CompactDex owned-data range is reversed or outside the data section",
        ));
    }
    if header.map_off != 0
        && usize::try_from(header.map_off).map_or(true, |offset| offset >= data.len())
    {
        return Err(Error::invalid_dex(
            HeaderField::MapOffset.offset(),
            "CompactDex map offset is outside the data section",
        ));
    }
    if header.debug_info_offsets_pos != 0
        && usize::try_from(header.debug_info_offsets_pos)
            .map_or(true, |offset| offset >= data.len())
    {
        return Err(Error::invalid_dex(
            0x74,
            "CompactDex debug-offset data is outside the data section",
        ));
    }
    Ok(())
}

fn validate_fixed_sections(header: &CompactDexHeader, main_size: usize) -> Result<()> {
    let sections = [
        (
            header.string_ids,
            ItemWidth::STRING_ID.bytes(),
            "string identifiers",
        ),
        (
            header.type_ids,
            ItemWidth::TYPE_ID.bytes(),
            "type identifiers",
        ),
        (
            header.proto_ids,
            ItemWidth::PROTOTYPE_ID.bytes(),
            "prototype identifiers",
        ),
        (
            header.field_ids,
            ItemWidth::FIELD_ID.bytes(),
            "field identifiers",
        ),
        (
            header.method_ids,
            ItemWidth::METHOD_ID.bytes(),
            "method identifiers",
        ),
        (
            header.class_defs,
            ItemWidth::CLASS_DEFINITION.bytes(),
            "class definitions",
        ),
    ];
    for (section, width, name) in sections {
        let _ = checked_section(section, width, main_size, name)?;
    }
    Ok(())
}

fn checked_section(section: Section, width: usize, limit: usize, name: &str) -> Result<usize> {
    if section.size == 0 {
        if section.offset == 0 {
            return Ok(0);
        }
        return Err(Error::invalid_dex(
            section.offset as usize,
            format!("empty CompactDex {name} section has a nonzero offset"),
        ));
    }
    if !section.offset.is_multiple_of(4) {
        return Err(Error::invalid_dex(
            section.offset as usize,
            format!("CompactDex {name} section is not word aligned"),
        ));
    }
    let offset = usize::try_from(section.offset)
        .map_err(|_| Error::invalid_dex(0, format!("CompactDex {name} offset is too large")))?;
    let length = usize::try_from(section.size)
        .ok()
        .and_then(|count| count.checked_mul(width))
        .ok_or_else(|| Error::invalid_dex(offset, format!("CompactDex {name} size overflowed")))?;
    let end = offset
        .checked_add(length)
        .ok_or_else(|| Error::invalid_dex(offset, format!("CompactDex {name} range overflowed")))?;
    if end > limit {
        return Err(Error::invalid_dex(
            offset,
            format!("CompactDex {name} section is truncated"),
        ));
    }
    Ok(offset)
}

fn validate_checksum(header: &CompactDexHeader, main: &[u8], data: &[u8]) -> Result<()> {
    let actual = calculate_checksum(main, data)?;
    if header.checksum == actual {
        Ok(())
    } else {
        Err(Error::invalid_dex(
            HeaderField::Checksum.offset(),
            format!(
                "CompactDex checksum mismatch: stored 0x{:08x}, calculated 0x{actual:08x}",
                header.checksum
            ),
        ))
    }
}

fn calculate_checksum(main: &[u8], data: &[u8]) -> Result<u32> {
    let header_size = super::COMPACT_DEX_HEADER_SIZE as usize;
    let mut header = main
        .get(..header_size)
        .ok_or_else(|| Error::invalid_dex(0, "truncated CompactDex header"))?
        .to_vec();
    header[HeaderField::Checksum.offset()..HeaderField::Checksum.offset() + CHECKSUM_FIELD_WIDTH]
        .fill(0);
    header[DATA_SIZE_OFFSET..DATA_SIZE_OFFSET + CHECKSUM_FIELD_WIDTH].fill(0);
    header[DATA_OFFSET_OFFSET..DATA_OFFSET_OFFSET + CHECKSUM_FIELD_WIDTH].fill(0);
    let mut checksum = integrity::adler32(&header);
    checksum =
        checksum.wrapping_mul(COMPACT_CHECKSUM_FACTOR) ^ integrity::adler32(&main[header_size..]);
    checksum = checksum.wrapping_mul(COMPACT_CHECKSUM_FACTOR) ^ integrity::adler32(data);
    Ok(checksum)
}

fn skip_fields(cursor: &mut crate::file::io::Cursor<'_>, count: u32) -> Result<()> {
    for _ in 0..count {
        cursor.uleb128()?;
        cursor.uleb128()?;
    }
    Ok(())
}

fn read_methods(
    cursor: &mut crate::file::io::Cursor<'_>,
    count: u32,
    method_limit: u32,
    seen: &mut BTreeSet<u32>,
    output: &mut Vec<CompactMethodLocation>,
) -> Result<()> {
    let mut previous = None;
    for _ in 0..count {
        let offset = cursor.position();
        let delta = cursor.uleb128()?;
        let method_index = previous
            .map_or(Some(delta), |previous: u32| previous.checked_add(delta))
            .ok_or_else(|| Error::invalid_dex(offset, "CompactDex method index overflowed"))?;
        if method_index >= method_limit {
            return Err(Error::invalid_dex(
                offset,
                format!("CompactDex method index {method_index} is out of bounds"),
            ));
        }
        if !seen.insert(method_index) {
            return Err(Error::invalid_dex(
                offset,
                format!("CompactDex method index {method_index} is duplicated"),
            ));
        }
        output.push(CompactMethodLocation {
            method_index,
            access_flags: cursor.uleb128()?,
            code_offset: cursor.uleb128()?,
        });
        previous = Some(method_index);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::CompactDexFile;
    use crate::file::compact::{
        COMPACT_DEX_HEADER_SIZE, CompactDexFeatureFlags, CompactDexHeader, CompactDexVersion,
    };
    use crate::file::header::SIGNATURE_SIZE;
    use crate::file::{Endian, Section};

    fn empty_header() -> CompactDexHeader {
        CompactDexHeader {
            version: CompactDexVersion::V001,
            checksum: 0,
            signature: [0; SIGNATURE_SIZE],
            file_size: COMPACT_DEX_HEADER_SIZE,
            header_size: COMPACT_DEX_HEADER_SIZE,
            endian: Endian::Little,
            link_size: 0,
            link_off: 0,
            map_off: 0,
            string_ids: Section::default(),
            type_ids: Section::default(),
            proto_ids: Section::default(),
            field_ids: Section::default(),
            method_ids: Section::default(),
            class_defs: Section::default(),
            data: Section::default(),
            feature_flags: CompactDexFeatureFlags::DEFAULT_METHODS,
            debug_info_offsets_pos: 0,
            debug_info_offsets_table_offset: 0,
            debug_info_base: 0,
            owned_data_begin: 0,
            owned_data_end: 0,
        }
    }

    #[test]
    fn split_sections_round_trip_exactly() {
        let built =
            CompactDexFile::from_parts(empty_header(), vec![1, 2, 3, 4], Vec::new()).unwrap();
        let sections = built.to_sections().unwrap();
        let parsed = CompactDexFile::parse_sections(&sections.main, &sections.data).unwrap();
        assert_eq!(parsed.source_format(), built.source_format());
        assert_eq!(parsed.to_sections().unwrap(), sections);
        assert!(parsed.is_pristine());
    }

    #[test]
    fn checksum_covers_shared_data() {
        let mut header = empty_header();
        header.owned_data_end = 4;
        let built = CompactDexFile::from_parts(header, Vec::new(), vec![1, 2, 3, 4]).unwrap();
        let mut sections = built.to_sections().unwrap();
        sections.data[2] ^= 0xff;
        assert!(CompactDexFile::parse_sections(&sections.main, &sections.data).is_err());
    }
}

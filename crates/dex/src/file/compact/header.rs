//! `CompactDex` header fields and binary codec.

use crate::file::header::{
    ENDIAN_CONSTANT, Endian, HeaderField, REVERSE_ENDIAN_CONSTANT, SECTION_OFFSET_DELTA,
    SIGNATURE_SIZE, Section,
};
use crate::file::io::Reader;
use crate::{Error, Result};

/// Complete `CompactDex` 001 magic.
pub const COMPACT_DEX_MAGIC: &[u8; 8] = b"cdex001\0";
/// Header width used by `CompactDex` 001.
pub const COMPACT_DEX_HEADER_SIZE: u32 = 0x88;

const FEATURE_FLAGS_OFFSET: usize = 0x70;
const DEBUG_INFO_OFFSETS_POS_OFFSET: usize = 0x74;
const DEBUG_INFO_TABLE_OFFSET_OFFSET: usize = 0x78;
const DEBUG_INFO_BASE_OFFSET: usize = 0x7c;
const OWNED_DATA_BEGIN_OFFSET: usize = 0x80;
const OWNED_DATA_END_OFFSET: usize = 0x84;

/// Supported `CompactDex` encoding version.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum CompactDexVersion {
    /// ART's `cdex001` encoding.
    V001,
}

impl CompactDexVersion {
    /// Every `CompactDex` version accepted by this crate.
    pub const ALL: &[Self] = &[Self::V001];

    /// Returns the complete on-disk magic for this version.
    #[must_use]
    pub const fn magic(self) -> &'static [u8; 8] {
        match self {
            Self::V001 => COMPACT_DEX_MAGIC,
        }
    }
}

/// `CompactDex` feature bits retained from the header.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct CompactDexFeatureFlags(u32);

impl CompactDexFeatureFlags {
    /// The artifact may contain default interface methods.
    pub const DEFAULT_METHODS: Self = Self(0x0000_0001);

    /// Retains all known and unknown feature bits.
    #[must_use]
    pub const fn from_bits_retain(bits: u32) -> Self {
        Self(bits)
    }

    /// Returns the exact encoded feature bits.
    #[must_use]
    pub const fn bits(self) -> u32 {
        self.0
    }

    /// Returns whether all requested flags are present.
    #[must_use]
    pub const fn contains(self, flags: Self) -> bool {
        self.0 & flags.0 == flags.0
    }
}

impl std::ops::BitOr for CompactDexFeatureFlags {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self::Output {
        Self(self.0 | rhs.0)
    }
}

/// Parsed `CompactDex` 001 header retaining every native coordinate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompactDexHeader {
    /// `CompactDex` version.
    pub version: CompactDexVersion,
    /// ART's stored split-section checksum.
    pub checksum: u32,
    /// Opaque inherited DEX signature bytes.
    pub signature: [u8; SIGNATURE_SIZE],
    /// Main-section byte size.
    pub file_size: u32,
    /// Encoded header size.
    pub header_size: u32,
    /// Declared byte order.
    pub endian: Endian,
    /// Static-link data size.
    pub link_size: u32,
    /// Static-link data offset in the data address space.
    pub link_off: u32,
    /// Map-list offset in the data address space.
    pub map_off: u32,
    /// String identifier count and main-section offset.
    pub string_ids: Section,
    /// Type identifier count and main-section offset.
    pub type_ids: Section,
    /// Prototype identifier count and main-section offset.
    pub proto_ids: Section,
    /// Field identifier count and main-section offset.
    pub field_ids: Section,
    /// Method identifier count and main-section offset.
    pub method_ids: Section,
    /// Class-definition count and main-section offset.
    pub class_defs: Section,
    /// Shared data-section size and inherited offset field.
    pub data: Section,
    /// Format feature flags, including unknown bits.
    pub feature_flags: CompactDexFeatureFlags,
    /// Start of compressed debug-offset blocks in the data section.
    pub debug_info_offsets_pos: u32,
    /// Offset of the debug block index relative to `debug_info_offsets_pos`.
    pub debug_info_offsets_table_offset: u32,
    /// Minimum nonzero debug-info offset.
    pub debug_info_base: u32,
    /// First byte in the shared data section owned by this member.
    pub owned_data_begin: u32,
    /// Exclusive end byte in the shared data section owned by this member.
    pub owned_data_end: u32,
}

impl CompactDexHeader {
    pub(super) fn parse(bytes: &[u8]) -> Result<Self> {
        let little = Reader::new(bytes, Endian::Little);
        let magic = little.bytes(HeaderField::Magic.offset(), COMPACT_DEX_MAGIC.len())?;
        if magic != COMPACT_DEX_MAGIC {
            return Err(Error::invalid_dex(0, "invalid CompactDex magic or version"));
        }
        let raw_endian = little.u32(HeaderField::EndianTag.offset())?;
        let endian = match raw_endian {
            ENDIAN_CONSTANT => Endian::Little,
            REVERSE_ENDIAN_CONSTANT => Endian::Reverse,
            _ => {
                return Err(Error::invalid_dex(
                    HeaderField::EndianTag.offset(),
                    format!("invalid CompactDex endian tag 0x{raw_endian:08x}"),
                ));
            }
        };
        let reader = Reader::new(bytes, endian);
        let header_size = reader.u32(HeaderField::HeaderSize.offset())?;
        if header_size != COMPACT_DEX_HEADER_SIZE {
            return Err(Error::invalid_dex(
                HeaderField::HeaderSize.offset(),
                format!(
                    "CompactDex 001 requires header size 0x{COMPACT_DEX_HEADER_SIZE:x}, found 0x{header_size:x}"
                ),
            ));
        }
        reader.bytes(0, COMPACT_DEX_HEADER_SIZE as usize)?;
        let signature = reader
            .bytes(HeaderField::Signature.offset(), SIGNATURE_SIZE)?
            .try_into()
            .map_err(|_| {
                Error::invalid_dex(HeaderField::Signature.offset(), "truncated signature")
            })?;
        Ok(Self {
            version: CompactDexVersion::V001,
            checksum: reader.u32(HeaderField::Checksum.offset())?,
            signature,
            file_size: reader.u32(HeaderField::FileSize.offset())?,
            header_size,
            endian,
            link_size: reader.u32(HeaderField::LinkSize.offset())?,
            link_off: reader.u32(HeaderField::LinkOffset.offset())?,
            map_off: reader.u32(HeaderField::MapOffset.offset())?,
            string_ids: section(reader, HeaderField::StringIds)?,
            type_ids: section(reader, HeaderField::TypeIds)?,
            proto_ids: section(reader, HeaderField::PrototypeIds)?,
            field_ids: section(reader, HeaderField::FieldIds)?,
            method_ids: section(reader, HeaderField::MethodIds)?,
            class_defs: section(reader, HeaderField::ClassDefinitions)?,
            data: section(reader, HeaderField::Data)?,
            feature_flags: CompactDexFeatureFlags::from_bits_retain(
                reader.u32(FEATURE_FLAGS_OFFSET)?,
            ),
            debug_info_offsets_pos: reader.u32(DEBUG_INFO_OFFSETS_POS_OFFSET)?,
            debug_info_offsets_table_offset: reader.u32(DEBUG_INFO_TABLE_OFFSET_OFFSET)?,
            debug_info_base: reader.u32(DEBUG_INFO_BASE_OFFSET)?,
            owned_data_begin: reader.u32(OWNED_DATA_BEGIN_OFFSET)?,
            owned_data_end: reader.u32(OWNED_DATA_END_OFFSET)?,
        })
    }

    pub(super) fn write_into(&self, bytes: &mut [u8]) -> Result<()> {
        let header = bytes
            .get_mut(..COMPACT_DEX_HEADER_SIZE as usize)
            .ok_or_else(|| {
                Error::invalid_assembly("CompactDex main section is shorter than its header")
            })?;
        header[..COMPACT_DEX_MAGIC.len()].copy_from_slice(self.version.magic());
        write_u32(
            header,
            HeaderField::Checksum.offset(),
            self.checksum,
            self.endian,
        )?;
        header[HeaderField::Signature.offset()..HeaderField::Signature.offset() + SIGNATURE_SIZE]
            .copy_from_slice(&self.signature);
        write_u32(
            header,
            HeaderField::FileSize.offset(),
            self.file_size,
            self.endian,
        )?;
        write_u32(
            header,
            HeaderField::HeaderSize.offset(),
            self.header_size,
            self.endian,
        )?;
        write_u32(
            header,
            HeaderField::EndianTag.offset(),
            match self.endian {
                Endian::Little => ENDIAN_CONSTANT,
                Endian::Reverse => REVERSE_ENDIAN_CONSTANT,
            },
            self.endian,
        )?;
        write_u32(
            header,
            HeaderField::LinkSize.offset(),
            self.link_size,
            self.endian,
        )?;
        write_u32(
            header,
            HeaderField::LinkOffset.offset(),
            self.link_off,
            self.endian,
        )?;
        write_u32(
            header,
            HeaderField::MapOffset.offset(),
            self.map_off,
            self.endian,
        )?;
        write_section(header, HeaderField::StringIds, self.string_ids, self.endian)?;
        write_section(header, HeaderField::TypeIds, self.type_ids, self.endian)?;
        write_section(
            header,
            HeaderField::PrototypeIds,
            self.proto_ids,
            self.endian,
        )?;
        write_section(header, HeaderField::FieldIds, self.field_ids, self.endian)?;
        write_section(header, HeaderField::MethodIds, self.method_ids, self.endian)?;
        write_section(
            header,
            HeaderField::ClassDefinitions,
            self.class_defs,
            self.endian,
        )?;
        write_section(header, HeaderField::Data, self.data, self.endian)?;
        write_compact_fields(header, self)?;
        Ok(())
    }
}

fn write_compact_fields(bytes: &mut [u8], header: &CompactDexHeader) -> Result<()> {
    for (offset, value) in [
        (FEATURE_FLAGS_OFFSET, header.feature_flags.bits()),
        (DEBUG_INFO_OFFSETS_POS_OFFSET, header.debug_info_offsets_pos),
        (
            DEBUG_INFO_TABLE_OFFSET_OFFSET,
            header.debug_info_offsets_table_offset,
        ),
        (DEBUG_INFO_BASE_OFFSET, header.debug_info_base),
        (OWNED_DATA_BEGIN_OFFSET, header.owned_data_begin),
        (OWNED_DATA_END_OFFSET, header.owned_data_end),
    ] {
        write_u32(bytes, offset, value, header.endian)?;
    }
    Ok(())
}

fn section(reader: Reader<'_>, field: HeaderField) -> Result<Section> {
    Ok(Section {
        size: reader.u32(field.offset())?,
        offset: reader.u32(field.offset() + SECTION_OFFSET_DELTA)?,
    })
}

fn write_section(
    bytes: &mut [u8],
    field: HeaderField,
    section: Section,
    endian: Endian,
) -> Result<()> {
    write_u32(bytes, field.offset(), section.size, endian)?;
    write_u32(
        bytes,
        field.offset() + SECTION_OFFSET_DELTA,
        section.offset,
        endian,
    )
}

fn write_u32(bytes: &mut [u8], offset: usize, value: u32, endian: Endian) -> Result<()> {
    let encoded = match endian {
        Endian::Little => value.to_le_bytes(),
        Endian::Reverse => value.to_be_bytes(),
    };
    bytes
        .get_mut(offset..offset + encoded.len())
        .ok_or_else(|| Error::invalid_assembly("CompactDex header field is out of bounds"))?
        .copy_from_slice(&encoded);
    Ok(())
}

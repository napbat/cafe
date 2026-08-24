//! DEX header values and supported format versions.

/// Standard little-endian DEX marker.
pub const ENDIAN_CONSTANT: u32 = 0x1234_5678;
/// Marker found in a byte-swapped DEX file.
pub const REVERSE_ENDIAN_CONSTANT: u32 = 0x7856_3412;
/// Header width used through DEX version 040.
pub const LEGACY_HEADER_SIZE: u32 = 0x70;
/// Header width used by DEX version 041 containers.
pub const CONTAINER_HEADER_SIZE: u32 = 0x78;
/// Sentinel for an absent table index.
pub const NO_INDEX: u32 = u32::MAX;
/// Fixed DEX magic width in bytes.
pub(crate) const MAGIC_SIZE: usize = 8;
/// Fixed DEX SHA-1 signature width in bytes.
pub const SIGNATURE_SIZE: usize = 20;
/// Format-identifying prefix before the three version digits.
pub(crate) const MAGIC_PREFIX: &[u8; 4] = b"dex\n";
/// Width of the decimal format version embedded in the magic.
pub(crate) const MAGIC_VERSION_SIZE: usize = 3;
/// Index of the terminating zero byte in the magic.
pub(crate) const MAGIC_TERMINATOR_INDEX: usize = MAGIC_SIZE - 1;
/// Required value of the terminating magic byte.
pub(crate) const MAGIC_TERMINATOR: u8 = 0;
/// Width between a section's size field and its offset field.
pub(crate) const SECTION_OFFSET_DELTA: usize = size_of::<u32>();
/// Width between section fields for writer offsets represented as `u32`.
pub(crate) const SECTION_OFFSET_DELTA_U32: u32 = u32::BITS / u8::BITS;
/// Canonical value for an absent optional offset or empty section coordinate.
pub(crate) const ABSENT_OFFSET: u32 = 0;
/// Physical header position used by versions 035 through 040.
pub(crate) const LEGACY_HEADER_OFFSET: usize = 0;

/// Byte offsets of fields within every supported DEX header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(usize)]
pub(crate) enum HeaderField {
    Magic = 0,
    Checksum = 8,
    Signature = 12,
    FileSize = 32,
    HeaderSize = 36,
    EndianTag = 40,
    LinkSize = 44,
    LinkOffset = 48,
    MapOffset = 52,
    StringIds = 56,
    TypeIds = 64,
    PrototypeIds = 72,
    FieldIds = 80,
    MethodIds = 88,
    ClassDefinitions = 96,
    Data = 104,
    ContainerSize = 112,
    HeaderOffset = 116,
}

impl HeaderField {
    pub(crate) const fn offset(self) -> usize {
        self as usize
    }
}

/// Supported DEX file-format version.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum DexVersion {
    /// Android's long-lived original production format.
    V035,
    /// Android 7 default-interface semantics.
    V037,
    /// Android 8 method handles and custom/polymorphic invocation.
    V038,
    /// Android 9 method-handle and method-type constants.
    V039,
    /// Android 10 expanded simple-name grammar.
    V040,
    /// Android 16 experimental multi-header container format.
    V041,
}

impl DexVersion {
    /// Every accepted modern DEX version in numeric order.
    pub const ALL: &[Self] = &[
        Self::V035,
        Self::V037,
        Self::V038,
        Self::V039,
        Self::V040,
        Self::V041,
    ];

    /// Parses the three decimal version bytes from the DEX magic.
    #[must_use]
    pub const fn from_digits(digits: [u8; 3]) -> Option<Self> {
        match digits {
            [b'0', b'3', b'5'] => Some(Self::V035),
            [b'0', b'3', b'7'] => Some(Self::V037),
            [b'0', b'3', b'8'] => Some(Self::V038),
            [b'0', b'3', b'9'] => Some(Self::V039),
            [b'0', b'4', b'0'] => Some(Self::V040),
            [b'0', b'4', b'1'] => Some(Self::V041),
            _ => None,
        }
    }

    /// Returns the three decimal bytes embedded in the DEX magic.
    #[must_use]
    pub const fn digits(self) -> [u8; 3] {
        match self {
            Self::V035 => *b"035",
            Self::V037 => *b"037",
            Self::V038 => *b"038",
            Self::V039 => *b"039",
            Self::V040 => *b"040",
            Self::V041 => *b"041",
        }
    }

    /// Returns the exact header width required by this version.
    #[must_use]
    pub const fn header_size(self) -> u32 {
        match self {
            Self::V035 | Self::V037 | Self::V038 | Self::V039 | Self::V040 => LEGACY_HEADER_SIZE,
            Self::V041 => CONTAINER_HEADER_SIZE,
        }
    }
}

/// Byte order declared by a DEX header.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Endian {
    /// Standard little-endian encoding.
    Little,
    /// Reverse-endian encoding requiring byte swapping.
    Reverse,
}

/// Parsed DEX header retaining every declared size and offset.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DexHeader {
    /// File format version.
    pub version: DexVersion,
    /// Stored Adler-32 checksum.
    pub checksum: u32,
    /// Stored SHA-1 signature.
    pub signature: [u8; SIGNATURE_SIZE],
    /// Logical file size from this header.
    pub file_size: u32,
    /// Encoded header size.
    pub header_size: u32,
    /// Declared byte order.
    pub endian: Endian,
    /// Static-link data size.
    pub link_size: u32,
    /// Static-link data offset.
    pub link_off: u32,
    /// Map-list offset.
    pub map_off: u32,
    /// String identifier count and offset.
    pub string_ids: Section,
    /// Type identifier count and offset.
    pub type_ids: Section,
    /// Prototype identifier count and offset.
    pub proto_ids: Section,
    /// Field identifier count and offset.
    pub field_ids: Section,
    /// Method identifier count and offset.
    pub method_ids: Section,
    /// Class-definition count and offset.
    pub class_defs: Section,
    /// Legacy contiguous data-area count and offset.
    pub data: Section,
    /// Physical container size for version 041, otherwise `file_size`.
    pub container_size: u32,
    /// This header's physical offset for version 041, otherwise zero.
    pub header_offset: u32,
}

impl DexHeader {
    pub(crate) fn empty(version: DexVersion) -> Self {
        let empty = Section::default();
        Self {
            version,
            checksum: u32::default(),
            signature: [u8::default(); SIGNATURE_SIZE],
            file_size: u32::default(),
            header_size: version.header_size(),
            endian: Endian::Little,
            link_size: u32::default(),
            link_off: ABSENT_OFFSET,
            map_off: ABSENT_OFFSET,
            string_ids: empty,
            type_ids: empty,
            proto_ids: empty,
            field_ids: empty,
            method_ids: empty,
            class_defs: empty,
            data: empty,
            container_size: u32::default(),
            header_offset: ABSENT_OFFSET,
        }
    }
}

/// Count and file offset of a top-level DEX section.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct Section {
    /// Number of items, or byte size for byte-oriented sections.
    pub size: u32,
    /// Absolute file offset, or zero when the section is empty.
    pub offset: u32,
}

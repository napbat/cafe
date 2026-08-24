//! DEX map-list entries and extensible item-type codes.

use crate::file::header::DexVersion;
use crate::file::layout::ItemWidth;

/// Type of a section listed in a DEX map.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum MapItemType {
    /// Header item (`0x0000`).
    Header,
    /// String identifier table (`0x0001`).
    StringId,
    /// Type identifier table (`0x0002`).
    TypeId,
    /// Prototype identifier table (`0x0003`).
    PrototypeId,
    /// Field identifier table (`0x0004`).
    FieldId,
    /// Method identifier table (`0x0005`).
    MethodId,
    /// Class-definition table (`0x0006`).
    ClassDefinition,
    /// Call-site identifier table (`0x0007`).
    CallSiteId,
    /// Method-handle table (`0x0008`).
    MethodHandle,
    /// Map list (`0x1000`).
    MapList,
    /// Type list (`0x1001`).
    TypeList,
    /// Annotation-set reference list (`0x1002`).
    AnnotationSetRefList,
    /// Annotation set (`0x1003`).
    AnnotationSet,
    /// Class data (`0x2000`).
    ClassData,
    /// Code item (`0x2001`).
    Code,
    /// String data (`0x2002`).
    StringData,
    /// Debug information (`0x2003`).
    DebugInfo,
    /// Annotation item (`0x2004`).
    Annotation,
    /// Encoded array (`0x2005`).
    EncodedArray,
    /// Annotation directory (`0x2006`).
    AnnotationDirectory,
    /// Hidden API class data (`0xf000`).
    HiddenApiClassData,
    /// Future or implementation-specific item retained by numeric code.
    Unknown(u16),
}

impl MapItemType {
    /// Parses a map item type while retaining unknown future values.
    #[must_use]
    pub const fn from_u16(value: u16) -> Self {
        match value {
            0x0000 => Self::Header,
            0x0001 => Self::StringId,
            0x0002 => Self::TypeId,
            0x0003 => Self::PrototypeId,
            0x0004 => Self::FieldId,
            0x0005 => Self::MethodId,
            0x0006 => Self::ClassDefinition,
            0x0007 => Self::CallSiteId,
            0x0008 => Self::MethodHandle,
            0x1000 => Self::MapList,
            0x1001 => Self::TypeList,
            0x1002 => Self::AnnotationSetRefList,
            0x1003 => Self::AnnotationSet,
            0x2000 => Self::ClassData,
            0x2001 => Self::Code,
            0x2002 => Self::StringData,
            0x2003 => Self::DebugInfo,
            0x2004 => Self::Annotation,
            0x2005 => Self::EncodedArray,
            0x2006 => Self::AnnotationDirectory,
            0xf000 => Self::HiddenApiClassData,
            value => Self::Unknown(value),
        }
    }

    /// Returns the exact encoded type code.
    #[must_use]
    pub const fn as_u16(self) -> u16 {
        match self {
            Self::Header => 0x0000,
            Self::StringId => 0x0001,
            Self::TypeId => 0x0002,
            Self::PrototypeId => 0x0003,
            Self::FieldId => 0x0004,
            Self::MethodId => 0x0005,
            Self::ClassDefinition => 0x0006,
            Self::CallSiteId => 0x0007,
            Self::MethodHandle => 0x0008,
            Self::MapList => 0x1000,
            Self::TypeList => 0x1001,
            Self::AnnotationSetRefList => 0x1002,
            Self::AnnotationSet => 0x1003,
            Self::ClassData => 0x2000,
            Self::Code => 0x2001,
            Self::StringData => 0x2002,
            Self::DebugInfo => 0x2003,
            Self::Annotation => 0x2004,
            Self::EncodedArray => 0x2005,
            Self::AnnotationDirectory => 0x2006,
            Self::HiddenApiClassData => 0xf000,
            Self::Unknown(value) => value,
        }
    }

    /// Returns the fixed item width for a format version when the type is not
    /// variable-length.
    #[must_use]
    pub const fn fixed_width(self, version: DexVersion) -> Option<ItemWidth> {
        match self {
            Self::Header => Some(ItemWidth::from_u32(version.header_size())),
            Self::StringId => Some(ItemWidth::STRING_ID),
            Self::TypeId => Some(ItemWidth::TYPE_ID),
            Self::PrototypeId => Some(ItemWidth::PROTOTYPE_ID),
            Self::FieldId => Some(ItemWidth::FIELD_ID),
            Self::MethodId => Some(ItemWidth::METHOD_ID),
            Self::ClassDefinition => Some(ItemWidth::CLASS_DEFINITION),
            Self::CallSiteId => Some(ItemWidth::CALL_SITE_ID),
            Self::MethodHandle => Some(ItemWidth::METHOD_HANDLE),
            Self::MapList
            | Self::TypeList
            | Self::AnnotationSetRefList
            | Self::AnnotationSet
            | Self::ClassData
            | Self::Code
            | Self::StringData
            | Self::DebugInfo
            | Self::Annotation
            | Self::EncodedArray
            | Self::AnnotationDirectory
            | Self::HiddenApiClassData
            | Self::Unknown(_) => None,
        }
    }
}

/// One entry in the DEX map list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MapItem {
    /// Item type.
    pub item_type: MapItemType,
    /// Number of items in the section.
    pub size: u32,
    /// Absolute offset of the first item.
    pub offset: u32,
}

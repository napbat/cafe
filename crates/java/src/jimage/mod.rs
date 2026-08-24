//! Read-only ingestion of the JDK's module-image (`lib/modules`) container.
//!
//! JIMAGE is an indexed resource container. Its class resources remain normal
//! JVM class files, so this module deliberately exposes container inventory,
//! decompression, and class visitation without adding instruction semantics.

mod decompress;
mod parse;

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::sync::Arc;

use crate::classfile::ClassFile;
use crate::{Error, Result};

/// JIMAGE header magic in the image's native byte order.
pub const JIMAGE_MAGIC: u32 = 0xcafe_dada;
/// Supported JIMAGE major format version.
pub const JIMAGE_MAJOR_VERSION: u16 = 1;
/// Supported JIMAGE minor format version.
pub const JIMAGE_MINOR_VERSION: u16 = 0;
/// Number of fixed-width 32-bit slots in a JIMAGE header.
pub const JIMAGE_HEADER_SLOTS: usize = 7;
/// Byte width of a JIMAGE header.
pub const JIMAGE_HEADER_SIZE: usize = JIMAGE_HEADER_SLOTS * size_of::<u32>();
/// Magic stored at the start of a compressed JIMAGE resource layer.
pub const COMPRESSED_RESOURCE_MAGIC: u32 = 0xcafe_fafa;
/// Byte width of a compressed-resource layer header.
pub const COMPRESSED_RESOURCE_HEADER_SIZE: usize = 29;

/// Byte order used by one JIMAGE container.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum JimageEndian {
    /// Least-significant byte first.
    Little,
    /// Most-significant byte first.
    Big,
}

impl JimageEndian {
    pub(crate) fn read_u32(self, bytes: [u8; 4]) -> u32 {
        match self {
            Self::Little => u32::from_le_bytes(bytes),
            Self::Big => u32::from_be_bytes(bytes),
        }
    }

    pub(crate) fn read_u64(self, bytes: [u8; 8]) -> u64 {
        match self {
            Self::Little => u64::from_le_bytes(bytes),
            Self::Big => u64::from_be_bytes(bytes),
        }
    }
}

/// Fixed JIMAGE index header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JimageHeader {
    /// Container byte order inferred from the magic bytes.
    pub endian: JimageEndian,
    /// Major format version.
    pub major_version: u16,
    /// Minor format version.
    pub minor_version: u16,
    /// Format flags retained verbatim.
    pub flags: u32,
    /// Number of indexed resources.
    pub resource_count: u32,
    /// Slot count of both redirect and location-offset tables.
    pub table_length: u32,
    /// Byte length of the compressed location-attribute section.
    pub locations_size: u32,
    /// Byte length of the modified-UTF-8 string table.
    pub strings_size: u32,
}

/// Inventory record for one physical JIMAGE resource.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JimageEntry {
    /// Fully qualified image name, normally `/module/path/name.ext`.
    pub name: String,
    /// Module name, or an empty string for a non-module resource.
    pub module: String,
    /// Resource path relative to its module.
    pub path: String,
    /// Offset relative to the end of the image index.
    pub offset: u64,
    /// Stored size, excluding zero for an uncompressed resource.
    pub compressed_size: u64,
    /// Size after all resource decompression layers.
    pub uncompressed_size: u64,
    /// Preview-feature flags retained from newer image writers.
    pub preview_flags: u64,
}

impl JimageEntry {
    /// Returns whether the payload uses one or more compression layers.
    #[must_use]
    pub const fn is_compressed(&self) -> bool {
        self.compressed_size != 0
    }

    /// Returns whether this resource contains a JVM class file.
    #[must_use]
    pub fn is_class(&self) -> bool {
        self.path.ends_with(crate::jar::CLASS_ENTRY_SUFFIX)
    }
}

/// Borrowed metadata passed to a JIMAGE class visitor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JimageClassEntry<'a> {
    /// Fully qualified image resource name.
    pub resource_name: &'a str,
    /// Module owning the class.
    pub module: &'a str,
    /// Internal class name without the `.class` suffix.
    pub class_name: &'a str,
    /// Uncompressed class-file size.
    pub size: u64,
}

/// Controls continuation of class visitation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum JimageVisitControl {
    /// Continue with the next selected class.
    Continue,
    /// End visitation successfully.
    Break,
}

/// Parsed, read-only JIMAGE retaining its exact original bytes.
#[derive(Debug, Clone)]
pub struct JimageFile {
    bytes: Arc<[u8]>,
    header: JimageHeader,
    index_size: usize,
    strings_offset: usize,
    strings_size: usize,
    entries: Vec<JimageEntry>,
    by_name: BTreeMap<String, usize>,
}

impl JimageFile {
    /// Opens a JIMAGE from disk.
    ///
    /// # Errors
    ///
    /// Returns an error for unreadable or malformed input.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        Self::from_bytes(fs::read(path)?)
    }

    /// Parses one complete JIMAGE container.
    ///
    /// # Errors
    ///
    /// Returns an error for an unsupported version, malformed index, invalid
    /// string reference, duplicate name, or out-of-bounds resource.
    pub fn from_bytes(bytes: impl Into<Vec<u8>>) -> Result<Self> {
        parse::parse(bytes.into())
    }

    /// Returns the parsed fixed header.
    #[must_use]
    pub const fn header(&self) -> JimageHeader {
        self.header
    }

    /// Returns the exact original container bytes.
    #[must_use]
    pub fn original_bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Returns the byte offset at which resource payloads begin.
    #[must_use]
    pub const fn index_size(&self) -> usize {
        self.index_size
    }

    /// Returns resources in deterministic fully-qualified-name order.
    #[must_use]
    pub fn entries(&self) -> &[JimageEntry] {
        &self.entries
    }

    /// Looks up resource metadata by its fully qualified image name.
    #[must_use]
    pub fn entry(&self, name: &str) -> Option<&JimageEntry> {
        let normalized = normalize_resource_name(name);
        self.by_name
            .get(&normalized)
            .and_then(|&index| self.entries.get(index))
    }

    /// Reads and fully decompresses one resource.
    ///
    /// # Errors
    ///
    /// Returns an error when the name is absent, its stored range is invalid,
    /// or a compression layer is malformed or unsupported.
    pub fn read_entry(&self, name: &str) -> Result<Vec<u8>> {
        let normalized = normalize_resource_name(name);
        let entry = self
            .entry(&normalized)
            .ok_or_else(|| Error::JimageEntryNotFound(normalized.clone()))?;
        self.read_entry_record(entry)
    }

    /// Reads and parses a class by module and internal class name.
    ///
    /// # Errors
    ///
    /// Returns an error when the resource is absent or malformed.
    pub fn read_class(&self, module: &str, class_name: &str) -> Result<ClassFile> {
        let class_name = normalize_class_name(class_name);
        let resource_name = format!("/{module}/{class_name}{}", crate::jar::CLASS_ENTRY_SUFFIX);
        let bytes = self.read_entry(&resource_name)?;
        ClassFile::parse(&bytes).map_err(|error| error.in_jar_entry(resource_name))
    }

    /// Visits selected class resources without reopening the image.
    ///
    /// # Errors
    ///
    /// Returns the first selected decompression, class parsing, or visitor
    /// error.
    pub fn visit_classes<S, V, E>(&self, mut select: S, mut visit: V) -> std::result::Result<(), E>
    where
        for<'entry> S: FnMut(JimageClassEntry<'entry>) -> bool,
        for<'entry> V: FnMut(
            JimageClassEntry<'entry>,
            ClassFile,
        ) -> std::result::Result<JimageVisitControl, E>,
        E: From<Error>,
    {
        for entry in &self.entries {
            let Some(class_entry) = class_entry(entry) else {
                continue;
            };
            if !select(class_entry) {
                continue;
            }
            let bytes = self.read_entry_record(entry).map_err(E::from)?;
            let class = ClassFile::parse(&bytes)
                .map_err(|error| E::from(error.in_jar_entry(entry.name.clone())))?;
            if visit(class_entry, class)? == JimageVisitControl::Break {
                break;
            }
        }
        Ok(())
    }

    /// Visits selected raw class payloads without reopening the image.
    ///
    /// Decompression failures are passed to the visitor, allowing aggregate
    /// validation to continue with later resources.
    ///
    /// # Errors
    ///
    /// Returns only an error produced by the visitor.
    pub fn visit_class_bytes<S, V, E>(
        &self,
        mut select: S,
        mut visit: V,
    ) -> std::result::Result<(), E>
    where
        for<'entry> S: FnMut(JimageClassEntry<'entry>) -> bool,
        for<'entry, 'payload> V: FnMut(
            JimageClassEntry<'entry>,
            std::result::Result<&'payload [u8], Error>,
        ) -> std::result::Result<JimageVisitControl, E>,
    {
        for entry in &self.entries {
            let Some(class_entry) = class_entry(entry) else {
                continue;
            };
            if !select(class_entry) {
                continue;
            }
            let bytes = self.read_entry_record(entry);
            let control = match bytes {
                Ok(bytes) => visit(class_entry, Ok(&bytes))?,
                Err(error) => visit(class_entry, Err(error))?,
            };
            if control == JimageVisitControl::Break {
                break;
            }
        }
        Ok(())
    }

    fn read_entry_record(&self, entry: &JimageEntry) -> Result<Vec<u8>> {
        let stored_size = if entry.is_compressed() {
            entry.compressed_size
        } else {
            entry.uncompressed_size
        };
        let start = self
            .index_size
            .checked_add(usize::try_from(entry.offset).map_err(|_| {
                Error::invalid_jimage(self.index_size, "resource offset does not fit memory")
            })?)
            .ok_or_else(|| Error::invalid_jimage(self.index_size, "resource offset overflow"))?;
        let end = start
            .checked_add(usize::try_from(stored_size).map_err(|_| {
                Error::invalid_jimage(start, "stored resource size does not fit memory")
            })?)
            .ok_or_else(|| Error::invalid_jimage(start, "stored resource range overflow"))?;
        let raw = self.bytes.get(start..end).ok_or_else(|| {
            Error::invalid_jimage(
                start,
                format!("resource `{}` exceeds the image", entry.name),
            )
        })?;
        if entry.is_compressed() {
            decompress::resource(self, entry, raw)
        } else {
            Ok(raw.to_vec())
        }
    }

    pub(crate) fn string_units(&self, offset: u32) -> Result<Vec<u16>> {
        parse::string_units(&self.bytes, self.strings_offset, self.strings_size, offset)
    }
}

fn class_entry(entry: &JimageEntry) -> Option<JimageClassEntry<'_>> {
    let class_name = entry.path.strip_suffix(crate::jar::CLASS_ENTRY_SUFFIX)?;
    Some(JimageClassEntry {
        resource_name: &entry.name,
        module: &entry.module,
        class_name,
        size: entry.uncompressed_size,
    })
}

fn normalize_resource_name(name: &str) -> String {
    let normalized = name.replace('\\', "/");
    if normalized.starts_with('/') {
        normalized
    } else {
        format!("/{normalized}")
    }
}

fn normalize_class_name(name: &str) -> String {
    name.trim_start_matches('/')
        .trim_end_matches(crate::jar::CLASS_ENTRY_SUFFIX)
        .replace(['.', '\\'], "/")
}

#[cfg(test)]
mod tests;

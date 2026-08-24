//! Archive-entry and class inventory models and operations.

use crate::Result;
use crate::classfile::{ClassAccessFlags, ClassFile};

use super::entry::EntryData;
use super::{EntryId, EntryKind, EntryMetadata, JarFile, is_class_entry};

/// Metadata for one archive entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntryInfo {
    /// Stable identity within the open JAR.
    pub id: EntryId,
    /// Current entry name using the JAR's forward-slash separator.
    pub name: String,
    /// Name read from the original archive, or `None` for a new entry.
    pub original_name: Option<String>,
    /// Current uncompressed byte length.
    pub size: u64,
    /// Original compressed length, or `None` for new/replaced payloads.
    pub compressed_size: Option<u64>,
    /// Original CRC-32, or `None` for new/replaced payloads.
    pub crc32: Option<u32>,
    /// Whether this member is a file, directory marker, or symbolic link.
    pub kind: EntryKind,
    /// Editable ZIP metadata retained for the entry.
    pub metadata: EntryMetadata,
    /// Whether the source ZIP marked the member as encrypted.
    pub encrypted: bool,
}

impl EntryInfo {
    /// Returns whether the entry name ends in `.class`.
    #[must_use]
    pub fn is_class(&self) -> bool {
        self.kind == EntryKind::File && is_class_entry(&self.name)
    }

    /// Returns whether this entry is a directory marker.
    #[must_use]
    pub const fn is_dir(&self) -> bool {
        matches!(self.kind, EntryKind::Directory)
    }

    /// Returns whether this entry is a Unix symbolic link.
    #[must_use]
    pub const fn is_symlink(&self) -> bool {
        matches!(self.kind, EntryKind::Symlink)
    }

    /// Returns whether the entry payload still comes from the original ZIP.
    #[must_use]
    pub const fn has_original_payload(&self) -> bool {
        self.crc32.is_some()
    }
}

/// Metadata obtained by parsing one class entry during archive enumeration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClassSummary {
    /// Stable identity of the class entry.
    pub entry_id: EntryId,
    /// Exact entry name in the JAR.
    pub entry_name: String,
    /// Internal JVM class name declared by the class file.
    pub internal_name: String,
    /// Class-file minor version.
    pub minor_version: u16,
    /// Class-file major version.
    pub major_version: u16,
    /// Typed class access flags.
    pub access_flags: ClassAccessFlags,
    /// Number of declared fields.
    pub fields: usize,
    /// Number of declared methods.
    pub methods: usize,
    /// Uncompressed class-file length.
    pub size: u64,
}

impl JarFile {
    /// Collects metadata for every archive entry in current order.
    ///
    /// # Errors
    ///
    /// This cached inventory currently cannot fail; the result wrapper is
    /// retained for source compatibility with the original read-only API.
    pub fn entries(&self) -> Result<Vec<EntryInfo>> {
        Ok(self
            .entries
            .iter()
            .map(|entry| {
                let (size, compressed_size, crc32) = match (&entry.data, entry.original_stats) {
                    (EntryData::Original(_), Some(stats)) => {
                        (stats.size, Some(stats.compressed_size), Some(stats.crc32))
                    }
                    (EntryData::Owned(bytes), _) => {
                        (u64::try_from(bytes.len()).unwrap_or(u64::MAX), None, None)
                    }
                    (EntryData::Original(_), None) => {
                        unreachable!("original JAR entries retain their source statistics")
                    }
                };
                EntryInfo {
                    id: entry.id,
                    name: entry.name.clone(),
                    original_name: entry.original_name.clone(),
                    size,
                    compressed_size,
                    crc32,
                    kind: entry.kind,
                    metadata: entry.metadata.clone(),
                    encrypted: entry.encrypted,
                }
            })
            .collect())
    }

    /// Returns all `.class` entry names in archive order.
    #[must_use]
    pub fn class_entry_names(&self) -> Vec<String> {
        self.entries
            .iter()
            .filter(|entry| entry.kind == EntryKind::File && is_class_entry(&entry.name))
            .map(|entry| entry.name.clone())
            .collect()
    }

    /// Returns the number of `.class` entries without reading their payloads.
    #[must_use]
    pub fn class_entry_count(&self) -> usize {
        self.entries
            .iter()
            .filter(|entry| entry.kind == EntryKind::File && is_class_entry(&entry.name))
            .count()
    }

    /// Parses and returns metadata for every class entry in archive order.
    ///
    /// # Errors
    ///
    /// Returns an error identifying the first unreadable or invalid class
    /// entry.
    pub fn class_summaries(&self) -> Result<Vec<ClassSummary>> {
        let mut summaries = Vec::new();
        for entry in &self.entries {
            if entry.kind != EntryKind::File || !is_class_entry(&entry.name) {
                continue;
            }
            let bytes = self.read_entry_by_id(entry.id)?;
            let class =
                ClassFile::parse(&bytes).map_err(|error| error.in_jar_entry(entry.name.clone()))?;
            let internal_name = class
                .class_name()
                .map_err(|error| error.in_jar_entry(entry.name.clone()))?
                .to_owned();
            summaries.push(ClassSummary {
                entry_id: entry.id,
                entry_name: entry.name.clone(),
                internal_name,
                minor_version: class.minor_version,
                major_version: class.major_version,
                access_flags: class.access_flags,
                fields: class.fields.len(),
                methods: class.methods.len(),
                size: bytes.len() as u64,
            });
        }
        Ok(summaries)
    }
}

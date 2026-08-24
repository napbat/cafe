//! Archive-entry inventory models and operations.

use crate::Result;

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
                let (compressed_size, crc32) = match (&entry.data, entry.original_stats) {
                    (EntryData::Original(_), Some(stats)) => {
                        (Some(stats.compressed_size), Some(stats.crc32))
                    }
                    (EntryData::Owned(_), _) => (None, None),
                    (EntryData::Original(_), None) => {
                        unreachable!("original JAR entries retain their source statistics")
                    }
                };
                EntryInfo {
                    id: entry.id,
                    name: entry.name.clone(),
                    original_name: entry.original_name.clone(),
                    size: entry.uncompressed_size(),
                    compressed_size,
                    crc32,
                    kind: entry.kind,
                    metadata: entry.metadata.clone(),
                    encrypted: entry.encrypted,
                }
            })
            .collect())
    }
}

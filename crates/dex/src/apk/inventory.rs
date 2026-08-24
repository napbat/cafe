//! APK entry inventory models.

use super::entry::EntryData;
use super::{ApkFile, EntryId, EntryKind, EntryMetadata};

/// Metadata for one APK entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntryInfo {
    /// Stable identity within the open APK.
    pub id: EntryId,
    /// Current forward-slash-separated entry name.
    pub name: String,
    /// Original entry name, or `None` for a new member.
    pub original_name: Option<String>,
    /// Current uncompressed byte length.
    pub size: u64,
    /// Original compressed length, if the payload is unchanged.
    pub compressed_size: Option<u64>,
    /// Original CRC-32, if the payload is unchanged.
    pub crc32: Option<u32>,
    /// File, directory, or symbolic-link kind.
    pub kind: EntryKind,
    /// Editable ZIP metadata.
    pub metadata: EntryMetadata,
    /// Whether the source ZIP marks this member as encrypted.
    pub encrypted: bool,
}

impl EntryInfo {
    /// Returns whether this is a directory marker.
    #[must_use]
    pub const fn is_dir(&self) -> bool {
        matches!(self.kind, EntryKind::Directory)
    }

    /// Returns whether this is a Unix symbolic link.
    #[must_use]
    pub const fn is_symlink(&self) -> bool {
        matches!(self.kind, EntryKind::Symlink)
    }

    /// Returns whether the payload still comes from the original ZIP.
    #[must_use]
    pub const fn has_original_payload(&self) -> bool {
        self.crc32.is_some()
    }
}

impl ApkFile {
    /// Collects metadata for every member in current archive order.
    #[must_use]
    pub fn entries(&self) -> Vec<EntryInfo> {
        self.entries
            .iter()
            .map(|entry| {
                let (size, compressed_size, crc32) = match (&entry.data, &entry.original_stats) {
                    (EntryData::Original(_), Some(stats)) => {
                        (stats.size, Some(stats.compressed_size), Some(stats.crc32))
                    }
                    (EntryData::Owned(bytes), _) => {
                        (u64::try_from(bytes.len()).unwrap_or(u64::MAX), None, None)
                    }
                    (EntryData::Original(_), None) => {
                        unreachable!("original APK entries retain their source statistics")
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
            .collect()
    }
}

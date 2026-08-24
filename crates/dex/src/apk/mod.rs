//! Lossless APK archive editing and deterministic multidex provenance.
//!
//! APK is modeled as a ZIP container around DEX artifacts and resources. It
//! does not introduce another instruction set or shared binary format.

use std::fs;
use std::io::{Cursor, Read};
use std::path::Path;
use std::sync::Arc;

use zip::ZipArchive;

use crate::{Error, Result};

mod dex;
mod discovery;
mod edit;
mod entry;
mod inventory;
mod layout;
mod signature;

pub use self::dex::{DexArtifact, DexEntry, DexOrdinal, dex_entry_name, parse_dex_entry_name};
pub use self::discovery::{APK_EXTENSION, Traversal, discover_apks, is_apk_path};
pub use self::edit::SignaturePolicy;
pub use self::entry::{EntryId, EntryKind, EntryMetadata, ExtraField, ExtraFieldPlacement};
pub use self::inventory::EntryInfo;
pub use self::signature::{
    SOURCE_STAMP_CERTIFICATE_ENTRY, SignatureState, SigningBlock, SigningBlockEntry,
    SigningBlockId, SigningBlockKind, V1_MANIFEST_ENTRY, is_v1_signature_entry,
};
pub use zip::{CompressionMethod, DateTime, System};

use self::entry::{ApkEntry, EntryData, OriginalEntryStats};
use self::signature::parse_signing_block;

type SourceReader = Cursor<Arc<[u8]>>;

/// In-memory editable APK with stable entry identities and exact pristine output.
#[derive(Debug, Clone)]
pub struct ApkFile {
    original: Option<Arc<[u8]>>,
    entries: Vec<ApkEntry>,
    comment: Vec<u8>,
    signing_block: Option<SigningBlock>,
    dirty: bool,
    next_id: EntryId,
}

impl ApkFile {
    /// Creates an empty editable APK.
    #[must_use]
    pub fn new() -> Self {
        Self {
            original: None,
            entries: Vec::new(),
            comment: Vec::new(),
            signing_block: None,
            dirty: true,
            next_id: EntryId::initial(),
        }
    }

    /// Opens an APK file from disk.
    ///
    /// # Errors
    ///
    /// Returns an error when the path cannot be read or the APK is malformed.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        Self::from_bytes(fs::read(path)?)
    }

    /// Opens an APK from complete archive bytes.
    ///
    /// The ZIP directory and APK signing block are parsed eagerly. Entry
    /// payloads remain lazy until requested.
    ///
    /// # Errors
    ///
    /// Returns an error for malformed ZIP metadata or APK signing-block data.
    pub fn from_bytes(bytes: impl Into<Vec<u8>>) -> Result<Self> {
        let original: Arc<[u8]> = bytes.into().into();
        let signing_block = parse_signing_block(&original)?;
        let mut archive = ZipArchive::new(Cursor::new(Arc::clone(&original)))?;
        let comment = archive.comment().to_vec();
        let mut entries = Vec::with_capacity(archive.len());
        for index in 0..archive.len() {
            let file = archive.by_index(index)?;
            let id = EntryId::from_position(index)?;
            let name = file.name().to_owned();
            let kind = if file.is_symlink() {
                EntryKind::Symlink
            } else if file.is_dir() {
                EntryKind::Directory
            } else {
                EntryKind::File
            };
            entries.push(ApkEntry {
                id,
                name: name.clone(),
                original_name: Some(name),
                kind,
                metadata: EntryMetadata::from_zip(&file)?,
                data: EntryData::Original(index),
                original_stats: Some(OriginalEntryStats {
                    size: file.size(),
                    compressed_size: file.compressed_size(),
                    crc32: file.crc32(),
                }),
                encrypted: file.encrypted(),
            });
        }
        let next_id = EntryId::from_position(archive.len())?;
        Ok(Self {
            original: Some(original),
            entries,
            comment,
            signing_block,
            dirty: false,
            next_id,
        })
    }

    /// Returns the number of entries, including resources and directories.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Returns whether the APK contains no ZIP entries.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Returns whether an opened archive has not been mutated.
    #[must_use]
    pub fn is_pristine(&self) -> bool {
        self.original.is_some() && !self.dirty
    }

    /// Returns the exact original archive bytes, when opened from existing data.
    #[must_use]
    pub fn original_bytes(&self) -> Option<&[u8]> {
        self.original.as_deref()
    }

    /// Returns the raw, potentially non-UTF-8 ZIP comment.
    #[must_use]
    pub fn archive_comment(&self) -> &[u8] {
        &self.comment
    }

    /// Reads an unambiguous entry's complete uncompressed contents.
    ///
    /// # Errors
    ///
    /// Returns an error when the name is absent, duplicated, or unreadable.
    pub fn read_entry(&self, name: &str) -> Result<Vec<u8>> {
        self.read_entry_by_id(self.unique_entry_id(name)?)
    }

    /// Reads an entry by stable identity.
    ///
    /// # Errors
    ///
    /// Returns an error when the ID is absent or its payload cannot be read.
    pub fn read_entry_by_id(&self, id: EntryId) -> Result<Vec<u8>> {
        let entry = self.entry_record(id)?;
        match &entry.data {
            EntryData::Owned(bytes) => Ok(bytes.clone()),
            EntryData::Original(index) => {
                let mut archive = self.source_archive()?;
                let mut file = archive.by_index(*index)?;
                read_zip_file(&mut file)
            }
        }
    }

    /// Returns stable IDs for every exact name match in archive order.
    #[must_use]
    pub fn entry_ids_named(&self, name: &str) -> Vec<EntryId> {
        self.entries
            .iter()
            .filter(|entry| entry.name == name)
            .map(|entry| entry.id)
            .collect()
    }

    fn unique_entry_id(&self, name: &str) -> Result<EntryId> {
        let mut matches = self.entry_ids_named(name).into_iter();
        let Some(id) = matches.next() else {
            return Err(Error::ApkEntryNotFound(name.to_owned()));
        };
        let remainder = matches.count();
        if remainder != 0 {
            return Err(Error::AmbiguousApkEntry {
                name: name.to_owned(),
                count: remainder + 1,
            });
        }
        Ok(id)
    }

    fn entry_record(&self, id: EntryId) -> Result<&ApkEntry> {
        self.entries
            .iter()
            .find(|entry| entry.id == id)
            .ok_or(Error::ApkEntryIdNotFound(id.get()))
    }

    fn entry_record_mut(&mut self, id: EntryId) -> Result<&mut ApkEntry> {
        self.entries
            .iter_mut()
            .find(|entry| entry.id == id)
            .ok_or(Error::ApkEntryIdNotFound(id.get()))
    }

    fn source_archive(&self) -> Result<ZipArchive<SourceReader>> {
        let original = self
            .original
            .as_ref()
            .ok_or_else(|| Error::invalid_apk("entry has no original archive backing"))?;
        Ok(ZipArchive::new(Cursor::new(Arc::clone(original)))?)
    }
}

impl Default for ApkFile {
    fn default() -> Self {
        Self::new()
    }
}

pub(super) fn read_zip_file<R: Read>(reader: &mut R) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    reader.read_to_end(&mut bytes)?;
    Ok(bytes)
}

//! Reading, creating, editing, and validating Java archives.

use std::fs;
use std::io::{Cursor, Read};
use std::path::Path;
use std::sync::Arc;

use zip::ZipArchive;

use crate::classfile::ClassFile;
use crate::{Error, Result};

mod discovery;
mod edit;
mod entry;
mod inventory;
mod layout;
mod manifest;
mod multi_release;
mod services;
mod signature;
mod validation;

pub use self::discovery::{Traversal, discover_jars, is_jar_path};
pub use self::edit::SignaturePolicy;
pub use self::entry::{EntryId, EntryKind, EntryMetadata, ExtraField, ExtraFieldPlacement};
pub use self::inventory::{ClassSummary, EntryInfo};
pub use self::manifest::{
    DEFAULT_MANIFEST_VERSION, MANIFEST_VERSION_HEADER, Manifest, ManifestAttribute,
    ManifestSection, NAME_HEADER, NamedManifestSection,
};
pub use self::multi_release::{ResolvedEntry, parse_versioned_entry, versioned_entry_name};
pub use self::services::{SERVICE_PREFIX, ServiceConfiguration, is_service_entry};
pub use self::signature::{SignatureState, is_signature_entry};
pub use self::validation::{ArchiveValidationReport, ValidationReport};
pub use zip::{CompressionMethod, DateTime, System};

use self::entry::{EntryData, JarEntry, OriginalEntryStats};

/// Conventional path of a JAR manifest.
pub const MANIFEST_ENTRY: &str = "META-INF/MANIFEST.MF";

/// File-name suffix used by JVM class entries in a JAR.
pub const CLASS_ENTRY_SUFFIX: &str = ".class";
/// Conventional top-level metadata directory.
pub const META_INF_DIRECTORY: &str = META_INF_PREFIX;
/// Prefix of every top-level JAR metadata entry.
pub const META_INF_PREFIX: &str = "META-INF/";
/// Manifest header enabling multi-release entry selection.
pub const MULTI_RELEASE_HEADER: &str = "Multi-Release";
/// Canonical enabled value of [`MULTI_RELEASE_HEADER`].
pub const MULTI_RELEASE_ENABLED_VALUE: &str = "true";
/// Archive prefix containing version-specific multi-release entries.
pub const MULTI_RELEASE_ENTRY_PREFIX: &str = "META-INF/versions/";

type SourceReader = Cursor<Arc<[u8]>>;

/// An in-memory, editable JAR with stable entry identities.
///
/// Original payloads remain lazy until read. An unchanged archive is emitted
/// using its original bytes, preserving every ZIP detail exactly.
#[derive(Debug, Clone)]
pub struct JarFile {
    pub(crate) original: Option<Arc<[u8]>>,
    pub(crate) entries: Vec<JarEntry>,
    pub(crate) comment: Vec<u8>,
    pub(crate) dirty: bool,
    pub(crate) next_id: EntryId,
}

impl JarFile {
    /// Creates an empty editable JAR.
    #[must_use]
    pub fn new() -> Self {
        Self {
            original: None,
            entries: Vec::new(),
            comment: Vec::new(),
            dirty: true,
            next_id: EntryId::initial(),
        }
    }

    /// Opens a JAR file from disk and reads its ZIP directory.
    ///
    /// # Errors
    ///
    /// Returns an error if the path cannot be read or is not a valid ZIP.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        Self::from_bytes(fs::read(path)?)
    }

    /// Opens a JAR from complete ZIP bytes.
    ///
    /// # Errors
    ///
    /// Returns an error if the bytes are not a valid ZIP archive or contain
    /// malformed entry metadata.
    pub fn from_bytes(bytes: impl Into<Vec<u8>>) -> Result<Self> {
        let original: Arc<[u8]> = bytes.into().into();
        let mut archive = ZipArchive::new(Cursor::new(Arc::clone(&original)))?;
        let comment = archive.comment().to_vec();
        let mut entries = Vec::with_capacity(archive.len());
        for index in 0..archive.len() {
            let file = archive.by_index(index)?;
            let name = file.name().to_owned();
            let kind = if file.is_symlink() {
                EntryKind::Symlink
            } else if file.is_dir() {
                EntryKind::Directory
            } else {
                EntryKind::File
            };
            entries.push(JarEntry {
                id: EntryId::from_position(index)?,
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
        Ok(Self {
            original: Some(original),
            entries,
            comment,
            dirty: false,
            next_id: EntryId::from_position(archive.len())?,
        })
    }

    /// Returns the number of all entries, including resources and directories.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Returns whether the JAR has no entries.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Returns whether no operation has changed an archive opened from bytes.
    #[must_use]
    pub fn is_pristine(&self) -> bool {
        self.original.is_some() && !self.dirty
    }

    /// Returns the complete original archive bytes, when this JAR was opened
    /// from an existing archive.
    #[must_use]
    pub fn original_bytes(&self) -> Option<&[u8]> {
        self.original.as_deref()
    }

    /// Returns the raw, potentially non-UTF-8 ZIP archive comment.
    #[must_use]
    pub fn archive_comment(&self) -> &[u8] {
        &self.comment
    }

    /// Reads an unambiguous entry's complete uncompressed contents.
    ///
    /// Use [`Self::entry_ids_named`] and [`Self::read_entry_by_id`] when an
    /// input ZIP contains duplicate names.
    ///
    /// # Errors
    ///
    /// Returns an error if the entry is absent, ambiguous, unsupported, or
    /// cannot be decompressed.
    pub fn read_entry(&self, name: &str) -> Result<Vec<u8>> {
        let id = self.unique_entry_id(name)?;
        self.read_entry_by_id(id)
    }

    /// Reads an entry by stable identity.
    ///
    /// # Errors
    ///
    /// Returns an error if the ID is absent or the original payload cannot be
    /// decompressed.
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

    /// Resolves a dotted, internal, or `.class` name and parses that class.
    ///
    /// # Errors
    ///
    /// Returns an error if the class is absent, ambiguous, cannot be
    /// decompressed, or is an invalid class file.
    pub fn read_class(&self, class_name: &str) -> Result<ClassFile> {
        let entry_name = normalize_class_entry(class_name);
        let bytes = match self.read_entry(&entry_name) {
            Ok(bytes) => bytes,
            Err(Error::JarEntryNotFound(_)) => return Err(Error::ClassNotFound(entry_name)),
            Err(error) => return Err(error),
        };
        ClassFile::parse(&bytes).map_err(|error| error.in_jar_entry(entry_name))
    }

    /// Returns stable IDs for every exact entry-name match in archive order.
    #[must_use]
    pub fn entry_ids_named(&self, name: &str) -> Vec<EntryId> {
        self.entries
            .iter()
            .filter(|entry| entry.name == name)
            .map(|entry| entry.id)
            .collect()
    }

    pub(crate) fn unique_entry_id(&self, name: &str) -> Result<EntryId> {
        let mut matches = self.entry_ids_named(name).into_iter();
        let Some(id) = matches.next() else {
            return Err(Error::JarEntryNotFound(name.to_owned()));
        };
        let remainder = matches.count();
        if remainder != 0 {
            return Err(Error::AmbiguousJarEntry {
                name: name.to_owned(),
                count: remainder + 1,
            });
        }
        Ok(id)
    }

    pub(crate) fn entry_record(&self, id: EntryId) -> Result<&JarEntry> {
        self.entries
            .iter()
            .find(|entry| entry.id == id)
            .ok_or(Error::JarEntryIdNotFound(id.0))
    }

    pub(crate) fn entry_record_mut(&mut self, id: EntryId) -> Result<&mut JarEntry> {
        self.entries
            .iter_mut()
            .find(|entry| entry.id == id)
            .ok_or(Error::JarEntryIdNotFound(id.0))
    }

    pub(crate) fn source_archive(&self) -> Result<ZipArchive<SourceReader>> {
        let original = self
            .original
            .as_ref()
            .ok_or_else(|| Error::InvalidJar("entry has no original archive backing".to_owned()))?;
        Ok(ZipArchive::new(Cursor::new(Arc::clone(original)))?)
    }
}

impl Default for JarFile {
    fn default() -> Self {
        Self::new()
    }
}

/// Normalizes `java.lang.String`, `java/lang/String`, or a class entry name.
#[must_use]
pub fn normalize_class_entry(class_name: &str) -> String {
    let mut normalized = class_name
        .trim()
        .trim_start_matches(['/', '\\'])
        .replace('\\', "/");
    if is_class_entry(&normalized) {
        normalized.truncate(normalized.len() - CLASS_ENTRY_SUFFIX.len());
    }
    if !normalized.contains('/') {
        normalized = normalized.replace('.', "/");
    }
    normalized.push_str(CLASS_ENTRY_SUFFIX);
    normalized
}

pub(crate) fn read_zip_file<R: Read>(file: &mut R) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    Ok(bytes)
}

/// Returns whether an archive name has the JVM class-entry suffix.
#[must_use]
pub fn is_class_entry(name: &str) -> bool {
    name.ends_with(CLASS_ENTRY_SUFFIX)
}

#[cfg(test)]
mod tests {
    use super::normalize_class_entry;

    #[test]
    fn normalizes_common_class_name_forms() {
        assert_eq!(
            normalize_class_entry("java.lang.String"),
            "java/lang/String.class"
        );
        assert_eq!(
            normalize_class_entry("java/lang/String"),
            "java/lang/String.class"
        );
        assert_eq!(
            normalize_class_entry("java\\lang\\String.class"),
            "java/lang/String.class"
        );
        assert_eq!(
            normalize_class_entry("java.lang.String.class"),
            "java/lang/String.class"
        );
    }
}

//! Stable entry identities and editable ZIP metadata.

use std::io::Read;

use zip::read::ZipFile;
use zip::write::{FullFileOptions, SimpleFileOptions};
use zip::{CompressionMethod, DateTime, HasZipMetadata, System};

use crate::{Error, Result};

/// Stable identity of one entry within an open [`super::JarFile`].
///
/// IDs survive renames and reordering. Removing an entry permanently retires
/// its ID.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct EntryId(pub(crate) u64);

impl EntryId {
    /// Returns the numeric value used to identify this entry.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// Broad kind of a JAR archive entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EntryKind {
    /// A regular archive member containing bytes.
    File,
    /// A directory marker whose name ends in `/`.
    Directory,
    /// A Unix symbolic link whose payload is its UTF-8 target.
    Symlink,
}

/// Placement of an uninterpreted ZIP extra field.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ExtraFieldPlacement {
    /// Write the field to both the local header and central directory.
    LocalAndCentral,
    /// Write the field only to the central directory.
    CentralOnly,
}

/// One uninterpreted ZIP extra field retained across archive rewrites.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtraField {
    /// ZIP extra-field header identifier.
    pub header_id: u16,
    /// Raw field payload, excluding its identifier and length.
    pub data: Vec<u8>,
    /// Header locations in which the field is written.
    pub placement: ExtraFieldPlacement,
}

/// Editable metadata associated with a JAR entry.
///
/// Structural ZIP64 and encryption fields are interpreted by the ZIP reader
/// and are not exposed as opaque extras. All other fields are retained.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntryMetadata {
    /// Compression method used when the entry is next written.
    pub compression: CompressionMethod,
    /// Encoder-specific compression level, or the codec default.
    pub compression_level: Option<i64>,
    /// DOS-compatible last-modified timestamp, when one was present.
    pub last_modified: Option<DateTime>,
    /// Unix permission bits, when supplied by the archive.
    pub unix_mode: Option<u32>,
    /// Platform whose external attributes describe the entry.
    pub system: System,
    /// UTF-8 entry comment.
    pub comment: String,
    /// Uninterpreted non-structural ZIP extra fields.
    pub extra_fields: Vec<ExtraField>,
}

impl Default for EntryMetadata {
    fn default() -> Self {
        Self {
            compression: CompressionMethod::Deflated,
            compression_level: None,
            last_modified: Some(DateTime::default()),
            unix_mode: Some(0o644),
            system: System::Unix,
            comment: String::new(),
            extra_fields: Vec::new(),
        }
    }
}

impl EntryMetadata {
    /// Returns metadata suitable for a portable directory marker.
    #[must_use]
    pub fn directory() -> Self {
        Self {
            compression: CompressionMethod::Stored,
            unix_mode: Some(0o755),
            ..Self::default()
        }
    }

    /// Returns metadata suitable for a portable symbolic link.
    #[must_use]
    pub fn symlink() -> Self {
        Self {
            compression: CompressionMethod::Stored,
            unix_mode: Some(0o777),
            ..Self::default()
        }
    }

    pub(crate) fn from_zip<R: Read>(file: &ZipFile<'_, R>) -> Result<Self> {
        let metadata = file.get_metadata();
        let mut extra_fields = parse_extra_fields(
            metadata.extra_field.as_deref().unwrap_or_default(),
            ExtraFieldPlacement::LocalAndCentral,
        )?;
        extra_fields.extend(parse_extra_fields(
            metadata.central_extra_field.as_deref().unwrap_or_default(),
            ExtraFieldPlacement::CentralOnly,
        )?);
        Ok(Self {
            compression: file.compression(),
            compression_level: metadata.compression_level,
            last_modified: file.last_modified(),
            unix_mode: file.unix_mode(),
            system: metadata.system,
            comment: file.comment().to_owned(),
            extra_fields,
        })
    }

    pub(crate) fn write_options(&self, entry: &str) -> Result<FullFileOptions<'static>> {
        if !matches!(
            self.compression,
            CompressionMethod::Stored | CompressionMethod::Deflated
        ) {
            return Err(Error::UnsupportedJarEntry {
                entry: entry.to_owned(),
                message: format!(
                    "compression method {:?} has no configured encoder",
                    self.compression
                ),
            });
        }
        let mut options = SimpleFileOptions::default()
            .compression_method(self.compression)
            .compression_level(self.compression_level)
            .system(self.system)
            .large_file(false)
            .into_full_options()
            .with_file_comment(self.comment.clone());
        if let Some(last_modified) = self.last_modified {
            if !last_modified.is_valid() {
                return Err(Error::UnsupportedJarEntry {
                    entry: entry.to_owned(),
                    message: "last-modified time is outside the ZIP range".to_owned(),
                });
            }
            options = options.last_modified_time(last_modified);
        }
        if let Some(mode) = self.unix_mode {
            options = options.unix_permissions(mode);
        }
        for field in &self.extra_fields {
            options.add_extra_data(
                field.header_id,
                &field.data,
                field.placement == ExtraFieldPlacement::CentralOnly,
            )?;
        }
        Ok(options)
    }
}

fn parse_extra_fields(raw: &[u8], placement: ExtraFieldPlacement) -> Result<Vec<ExtraField>> {
    let mut fields = Vec::new();
    let mut cursor = 0;
    while cursor < raw.len() {
        if raw.len() - cursor < 4 {
            return Err(Error::InvalidJar(
                "truncated ZIP extra-field header".to_owned(),
            ));
        }
        let header_id = u16::from_le_bytes([raw[cursor], raw[cursor + 1]]);
        let length = usize::from(u16::from_le_bytes([raw[cursor + 2], raw[cursor + 3]]));
        cursor += 4;
        let end = cursor
            .checked_add(length)
            .ok_or_else(|| Error::InvalidJar("ZIP extra-field length overflow".to_owned()))?;
        let data = raw.get(cursor..end).ok_or_else(|| {
            Error::InvalidJar(format!("truncated ZIP extra field 0x{header_id:04x}"))
        })?;
        fields.push(ExtraField {
            header_id,
            data: data.to_vec(),
            placement,
        });
        cursor = end;
    }
    Ok(fields)
}

pub(crate) fn validate_entry_name(name: &str, kind: EntryKind) -> Result<()> {
    if name.is_empty() {
        return Err(Error::invalid_jar_entry_name(name, "name is empty"));
    }
    if name.len() > usize::from(u16::MAX) {
        return Err(Error::invalid_jar_entry_name(
            name,
            "UTF-8 name is longer than the ZIP limit",
        ));
    }
    if name.starts_with('/') || name.starts_with('\\') {
        return Err(Error::invalid_jar_entry_name(
            name,
            "name must be archive-relative",
        ));
    }
    if name.contains('\\') {
        return Err(Error::invalid_jar_entry_name(
            name,
            "JAR names must use forward slashes",
        ));
    }
    if name.contains('\0') {
        return Err(Error::invalid_jar_entry_name(name, "name contains NUL"));
    }
    if name
        .split('/')
        .any(|component| component == "." || component == "..")
    {
        return Err(Error::invalid_jar_entry_name(
            name,
            "dot path components are not allowed",
        ));
    }
    let is_directory_name = name.ends_with('/');
    if (kind == EntryKind::Directory) != is_directory_name {
        let expectation = if kind == EntryKind::Directory {
            "directory names must end in `/`"
        } else {
            "file and symlink names must not end in `/`"
        };
        return Err(Error::invalid_jar_entry_name(name, expectation));
    }
    Ok(())
}

#[derive(Debug, Clone)]
pub(crate) enum EntryData {
    Original(usize),
    Owned(Vec<u8>),
}

#[derive(Debug, Clone)]
pub(crate) struct JarEntry {
    pub(crate) id: EntryId,
    pub(crate) name: String,
    pub(crate) original_name: Option<String>,
    pub(crate) kind: EntryKind,
    pub(crate) metadata: EntryMetadata,
    pub(crate) data: EntryData,
    pub(crate) original_size: u64,
    pub(crate) original_compressed_size: u64,
    pub(crate) original_crc32: u32,
    pub(crate) encrypted: bool,
}

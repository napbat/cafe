//! Stable APK entry identities and editable ZIP metadata.

use std::io::Read;

use zip::read::ZipFile;
use zip::write::{FullFileOptions, SimpleFileOptions};
use zip::{CompressionMethod, DateTime, HasZipMetadata, System};

use super::layout::{
    ENTRY_ID_INCREMENT, INITIAL_ENTRY_ID, PORTABLE_DIRECTORY_MODE, PORTABLE_FILE_MODE,
    PORTABLE_SYMLINK_MODE, ZIP_EXTRA_FIELD_HEADER_SIZE, ZIP_EXTRA_FIELD_ID_OFFSET,
    ZIP_EXTRA_FIELD_LENGTH_OFFSET, ZIP_U16_FIELD_WIDTH, ZIP_U16_MAXIMUM,
};
use crate::{Error, Result};

const ARCHIVE_SEPARATOR: char = '/';
const WINDOWS_SEPARATOR: char = '\\';
const NUL_CHARACTER: char = '\0';
const CURRENT_DIRECTORY_COMPONENT: &str = ".";
const PARENT_DIRECTORY_COMPONENT: &str = "..";
const MAXIMUM_ENTRY_NAME_SIZE: usize = ZIP_U16_MAXIMUM;

/// Stable identity of one entry within an open [`super::ApkFile`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct EntryId(pub(super) u64);

impl EntryId {
    /// Returns the numeric value used to identify this entry.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }

    pub(super) const fn initial() -> Self {
        Self(INITIAL_ENTRY_ID)
    }

    pub(super) fn from_position(position: usize) -> Result<Self> {
        u64::try_from(position)
            .map(Self)
            .map_err(|_| Error::invalid_apk("APK entry position does not fit the ID type"))
    }

    pub(super) const fn next(self) -> Option<Self> {
        match self.0.checked_add(ENTRY_ID_INCREMENT) {
            Some(value) => Some(Self(value)),
            None => None,
        }
    }
}

/// Broad kind of an APK archive member.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EntryKind {
    /// Regular file containing bytes.
    File,
    /// Directory marker whose name ends in `/`.
    Directory,
    /// Unix symbolic link whose payload is its UTF-8 target.
    Symlink,
}

/// Placement of an uninterpreted ZIP extra field.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ExtraFieldPlacement {
    /// Write the field to the local header and central directory.
    LocalAndCentral,
    /// Write the field only to the central directory.
    CentralOnly,
}

/// One uninterpreted ZIP extra field retained during rewrites.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtraField {
    /// ZIP extra-field header identifier.
    pub header_id: u16,
    /// Raw payload excluding identifier and length.
    pub data: Vec<u8>,
    /// Header locations in which this field is written.
    pub placement: ExtraFieldPlacement,
}

/// Editable ZIP metadata associated with an APK member.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntryMetadata {
    /// Compression method used during the next rewrite.
    pub compression: CompressionMethod,
    /// Encoder-specific compression level, or the codec default.
    pub compression_level: Option<i64>,
    /// DOS-compatible last-modified timestamp, when present.
    pub last_modified: Option<DateTime>,
    /// Unix permission bits, when supplied by the archive.
    pub unix_mode: Option<u32>,
    /// Platform interpreting the external attributes.
    pub system: System,
    /// UTF-8 member comment.
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
            unix_mode: Some(PORTABLE_FILE_MODE),
            system: System::Unix,
            comment: String::new(),
            extra_fields: Vec::new(),
        }
    }
}

impl EntryMetadata {
    /// Returns portable metadata for a directory marker.
    #[must_use]
    pub fn directory() -> Self {
        Self {
            compression: CompressionMethod::Stored,
            unix_mode: Some(PORTABLE_DIRECTORY_MODE),
            ..Self::default()
        }
    }

    /// Returns portable metadata for a symbolic link.
    #[must_use]
    pub fn symlink() -> Self {
        Self {
            compression: CompressionMethod::Stored,
            unix_mode: Some(PORTABLE_SYMLINK_MODE),
            ..Self::default()
        }
    }

    pub(super) fn from_zip<R: Read>(file: &ZipFile<'_, R>) -> Result<Self> {
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

    pub(super) fn write_options(&self, entry: &str) -> Result<FullFileOptions<'static>> {
        if !matches!(
            self.compression,
            CompressionMethod::Stored | CompressionMethod::Deflated
        ) {
            return Err(Error::UnsupportedApkEntry {
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
                return Err(Error::UnsupportedApkEntry {
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
    let mut cursor = ZIP_EXTRA_FIELD_ID_OFFSET;
    while cursor < raw.len() {
        let header_end = cursor
            .checked_add(ZIP_EXTRA_FIELD_HEADER_SIZE)
            .ok_or_else(|| Error::invalid_apk("ZIP extra-field header offset overflowed"))?;
        let header = raw
            .get(cursor..header_end)
            .ok_or_else(|| Error::invalid_apk("truncated ZIP extra-field header"))?;
        let header_id = read_extra_u16(header, ZIP_EXTRA_FIELD_ID_OFFSET);
        let length = usize::from(read_extra_u16(header, ZIP_EXTRA_FIELD_LENGTH_OFFSET));
        cursor = header_end;
        let end = cursor
            .checked_add(length)
            .ok_or_else(|| Error::invalid_apk("ZIP extra-field length overflowed"))?;
        let data = raw.get(cursor..end).ok_or_else(|| {
            Error::invalid_apk(format!("truncated ZIP extra field 0x{header_id:04x}"))
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

fn read_extra_u16(header: &[u8], offset: usize) -> u16 {
    let end = offset + ZIP_U16_FIELD_WIDTH;
    u16::from_le_bytes(
        header[offset..end]
            .try_into()
            .expect("extra-field header length was checked"),
    )
}

pub(super) fn validate_entry_name(name: &str, kind: EntryKind) -> Result<()> {
    if name.is_empty() {
        return Err(Error::invalid_apk_entry_name(name, "name is empty"));
    }
    if name.len() > MAXIMUM_ENTRY_NAME_SIZE {
        return Err(Error::invalid_apk_entry_name(
            name,
            "UTF-8 name is longer than the ZIP limit",
        ));
    }
    if name.starts_with(ARCHIVE_SEPARATOR) || name.starts_with(WINDOWS_SEPARATOR) {
        return Err(Error::invalid_apk_entry_name(
            name,
            "name must be archive-relative",
        ));
    }
    if name.contains(WINDOWS_SEPARATOR) {
        return Err(Error::invalid_apk_entry_name(
            name,
            "APK names must use forward slashes",
        ));
    }
    if name.contains(NUL_CHARACTER) {
        return Err(Error::invalid_apk_entry_name(name, "name contains NUL"));
    }
    if name.split(ARCHIVE_SEPARATOR).any(|component| {
        component == CURRENT_DIRECTORY_COMPONENT || component == PARENT_DIRECTORY_COMPONENT
    }) {
        return Err(Error::invalid_apk_entry_name(
            name,
            "dot path components are not allowed",
        ));
    }
    let is_directory_name = name.ends_with(ARCHIVE_SEPARATOR);
    if (kind == EntryKind::Directory) != is_directory_name {
        let expectation = if kind == EntryKind::Directory {
            "directory names must end in `/`"
        } else {
            "file and symlink names must not end in `/`"
        };
        return Err(Error::invalid_apk_entry_name(name, expectation));
    }
    Ok(())
}

#[derive(Debug, Clone)]
pub(super) enum EntryData {
    Original(usize),
    Owned(Vec<u8>),
}

#[derive(Debug, Clone)]
pub(super) struct OriginalEntryStats {
    pub(super) size: u64,
    pub(super) compressed_size: u64,
    pub(super) crc32: u32,
    pub(super) raw_name: Vec<u8>,
    pub(super) flags: u16,
    pub(super) version_made_by: u8,
    pub(super) using_data_descriptor: bool,
    pub(super) external_attributes: u32,
    pub(super) large_file: bool,
}

#[derive(Debug, Clone)]
pub(super) struct ApkEntry {
    pub(super) id: EntryId,
    pub(super) name: String,
    pub(super) original_name: Option<String>,
    pub(super) kind: EntryKind,
    pub(super) metadata: EntryMetadata,
    pub(super) data: EntryData,
    pub(super) original_stats: Option<OriginalEntryStats>,
    pub(super) encrypted: bool,
}

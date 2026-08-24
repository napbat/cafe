//! APK mutation, transactions, and deterministic serialization.

use std::collections::HashSet;
use std::fs;
use std::io::{Cursor, Seek, Write};
use std::path::Path;

use zip::ZipWriter;

use super::entry::{ApkEntry, EntryData, validate_entry_name};
use super::layout::{ZIP_MAXIMUM_COMMENT_SIZE, write_central_directory_offset, zip_sections};
use super::{ApkFile, EntryId, EntryKind, EntryMetadata};
use crate::{Error, Result};

pub use super::signature::SignaturePolicy;

impl ApkFile {
    /// Replaces the raw ZIP archive comment.
    ///
    /// # Errors
    ///
    /// Returns an error when the comment exceeds ZIP's 16-bit length limit.
    pub fn set_archive_comment(&mut self, comment: impl Into<Vec<u8>>) -> Result<()> {
        let comment = comment.into();
        if comment.len() > ZIP_MAXIMUM_COMMENT_SIZE {
            return Err(Error::invalid_apk(
                "archive comment is longer than the ZIP limit",
            ));
        }
        if self.comment != comment {
            self.comment = comment;
            self.dirty = true;
        }
        Ok(())
    }

    /// Returns every stable entry ID in current archive order.
    #[must_use]
    pub fn entry_ids(&self) -> Vec<EntryId> {
        self.entries.iter().map(|entry| entry.id).collect()
    }

    /// Returns an entry's current name.
    ///
    /// # Errors
    ///
    /// Returns an error when the ID is absent.
    pub fn entry_name(&self, id: EntryId) -> Result<&str> {
        Ok(&self.entry_record(id)?.name)
    }

    /// Returns an entry's current kind.
    ///
    /// # Errors
    ///
    /// Returns an error when the ID is absent.
    pub fn entry_kind(&self, id: EntryId) -> Result<EntryKind> {
        Ok(self.entry_record(id)?.kind)
    }

    /// Returns an entry's editable metadata.
    ///
    /// # Errors
    ///
    /// Returns an error when the ID is absent.
    pub fn entry_metadata(&self, id: EntryId) -> Result<&EntryMetadata> {
        Ok(&self.entry_record(id)?.metadata)
    }

    /// Replaces an entry's metadata without changing its name or payload.
    ///
    /// # Errors
    ///
    /// Returns an error when the ID is absent.
    pub fn set_entry_metadata(&mut self, id: EntryId, metadata: EntryMetadata) -> Result<()> {
        let entry = self.entry_record_mut(id)?;
        if entry.metadata != metadata {
            entry.metadata = metadata;
            self.dirty = true;
        }
        Ok(())
    }

    /// Appends a regular file with default metadata.
    ///
    /// # Errors
    ///
    /// Returns an error for an unsafe or duplicate entry name.
    pub fn add_file(
        &mut self,
        name: impl Into<String>,
        data: impl Into<Vec<u8>>,
    ) -> Result<EntryId> {
        self.add_file_with_metadata(name, data, EntryMetadata::default())
    }

    /// Appends a regular file with explicit metadata.
    ///
    /// # Errors
    ///
    /// Returns an error for an unsafe or duplicate entry name.
    pub fn add_file_with_metadata(
        &mut self,
        name: impl Into<String>,
        data: impl Into<Vec<u8>>,
        metadata: EntryMetadata,
    ) -> Result<EntryId> {
        self.insert_entry_record(
            self.entries.len(),
            name.into(),
            EntryKind::File,
            data.into(),
            metadata,
        )
    }

    /// Inserts a regular file at an exact archive-order position.
    ///
    /// # Errors
    ///
    /// Returns an error for an invalid position, unsafe name, or duplicate name.
    pub fn insert_file(
        &mut self,
        position: usize,
        name: impl Into<String>,
        data: impl Into<Vec<u8>>,
        metadata: EntryMetadata,
    ) -> Result<EntryId> {
        self.insert_entry_record(
            position,
            name.into(),
            EntryKind::File,
            data.into(),
            metadata,
        )
    }

    /// Appends a directory marker with portable metadata.
    ///
    /// # Errors
    ///
    /// Returns an error for an unsafe or duplicate entry name.
    pub fn add_directory(&mut self, name: impl Into<String>) -> Result<EntryId> {
        self.insert_entry_record(
            self.entries.len(),
            name.into(),
            EntryKind::Directory,
            Vec::new(),
            EntryMetadata::directory(),
        )
    }

    /// Appends a Unix symbolic-link entry.
    ///
    /// # Errors
    ///
    /// Returns an error for an unsafe or duplicate name.
    pub fn add_symlink(
        &mut self,
        name: impl Into<String>,
        target: impl Into<String>,
    ) -> Result<EntryId> {
        self.insert_entry_record(
            self.entries.len(),
            name.into(),
            EntryKind::Symlink,
            target.into().into_bytes(),
            EntryMetadata::symlink(),
        )
    }

    /// Adds or replaces an unambiguous regular-file entry.
    ///
    /// # Errors
    ///
    /// Returns an error for an unsafe name or duplicate existing names.
    pub fn put_file(
        &mut self,
        name: impl Into<String>,
        data: impl Into<Vec<u8>>,
    ) -> Result<EntryId> {
        let name = name.into();
        let data = data.into();
        match self.entry_ids_named(&name).as_slice() {
            [] => self.add_file(name, data),
            [id] => {
                let id = *id;
                if self.entry_kind(id)? != EntryKind::File {
                    return Err(Error::UnsupportedApkEntry {
                        entry: name,
                        message: "replacement target is not a regular file".to_owned(),
                    });
                }
                self.replace_entry_by_id(id, data)?;
                Ok(id)
            }
            matches => Err(Error::AmbiguousApkEntry {
                name,
                count: matches.len(),
            }),
        }
    }

    /// Replaces an entry's payload while retaining its ID, kind, and metadata.
    ///
    /// # Errors
    ///
    /// Returns an error when the ID is absent or the payload violates its kind.
    pub fn replace_entry_by_id(
        &mut self,
        id: EntryId,
        data: impl Into<Vec<u8>>,
    ) -> Result<Vec<u8>> {
        let data = data.into();
        let previous = self.read_entry_by_id(id)?;
        let entry = self.entry_record_mut(id)?;
        validate_entry_payload(&entry.name, entry.kind, &data)?;
        if previous != data {
            entry.data = EntryData::Owned(data);
            entry.original_stats = None;
            self.dirty = true;
        }
        Ok(previous)
    }

    /// Removes an entry by stable identity and returns its payload.
    ///
    /// # Errors
    ///
    /// Returns an error when the ID is absent or its payload is unreadable.
    pub fn remove_entry_by_id(&mut self, id: EntryId) -> Result<Vec<u8>> {
        let data = self.read_entry_by_id(id)?;
        let position = self
            .entries
            .iter()
            .position(|entry| entry.id == id)
            .ok_or(Error::ApkEntryIdNotFound(id.get()))?;
        self.entries.remove(position);
        self.dirty = true;
        Ok(data)
    }

    /// Removes an unambiguous entry by exact name and returns its payload.
    ///
    /// # Errors
    ///
    /// Returns an error when the name is absent, duplicated, or unreadable.
    pub fn remove_entry(&mut self, name: &str) -> Result<Vec<u8>> {
        self.remove_entry_by_id(self.unique_entry_id(name)?)
    }

    /// Renames an entry while retaining its stable identity.
    ///
    /// # Errors
    ///
    /// Returns an error when the ID is absent or the new name is unsafe or used.
    pub fn rename_entry_by_id(&mut self, id: EntryId, name: impl Into<String>) -> Result<()> {
        let name = name.into();
        let kind = self.entry_kind(id)?;
        validate_entry_name(&name, kind)?;
        self.ensure_name_available(&name, Some(id))?;
        let entry = self.entry_record_mut(id)?;
        if entry.name != name {
            entry.name = name;
            self.dirty = true;
        }
        Ok(())
    }

    /// Renames an unambiguous entry.
    ///
    /// # Errors
    ///
    /// Returns an error when either name is absent, duplicated, unsafe, or used.
    pub fn rename_entry(&mut self, old_name: &str, new_name: impl Into<String>) -> Result<()> {
        self.rename_entry_by_id(self.unique_entry_id(old_name)?, new_name)
    }

    /// Moves an entry to an exact archive-order position.
    ///
    /// # Errors
    ///
    /// Returns an error when the ID or target position is invalid.
    pub fn move_entry(&mut self, id: EntryId, target: usize) -> Result<()> {
        if target >= self.entries.len() {
            return Err(Error::invalid_apk(format!(
                "entry target position {target} is out of bounds"
            )));
        }
        let source = self
            .entries
            .iter()
            .position(|entry| entry.id == id)
            .ok_or(Error::ApkEntryIdNotFound(id.get()))?;
        if source != target {
            let entry = self.entries.remove(source);
            self.entries.insert(target, entry);
            self.dirty = true;
        }
        Ok(())
    }

    /// Sorts entries lexicographically by exact name while retaining IDs.
    pub fn sort_entries_by_name(&mut self) {
        let before = self.entry_ids();
        self.entries
            .sort_by(|left, right| left.name.cmp(&right.name));
        if self.entry_ids() != before {
            self.dirty = true;
        }
    }

    /// Removes all v1 signature files, their manifest, source-stamp entry, and signing block.
    ///
    /// Returns the number of removed ZIP entries plus one when a signing block was removed.
    pub fn strip_signatures(&mut self) -> usize {
        self.strip_signature_artifacts()
    }

    /// Applies an edit transaction and rolls back if the closure or validation fails.
    ///
    /// # Errors
    ///
    /// Returns the closure error or a complete rewrite-validation error.
    pub fn try_edit<T>(&mut self, edit: impl FnOnce(&mut Self) -> Result<T>) -> Result<T> {
        let previous = self.clone();
        let result = edit(self).and_then(|value| self.validate_rewrite_entries().map(|()| value));
        if result.is_err() {
            *self = previous;
        }
        result
    }

    /// Serializes the APK using the safe signature policy.
    ///
    /// Unchanged archives are returned byte-for-byte. A signed mutation is
    /// rejected until the caller explicitly preserves or strips signatures.
    ///
    /// # Errors
    ///
    /// Returns an error for signature policy, entry, codec, or ZIP failures.
    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        self.to_bytes_with_signature_policy(SignaturePolicy::Reject)
    }

    /// Serializes the APK using an explicit signature policy.
    ///
    /// `Preserve` retains signature artifacts as knowingly stale metadata; it
    /// never makes a rewritten APK cryptographically valid.
    ///
    /// # Errors
    ///
    /// Returns an error for signature policy, entry, codec, or ZIP failures.
    pub fn to_bytes_with_signature_policy(&self, policy: SignaturePolicy) -> Result<Vec<u8>> {
        if !self.dirty
            && policy != SignaturePolicy::Strip
            && let Some(original) = &self.original
        {
            return Ok(original.to_vec());
        }
        match policy {
            SignaturePolicy::Reject if self.has_signature_artifacts() => {
                Err(Error::SignedApkMutation)
            }
            SignaturePolicy::Strip => {
                let mut stripped = self.clone();
                stripped.strip_signature_artifacts();
                if !stripped.dirty
                    && let Some(original) = &stripped.original
                {
                    Ok(original.to_vec())
                } else {
                    stripped.serialize_without_signing_block()
                }
            }
            SignaturePolicy::Preserve => {
                let bytes = self.serialize_without_signing_block()?;
                match &self.signing_block {
                    Some(block) => insert_signing_block(bytes, block.as_bytes()),
                    None => Ok(bytes),
                }
            }
            SignaturePolicy::Reject => self.serialize_without_signing_block(),
        }
    }

    /// Writes a serialized APK to a seekable destination.
    ///
    /// # Errors
    ///
    /// Returns an error when serialization or writing fails.
    pub fn write_to<W: Write + Seek>(&self, mut destination: W) -> Result<W> {
        destination.write_all(&self.to_bytes()?)?;
        Ok(destination)
    }

    /// Writes an APK using an explicit signature policy.
    ///
    /// # Errors
    ///
    /// Returns an error when serialization or writing fails.
    pub fn write_to_with_signature_policy<W: Write + Seek>(
        &self,
        mut destination: W,
        policy: SignaturePolicy,
    ) -> Result<W> {
        destination.write_all(&self.to_bytes_with_signature_policy(policy)?)?;
        Ok(destination)
    }

    /// Saves the APK using the safe signature policy.
    ///
    /// # Errors
    ///
    /// Returns an error when serialization or filesystem writing fails.
    pub fn save(&self, path: impl AsRef<Path>) -> Result<()> {
        fs::write(path, self.to_bytes()?)?;
        Ok(())
    }

    /// Saves the APK using an explicit signature policy.
    ///
    /// # Errors
    ///
    /// Returns an error when serialization or filesystem writing fails.
    pub fn save_with_signature_policy(
        &self,
        path: impl AsRef<Path>,
        policy: SignaturePolicy,
    ) -> Result<()> {
        fs::write(path, self.to_bytes_with_signature_policy(policy)?)?;
        Ok(())
    }

    fn insert_entry_record(
        &mut self,
        position: usize,
        name: String,
        kind: EntryKind,
        data: Vec<u8>,
        metadata: EntryMetadata,
    ) -> Result<EntryId> {
        if position > self.entries.len() {
            return Err(Error::invalid_apk(format!(
                "entry insertion position {position} is out of bounds"
            )));
        }
        validate_entry_name(&name, kind)?;
        validate_entry_payload(&name, kind, &data)?;
        self.ensure_name_available(&name, None)?;
        let id = self.next_id;
        self.next_id = self
            .next_id
            .next()
            .ok_or_else(|| Error::invalid_apk("entry ID space is exhausted"))?;
        self.entries.insert(
            position,
            ApkEntry {
                id,
                name,
                original_name: None,
                kind,
                metadata,
                data: EntryData::Owned(data),
                original_stats: None,
                encrypted: false,
            },
        );
        self.dirty = true;
        Ok(id)
    }

    fn ensure_name_available(&self, name: &str, except: Option<EntryId>) -> Result<()> {
        if self
            .entries
            .iter()
            .any(|entry| entry.name == name && Some(entry.id) != except)
        {
            return Err(Error::DuplicateApkEntry(name.to_owned()));
        }
        Ok(())
    }

    fn serialize_without_signing_block(&self) -> Result<Vec<u8>> {
        self.validate_rewrite_entries()?;
        let cursor = Cursor::new(Vec::new());
        let mut writer = ZipWriter::new(cursor);
        writer.set_raw_comment(self.comment.clone().into_boxed_slice())?;
        for entry in &self.entries {
            let options = entry.metadata.write_options(&entry.name)?;
            let data = self.read_entry_by_id(entry.id)?;
            match entry.kind {
                EntryKind::File => {
                    writer.start_file(&entry.name, options)?;
                    writer.write_all(&data)?;
                }
                EntryKind::Directory => writer.add_directory(&entry.name, options)?,
                EntryKind::Symlink => {
                    let target =
                        std::str::from_utf8(&data).map_err(|_| Error::UnsupportedApkEntry {
                            entry: entry.name.clone(),
                            message: "symbolic-link target is not UTF-8".to_owned(),
                        })?;
                    writer.add_symlink(&entry.name, target, options)?;
                }
            }
        }
        Ok(writer.finish()?.into_inner())
    }

    fn validate_rewrite_entries(&self) -> Result<()> {
        let mut names = HashSet::with_capacity(self.entries.len());
        for entry in &self.entries {
            validate_entry_name(&entry.name, entry.kind)?;
            if !names.insert(&entry.name) {
                return Err(Error::DuplicateApkEntry(entry.name.clone()));
            }
            if entry.encrypted {
                return Err(Error::UnsupportedApkEntry {
                    entry: entry.name.clone(),
                    message: "encrypted ZIP members cannot be rewritten".to_owned(),
                });
            }
        }
        Ok(())
    }
}

fn validate_entry_payload(name: &str, kind: EntryKind, data: &[u8]) -> Result<()> {
    if kind == EntryKind::Directory && !data.is_empty() {
        return Err(Error::UnsupportedApkEntry {
            entry: name.to_owned(),
            message: "directory entries cannot contain payload bytes".to_owned(),
        });
    }
    if kind == EntryKind::Symlink && std::str::from_utf8(data).is_err() {
        return Err(Error::UnsupportedApkEntry {
            entry: name.to_owned(),
            message: "symbolic-link target is not UTF-8".to_owned(),
        });
    }
    Ok(())
}

pub(super) fn insert_signing_block(mut bytes: Vec<u8>, block: &[u8]) -> Result<Vec<u8>> {
    let sections = zip_sections(&bytes)?;
    let central_directory = sections.central_directory;
    let new_central_directory = central_directory
        .checked_add(block.len())
        .and_then(|offset| u32::try_from(offset).ok())
        .ok_or_else(|| Error::invalid_apk("preserved signing block requires ZIP64"))?;
    bytes.splice(central_directory..central_directory, block.iter().copied());
    let new_end_record = sections
        .end_of_central_directory
        .checked_add(block.len())
        .ok_or_else(|| Error::invalid_apk("preserved signing-block offset overflowed"))?;
    write_central_directory_offset(&mut bytes, new_end_record, new_central_directory)?;
    Ok(bytes)
}

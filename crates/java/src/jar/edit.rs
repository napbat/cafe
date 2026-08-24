//! Archive mutation and deterministic serialization.

use std::collections::HashSet;
use std::fs;
use std::io::{Cursor, Seek, Write};
use std::path::Path;

use zip::ZipWriter;

use crate::classfile::ClassFile;
use crate::{Error, Result};

use super::entry::{EntryData, JarEntry, validate_entry_name};
use super::layout::ZIP_U16_MAXIMUM;
use super::reader::EntryReader;
use super::{EntryId, EntryKind, EntryMetadata, JarFile, normalize_class_entry};

/// Policy used when a rewrite encounters JAR signature artifacts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum SignaturePolicy {
    /// Reject rewrites that would silently invalidate signatures.
    #[default]
    Reject,
    /// Retain signature files even though the rewrite may invalidate them.
    Preserve,
    /// Remove signature files and manifest digest attributes before writing.
    Strip,
}

impl JarFile {
    /// Replaces the raw ZIP archive comment.
    ///
    /// # Errors
    ///
    /// Returns an error when the comment exceeds the ZIP 16-bit length limit.
    pub fn set_archive_comment(&mut self, comment: impl Into<Vec<u8>>) -> Result<()> {
        let comment = comment.into();
        if comment.len() > ZIP_U16_MAXIMUM {
            return Err(Error::InvalidJar(
                "archive comment is longer than the ZIP limit".to_owned(),
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
    /// Returns an error if the ID is absent.
    pub fn entry_name(&self, id: EntryId) -> Result<&str> {
        Ok(&self.entry_record(id)?.name)
    }

    /// Returns an entry's current kind.
    ///
    /// # Errors
    ///
    /// Returns an error if the ID is absent.
    pub fn entry_kind(&self, id: EntryId) -> Result<EntryKind> {
        Ok(self.entry_record(id)?.kind)
    }

    /// Returns an entry's editable metadata.
    ///
    /// # Errors
    ///
    /// Returns an error if the ID is absent.
    pub fn entry_metadata(&self, id: EntryId) -> Result<&EntryMetadata> {
        Ok(&self.entry_record(id)?.metadata)
    }

    /// Replaces an entry's metadata without changing its name or payload.
    ///
    /// # Errors
    ///
    /// Returns an error if the ID is absent.
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
    /// Returns an error for an unsafe or duplicate name.
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
    /// Returns an error for an unsafe or duplicate name.
    pub fn add_file_with_metadata(
        &mut self,
        name: impl Into<String>,
        data: impl Into<Vec<u8>>,
        metadata: EntryMetadata,
    ) -> Result<EntryId> {
        let index = self.entries.len();
        self.insert_entry_record(index, name.into(), EntryKind::File, data.into(), metadata)
    }

    /// Inserts a regular file at an exact archive-order position.
    ///
    /// # Errors
    ///
    /// Returns an error when the index is out of bounds or the name is unsafe
    /// or duplicated.
    pub fn insert_file(
        &mut self,
        index: usize,
        name: impl Into<String>,
        data: impl Into<Vec<u8>>,
        metadata: EntryMetadata,
    ) -> Result<EntryId> {
        self.insert_entry_record(index, name.into(), EntryKind::File, data.into(), metadata)
    }

    /// Appends a directory marker with portable default metadata.
    ///
    /// # Errors
    ///
    /// Returns an error for an unsafe or duplicate name.
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

    /// Adds a parsed class under the internal name it declares.
    ///
    /// # Errors
    ///
    /// Returns an error if the class cannot be assembled or its entry name is
    /// already present.
    pub fn add_class(&mut self, class: &ClassFile) -> Result<EntryId> {
        let name = normalize_class_entry(class.class_name()?);
        self.add_file(name, class.to_bytes()?)
    }

    /// Appends an entry of any supported kind with explicit metadata.
    ///
    /// Directory payloads must be empty, and symbolic-link payloads must be a
    /// UTF-8 target when the archive is serialized.
    ///
    /// # Errors
    ///
    /// Returns an error for an unsafe or duplicate name or a non-empty
    /// directory payload.
    pub fn add_entry_with_metadata(
        &mut self,
        name: impl Into<String>,
        kind: EntryKind,
        data: impl Into<Vec<u8>>,
        metadata: EntryMetadata,
    ) -> Result<EntryId> {
        self.insert_entry_with_metadata(self.entries.len(), name, kind, data, metadata)
    }

    /// Inserts an entry of any supported kind at an archive-order position.
    ///
    /// # Errors
    ///
    /// Returns an error when the index is out of bounds, the name is unsafe or
    /// duplicate, or a directory payload is non-empty.
    pub fn insert_entry_with_metadata(
        &mut self,
        index: usize,
        name: impl Into<String>,
        kind: EntryKind,
        data: impl Into<Vec<u8>>,
        metadata: EntryMetadata,
    ) -> Result<EntryId> {
        self.insert_entry_record(index, name.into(), kind, data.into(), metadata)
    }

    /// Copies one entry from another JAR and appends it under a new name.
    ///
    /// The uncompressed payload, kind, and editable metadata are copied.
    ///
    /// # Errors
    ///
    /// Returns an error if the source ID or payload is unavailable, or the
    /// destination name is unsafe or duplicated.
    pub fn copy_entry_from(
        &mut self,
        source: &JarFile,
        source_id: EntryId,
        destination_name: impl Into<String>,
    ) -> Result<EntryId> {
        let source_entry = source.entry_record(source_id)?;
        self.add_entry_with_metadata(
            destination_name,
            source_entry.kind,
            source.read_entry_by_id(source_id)?,
            source_entry.metadata.clone(),
        )
    }

    /// Adds or replaces a parsed class under the internal name it declares.
    ///
    /// # Errors
    ///
    /// Returns an error if the class cannot be assembled or a duplicate input
    /// name makes replacement ambiguous.
    pub fn put_class(&mut self, class: &ClassFile) -> Result<EntryId> {
        let name = normalize_class_entry(class.class_name()?);
        let bytes = class.to_bytes()?;
        match self.entry_ids_named(&name).as_slice() {
            [] => self.add_file(name, bytes),
            [id] => {
                let id = *id;
                self.replace_entry_by_id(id, bytes)?;
                Ok(id)
            }
            ids => Err(Error::AmbiguousJarEntry {
                name,
                count: ids.len(),
            }),
        }
    }

    /// Adds a file or replaces the uniquely named existing entry.
    ///
    /// Existing metadata and archive position are retained on replacement.
    ///
    /// # Errors
    ///
    /// Returns an error for an unsafe name or an ambiguous existing name.
    pub fn put_file(
        &mut self,
        name: impl Into<String>,
        data: impl Into<Vec<u8>>,
    ) -> Result<EntryId> {
        let name = name.into();
        validate_entry_name(&name, EntryKind::File)?;
        let data = data.into();
        match self.entry_ids_named(&name).as_slice() {
            [] => self.add_file(name, data),
            [id] => {
                let id = *id;
                self.replace_entry_by_id(id, data)?;
                Ok(id)
            }
            ids => Err(Error::AmbiguousJarEntry {
                name,
                count: ids.len(),
            }),
        }
    }

    /// Replaces the payload of a uniquely named file or symlink.
    ///
    /// # Errors
    ///
    /// Returns an error if the name is absent or ambiguous, or identifies a
    /// directory.
    pub fn replace_entry(&mut self, name: &str, data: impl Into<Vec<u8>>) -> Result<()> {
        let id = self.unique_entry_id(name)?;
        self.replace_entry_by_id(id, data)
    }

    /// Replaces the payload of a file or symlink by stable identity.
    ///
    /// # Errors
    ///
    /// Returns an error if the ID is absent or identifies a directory.
    pub fn replace_entry_by_id(&mut self, id: EntryId, data: impl Into<Vec<u8>>) -> Result<()> {
        let data = data.into();
        let entry = self.entry_record_mut(id)?;
        if entry.kind == EntryKind::Directory {
            return Err(Error::UnsupportedJarEntry {
                entry: entry.name.clone(),
                message: "directory entries cannot contain payload bytes".to_owned(),
            });
        }
        if entry.kind == EntryKind::Symlink && std::str::from_utf8(&data).is_err() {
            return Err(Error::UnsupportedJarEntry {
                entry: entry.name.clone(),
                message: "symbolic-link target is not UTF-8".to_owned(),
            });
        }
        entry.data = EntryData::Owned(data);
        entry.original_stats = None;
        entry.encrypted = false;
        self.dirty = true;
        Ok(())
    }

    /// Atomically replaces an entry's name, kind, payload, and metadata.
    ///
    /// This can convert between files, directory markers, and symbolic links
    /// without changing the stable entry ID or archive position.
    ///
    /// # Errors
    ///
    /// Returns an error if the ID is absent, the new name is unsafe or
    /// duplicated, a directory payload is non-empty, or a symbolic-link target
    /// is not UTF-8.
    pub fn replace_entry_definition(
        &mut self,
        id: EntryId,
        name: impl Into<String>,
        kind: EntryKind,
        data: impl Into<Vec<u8>>,
        metadata: EntryMetadata,
    ) -> Result<()> {
        self.entry_record(id)?;
        let name = name.into();
        let data = data.into();
        validate_new_entry(&name, kind, &data)?;
        self.ensure_name_available(&name, Some(id))?;
        let entry = self.entry_record_mut(id)?;
        entry.name = name;
        entry.kind = kind;
        entry.metadata = metadata;
        entry.data = EntryData::Owned(data);
        entry.original_stats = None;
        entry.encrypted = false;
        self.dirty = true;
        Ok(())
    }

    /// Renames a uniquely named entry.
    ///
    /// # Errors
    ///
    /// Returns an error if the old name is absent or ambiguous, or the new
    /// name is unsafe or already used.
    pub fn rename_entry(&mut self, old_name: &str, new_name: impl Into<String>) -> Result<()> {
        let id = self.unique_entry_id(old_name)?;
        self.rename_entry_by_id(id, new_name)
    }

    /// Renames an entry by stable identity.
    ///
    /// # Errors
    ///
    /// Returns an error if the ID is absent or the new name is unsafe or
    /// already used.
    pub fn rename_entry_by_id(&mut self, id: EntryId, new_name: impl Into<String>) -> Result<()> {
        let new_name = new_name.into();
        let entry = self.entry_record(id)?;
        validate_entry_name(&new_name, entry.kind)?;
        if entry.name == new_name {
            return Ok(());
        }
        self.ensure_name_available(&new_name, Some(id))?;
        self.entry_record_mut(id)?.name = new_name;
        self.dirty = true;
        Ok(())
    }

    /// Removes and returns the payload of a uniquely named entry.
    ///
    /// # Errors
    ///
    /// Returns an error if the name is absent or ambiguous, or its payload
    /// cannot be read.
    pub fn remove_entry(&mut self, name: &str) -> Result<Vec<u8>> {
        let id = self.unique_entry_id(name)?;
        self.remove_entry_by_id(id)
    }

    /// Removes and returns an entry payload by stable identity.
    ///
    /// # Errors
    ///
    /// Returns an error if the ID is absent or its payload cannot be read.
    pub fn remove_entry_by_id(&mut self, id: EntryId) -> Result<Vec<u8>> {
        let data = self.read_entry_by_id(id)?;
        let index = self
            .entries
            .iter()
            .position(|entry| entry.id == id)
            .ok_or(Error::JarEntryIdNotFound(id.0))?;
        self.entries.remove(index);
        self.dirty = true;
        Ok(data)
    }

    /// Removes every entry with an exact name, including duplicates.
    ///
    /// Payloads are returned in archive order.
    ///
    /// # Errors
    ///
    /// Returns an error if any matching original payload cannot be read. No
    /// entry is removed unless every payload is read successfully.
    pub fn remove_entries_named(&mut self, name: &str) -> Result<Vec<Vec<u8>>> {
        let ids = self.entry_ids_named(name);
        let mut reader = EntryReader::new(self);
        let payloads: Vec<_> = ids
            .iter()
            .map(|id| {
                let entry = self.entry_record(*id)?;
                reader.read(entry)
            })
            .collect::<Result<_>>()?;
        if !ids.is_empty() {
            let ids: HashSet<_> = ids.into_iter().collect();
            self.entries.retain(|entry| !ids.contains(&entry.id));
            self.dirty = true;
        }
        Ok(payloads)
    }

    /// Removes every entry while retaining the archive comment.
    pub fn clear_entries(&mut self) {
        if !self.entries.is_empty() {
            self.entries.clear();
            self.dirty = true;
        }
    }

    /// Moves an entry to an exact archive-order index.
    ///
    /// # Errors
    ///
    /// Returns an error if the ID or destination index is out of bounds.
    pub fn move_entry(&mut self, id: EntryId, new_index: usize) -> Result<()> {
        if new_index >= self.entries.len() {
            return Err(Error::InvalidJar(format!(
                "entry destination index {new_index} is out of bounds"
            )));
        }
        let old_index = self
            .entries
            .iter()
            .position(|entry| entry.id == id)
            .ok_or(Error::JarEntryIdNotFound(id.0))?;
        if old_index != new_index {
            let entry = self.entries.remove(old_index);
            self.entries.insert(new_index, entry);
            self.dirty = true;
        }
        Ok(())
    }

    /// Reorders entries according to an exact permutation of current IDs.
    ///
    /// # Errors
    ///
    /// Returns an error if an ID is missing, repeated, or foreign.
    pub fn reorder_entries(&mut self, order: &[EntryId]) -> Result<()> {
        if order.len() != self.entries.len() {
            return Err(Error::InvalidJar(
                "entry order is not a complete permutation".to_owned(),
            ));
        }
        let unique: HashSet<_> = order.iter().copied().collect();
        if unique.len() != order.len()
            || self.entries.iter().any(|entry| !unique.contains(&entry.id))
        {
            return Err(Error::InvalidJar(
                "entry order contains a repeated or foreign ID".to_owned(),
            ));
        }
        if self.entry_ids() == order {
            return Ok(());
        }
        let mut reordered = Vec::with_capacity(order.len());
        for id in order {
            let index = self
                .entries
                .iter()
                .position(|entry| entry.id == *id)
                .ok_or(Error::JarEntryIdNotFound(id.0))?;
            reordered.push(self.entries.remove(index));
        }
        self.entries = reordered;
        self.dirty = true;
        Ok(())
    }

    /// Sorts entries lexicographically by their exact UTF-8 names.
    pub fn sort_entries_by_name(&mut self) {
        let before = self.entry_ids();
        self.entries
            .sort_by(|left, right| left.name.cmp(&right.name));
        if self.entry_ids() != before {
            self.dirty = true;
        }
    }

    /// Serializes the JAR using the safe signature policy.
    ///
    /// Unchanged input archives are returned byte-for-byte. Rewriting a signed
    /// archive is rejected unless an explicit policy is selected.
    ///
    /// # Errors
    ///
    /// Returns an error for unsafe names, duplicate names, unsupported entry
    /// encodings, invalid signatures policy, or ZIP write failures.
    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        self.to_bytes_with_signature_policy(SignaturePolicy::Reject)
    }

    /// Serializes the JAR using an explicit signature policy.
    ///
    /// # Errors
    ///
    /// Returns an error for unsafe names, duplicate names, unsupported entry
    /// encodings, or ZIP write failures.
    pub fn to_bytes_with_signature_policy(&self, policy: SignaturePolicy) -> Result<Vec<u8>> {
        if !self.dirty
            && policy != SignaturePolicy::Strip
            && let Some(original) = &self.original
        {
            return Ok(original.to_vec());
        }
        match policy {
            SignaturePolicy::Reject if self.has_signature_artifacts() => {
                Err(Error::SignedJarMutation)
            }
            SignaturePolicy::Strip => {
                let mut stripped = self.clone();
                stripped.strip_signatures()?;
                if !stripped.dirty
                    && let Some(original) = &stripped.original
                {
                    Ok(original.to_vec())
                } else {
                    stripped.serialize_unchecked()
                }
            }
            SignaturePolicy::Reject | SignaturePolicy::Preserve => self.serialize_unchecked(),
        }
    }

    /// Writes a serialized JAR to a seekable destination.
    ///
    /// # Errors
    ///
    /// Returns an error if serialization or writing fails.
    pub fn write_to<W: Write + Seek>(&self, mut destination: W) -> Result<W> {
        destination.write_all(&self.to_bytes()?)?;
        Ok(destination)
    }

    /// Writes a serialized JAR using an explicit signature policy.
    ///
    /// # Errors
    ///
    /// Returns an error if serialization or writing fails.
    pub fn write_to_with_signature_policy<W: Write + Seek>(
        &self,
        mut destination: W,
        policy: SignaturePolicy,
    ) -> Result<W> {
        destination.write_all(&self.to_bytes_with_signature_policy(policy)?)?;
        Ok(destination)
    }

    /// Saves the JAR to a filesystem path using the safe signature policy.
    ///
    /// Serialization completes in memory before the destination is opened.
    ///
    /// # Errors
    ///
    /// Returns an error if serialization or filesystem writing fails.
    pub fn save(&self, path: impl AsRef<Path>) -> Result<()> {
        fs::write(path, self.to_bytes()?)?;
        Ok(())
    }

    /// Saves the JAR using an explicit signature policy.
    ///
    /// # Errors
    ///
    /// Returns an error if serialization or filesystem writing fails.
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
        index: usize,
        name: String,
        kind: EntryKind,
        data: Vec<u8>,
        metadata: EntryMetadata,
    ) -> Result<EntryId> {
        if index > self.entries.len() {
            return Err(Error::InvalidJar(format!(
                "entry insertion index {index} is out of bounds"
            )));
        }
        validate_new_entry(&name, kind, &data)?;
        self.ensure_name_available(&name, None)?;
        let id = self.next_id;
        self.next_id = self
            .next_id
            .next()
            .ok_or_else(|| Error::InvalidJar("entry ID space exhausted".to_owned()))?;
        self.entries.insert(
            index,
            JarEntry {
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
            return Err(Error::DuplicateJarEntry(name.to_owned()));
        }
        Ok(())
    }

    fn serialize_unchecked(&self) -> Result<Vec<u8>> {
        self.validate_rewrite_entries()?;
        let cursor = Cursor::new(Vec::new());
        let mut writer = ZipWriter::new(cursor);
        let mut reader = EntryReader::new(self);
        writer.set_raw_comment(self.comment.clone().into_boxed_slice())?;
        for entry in &self.entries {
            let options = entry.metadata.write_options(&entry.name)?;
            let data = reader
                .read(entry)
                .map_err(|error| error.in_jar_entry(entry.name.clone()))?;
            match entry.kind {
                EntryKind::File => {
                    writer.start_file(&entry.name, options)?;
                    writer.write_all(&data)?;
                }
                EntryKind::Directory => writer.add_directory(&entry.name, options)?,
                EntryKind::Symlink => {
                    let target =
                        std::str::from_utf8(&data).map_err(|_| Error::UnsupportedJarEntry {
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
                return Err(Error::DuplicateJarEntry(entry.name.clone()));
            }
            if entry.encrypted {
                return Err(Error::UnsupportedJarEntry {
                    entry: entry.name.clone(),
                    message: "encrypted ZIP members cannot be rewritten".to_owned(),
                });
            }
        }
        Ok(())
    }
}

fn validate_new_entry(name: &str, kind: EntryKind, data: &[u8]) -> Result<()> {
    validate_entry_name(name, kind)?;
    if kind == EntryKind::Directory && !data.is_empty() {
        return Err(Error::UnsupportedJarEntry {
            entry: name.to_owned(),
            message: "directory entries cannot contain payload bytes".to_owned(),
        });
    }
    if kind == EntryKind::Symlink && std::str::from_utf8(data).is_err() {
        return Err(Error::UnsupportedJarEntry {
            entry: name.to_owned(),
            message: "symbolic-link target is not UTF-8".to_owned(),
        });
    }
    Ok(())
}

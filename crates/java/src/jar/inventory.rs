//! Archive-entry and class inventory models and operations.

use crate::Result;
use crate::classfile::{ClassAccessFlags, ClassFile};

use super::{JarFile, is_class_entry, read_zip_file};

/// Broad kind of a JAR archive entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EntryKind {
    /// A regular archive member containing bytes.
    File,
    /// A directory marker.
    Directory,
}

/// Metadata for one archive entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntryInfo {
    /// Entry name using the JAR's forward-slash separator.
    pub name: String,
    /// Uncompressed byte length.
    pub size: u64,
    /// Compressed byte length.
    pub compressed_size: u64,
    /// Whether this member is a file or directory marker.
    pub kind: EntryKind,
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
}

/// Metadata obtained by parsing one class entry during archive enumeration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClassSummary {
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
    /// Collects metadata for every archive entry.
    ///
    /// # Errors
    ///
    /// Returns an error if ZIP metadata for an entry cannot be read.
    pub fn entries(&mut self) -> Result<Vec<EntryInfo>> {
        let mut entries = Vec::with_capacity(self.archive.len());
        for index in 0..self.archive.len() {
            let file = self.archive.by_index(index)?;
            entries.push(EntryInfo {
                name: file.name().to_owned(),
                size: file.size(),
                compressed_size: file.compressed_size(),
                kind: if file.is_dir() {
                    EntryKind::Directory
                } else {
                    EntryKind::File
                },
            });
        }
        Ok(entries)
    }

    /// Returns all `.class` entry names in archive order.
    #[must_use]
    pub fn class_entry_names(&self) -> Vec<String> {
        self.archive
            .file_names()
            .filter(|name| is_class_entry(name))
            .map(str::to_owned)
            .collect()
    }

    /// Returns the number of `.class` entries without reading their payloads.
    #[must_use]
    pub fn class_entry_count(&self) -> usize {
        self.archive
            .file_names()
            .filter(|name| is_class_entry(name))
            .count()
    }

    /// Parses and returns metadata for every class entry in archive order.
    ///
    /// # Errors
    ///
    /// Returns an error identifying the first unreadable or invalid class entry.
    pub fn class_summaries(&mut self) -> Result<Vec<ClassSummary>> {
        let mut summaries = Vec::new();
        for index in 0..self.archive.len() {
            let (entry_name, size, bytes) = {
                let mut file = self.archive.by_index(index)?;
                if file.is_dir() || !is_class_entry(file.name()) {
                    continue;
                }
                let entry_name = file.name().to_owned();
                let size = file.size();
                let bytes = read_zip_file(&mut file)?;
                (entry_name, size, bytes)
            };
            let class =
                ClassFile::parse(&bytes).map_err(|error| error.in_jar_entry(entry_name.clone()))?;
            let internal_name = class
                .class_name()
                .map_err(|error| error.in_jar_entry(entry_name.clone()))?
                .to_owned();
            summaries.push(ClassSummary {
                entry_name,
                internal_name,
                minor_version: class.minor_version,
                major_version: class.major_version,
                access_flags: class.access_flags,
                fields: class.fields.len(),
                methods: class.methods.len(),
                size,
            });
        }
        Ok(summaries)
    }
}

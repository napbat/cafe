//! Reading and validating class files inside JAR archives.

use std::fs::File;
use std::io::Read;
use std::path::Path;

use zip::ZipArchive;
use zip::result::ZipError;

use crate::classfile::ClassFile;
use crate::{Error, Result};

mod discovery;
mod inventory;
mod validation;

pub use self::discovery::{Traversal, discover_jars, is_jar_path};
pub use self::inventory::{ClassSummary, EntryInfo, EntryKind};
pub use self::validation::ValidationReport;

/// File-name suffix used by JVM class entries in a JAR.
pub const CLASS_ENTRY_SUFFIX: &str = ".class";

/// An open JAR file backed by a seekable ZIP archive.
pub struct JarFile {
    archive: ZipArchive<File>,
}

impl JarFile {
    /// Opens a JAR file from disk and reads its ZIP directory.
    ///
    /// # Errors
    ///
    /// Returns an error if the path cannot be opened or is not a readable ZIP.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let file = File::open(path)?;
        let archive = ZipArchive::new(file)?;
        Ok(Self { archive })
    }

    /// Returns the number of all entries, including resources and directories.
    #[must_use]
    pub fn len(&self) -> usize {
        self.archive.len()
    }

    /// Returns whether the JAR has no entries.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.archive.is_empty()
    }

    /// Reads an entry's complete uncompressed contents.
    ///
    /// # Errors
    ///
    /// Returns an error if the entry is absent, unsupported, or cannot be read.
    pub fn read_entry(&mut self, name: &str) -> Result<Vec<u8>> {
        let mut file = self.archive.by_name(name)?;
        read_zip_file(&mut file)
    }

    /// Resolves a dotted, internal, or `.class` name and parses that class.
    ///
    /// # Errors
    ///
    /// Returns an error if the class is absent, cannot be decompressed, or is an
    /// invalid class file.
    pub fn read_class(&mut self, class_name: &str) -> Result<ClassFile> {
        let entry_name = normalize_class_entry(class_name);
        let bytes = match self.archive.by_name(&entry_name) {
            Ok(mut file) => read_zip_file(&mut file)?,
            Err(ZipError::FileNotFound) => return Err(Error::ClassNotFound(entry_name)),
            Err(error) => return Err(error.into()),
        };
        ClassFile::parse(&bytes).map_err(|error| error.in_jar_entry(entry_name))
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

fn read_zip_file<R: Read>(file: &mut R) -> Result<Vec<u8>> {
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

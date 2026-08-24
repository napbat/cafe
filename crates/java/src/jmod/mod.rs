//! Read-only ingestion of JDK JMOD module archives.
//!
//! A JMOD is a versioned four-byte header followed by a ZIP archive whose
//! first path component identifies a module section. Class payloads remain
//! ordinary JVM class files and are parsed by the class-file frontend.

use std::fs;
use std::path::Path;

use crate::classfile::ClassFile;
use crate::jar::{ClassEntry, ClassVisitControl, EntryInfo, JarFile};
use crate::{Error, Result};

/// Fixed width of the JMOD header preceding its ZIP data.
pub const JMOD_HEADER_SIZE: usize = 4;
/// Current JMOD magic and format-version bytes.
pub const JMOD_MAGIC: [u8; JMOD_HEADER_SIZE] = [b'J', b'M', 1, 0];
/// Conventional JMOD file-name suffix.
pub const JMOD_EXTENSION: &str = "jmod";

const CLASSES_PREFIX: &str = "classes/";

/// Top-level content section of a JMOD archive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum JmodSection {
    /// JVM class files and class-path resources.
    Classes,
    /// Native commands installed into `bin`.
    Commands,
    /// Configuration files.
    Config,
    /// Native header files.
    Include,
    /// Legal notices and licenses.
    Legal,
    /// Native libraries.
    Libraries,
    /// Manual pages.
    ManPages,
    /// A section name not defined by the current JMOD contract.
    Unknown,
}

impl JmodSection {
    /// Classifies the first archive path component.
    #[must_use]
    pub fn from_name(name: &str) -> Self {
        match name {
            "classes" => Self::Classes,
            "bin" => Self::Commands,
            "conf" => Self::Config,
            "include" => Self::Include,
            "legal" => Self::Legal,
            "lib" => Self::Libraries,
            "man" => Self::ManPages,
            _ => Self::Unknown,
        }
    }
}

/// Inventory record for one physical JMOD member.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JmodEntry {
    /// Physical ZIP member metadata.
    pub archive: EntryInfo,
    /// Section selected by the first path component.
    pub section: JmodSection,
    /// Name relative to the section, or an empty string for its directory marker.
    pub name: String,
}

/// Borrowed metadata for one class in the JMOD `classes` section.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JmodClassEntry<'a> {
    /// Exact physical archive name, including `classes/`.
    pub physical_name: &'a str,
    /// Class entry name relative to the `classes` section.
    pub name: &'a str,
    /// Uncompressed class-file size.
    pub size: u64,
}

/// Read-only JMOD container retaining its exact original bytes.
#[derive(Debug, Clone)]
pub struct JmodFile {
    archive: JarFile,
}

impl JmodFile {
    /// Opens a JMOD from disk.
    ///
    /// # Errors
    ///
    /// Returns an error for unreadable input, an unsupported JMOD header, or
    /// malformed ZIP metadata.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        Self::from_bytes(fs::read(path)?)
    }

    /// Parses a complete JMOD image.
    ///
    /// # Errors
    ///
    /// Returns an error when the versioned header or following ZIP archive is
    /// malformed.
    pub fn from_bytes(bytes: impl Into<Vec<u8>>) -> Result<Self> {
        let bytes = bytes.into();
        let header = bytes
            .get(..JMOD_HEADER_SIZE)
            .ok_or_else(|| Error::invalid_jmod(0, "truncated JMOD header"))?;
        if header != JMOD_MAGIC {
            return Err(Error::invalid_jmod(0, "unsupported JMOD magic or version"));
        }
        Ok(Self {
            archive: JarFile::from_bytes(bytes)?,
        })
    }

    /// Returns the exact input bytes.
    #[must_use]
    pub fn original_bytes(&self) -> &[u8] {
        self.archive.original_bytes().unwrap_or_default()
    }

    /// Returns the underlying read-only ZIP/JAR view.
    #[must_use]
    pub const fn archive(&self) -> &JarFile {
        &self.archive
    }

    /// Builds a deterministic inventory in physical archive order.
    ///
    /// # Errors
    ///
    /// Returns an error if cached ZIP inventory cannot be produced.
    pub fn entries(&self) -> Result<Vec<JmodEntry>> {
        Ok(self
            .archive
            .entries()?
            .into_iter()
            .map(|archive| {
                let (section_name, name) =
                    archive.name.split_once('/').unwrap_or((&archive.name, ""));
                JmodEntry {
                    section: JmodSection::from_name(section_name),
                    name: name.to_owned(),
                    archive,
                }
            })
            .collect())
    }

    /// Parses one class by its name relative to the `classes` section.
    ///
    /// # Errors
    ///
    /// Returns an error if the entry is absent, ambiguous, unreadable, or not
    /// a valid class file.
    pub fn read_class(&self, name: &str) -> Result<ClassFile> {
        let logical = crate::jar::normalize_class_entry(name);
        let physical = format!("{CLASSES_PREFIX}{logical}");
        let bytes = self.archive.read_entry(&physical)?;
        ClassFile::parse(&bytes).map_err(|error| error.in_jar_entry(physical))
    }

    /// Visits selected classes with the JAR frontend's single ZIP reader.
    ///
    /// # Errors
    ///
    /// Returns the first selected payload, class-file, or visitor error.
    pub fn visit_classes<S, V, E>(&self, mut select: S, mut visit: V) -> std::result::Result<(), E>
    where
        for<'entry> S: FnMut(JmodClassEntry<'entry>) -> bool,
        for<'entry> V:
            FnMut(JmodClassEntry<'entry>, ClassFile) -> std::result::Result<ClassVisitControl, E>,
        E: From<Error>,
    {
        self.archive.visit_classes(
            |entry| class_entry(entry).is_some_and(&mut select),
            |entry, class| {
                let Some(entry) = class_entry(entry) else {
                    return Ok(ClassVisitControl::Continue);
                };
                visit(entry, class)
            },
        )
    }

    /// Visits selected raw class payloads through one underlying ZIP reader.
    ///
    /// Payload failures are delivered to the visitor so aggregate consumers
    /// can continue after a malformed member.
    ///
    /// # Errors
    ///
    /// Returns only a visitor error; individual member errors are visitor
    /// values qualified with the physical `classes/` entry name.
    pub fn visit_class_bytes<S, V, E>(
        &self,
        mut select: S,
        mut visit: V,
    ) -> std::result::Result<(), E>
    where
        for<'entry> S: FnMut(JmodClassEntry<'entry>) -> bool,
        for<'entry, 'payload> V: FnMut(
            JmodClassEntry<'entry>,
            std::result::Result<&'payload [u8], Error>,
        ) -> std::result::Result<ClassVisitControl, E>,
    {
        self.archive.visit_class_bytes(
            |entry| class_entry(entry).is_some_and(&mut select),
            |entry, bytes| {
                let Some(entry) = class_entry(entry) else {
                    return Ok(ClassVisitControl::Continue);
                };
                visit(entry, bytes)
            },
        )
    }
}

fn class_entry(entry: ClassEntry<'_>) -> Option<JmodClassEntry<'_>> {
    let name = entry.name.strip_prefix(CLASSES_PREFIX)?;
    Some(JmodClassEntry {
        physical_name: entry.name,
        name,
        size: entry.size,
    })
}

/// Returns whether a path has the conventional case-insensitive JMOD suffix.
#[must_use]
pub fn is_jmod_path(path: impl AsRef<Path>) -> bool {
    path.as_ref()
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case(JMOD_EXTENSION))
}

#[cfg(test)]
mod tests {
    use crate::classfile::{ClassAccessFlags, ClassFile, JAVA_8_MAJOR_VERSION};
    use crate::jar::JarFile;

    use super::*;

    #[test]
    fn reads_prefixed_jmod_classes() -> Result<()> {
        let class = ClassFile::new(
            JAVA_8_MAJOR_VERSION,
            "sample/Thing",
            Some("java/lang/Object"),
            ClassAccessFlags::PUBLIC,
        )?;
        let mut archive = JarFile::new();
        archive.add_file("classes/sample/Thing.class", class.to_bytes()?)?;
        archive.add_file("legal/LICENSE", b"license".to_vec())?;
        let mut bytes = JMOD_MAGIC.to_vec();
        bytes.extend_from_slice(&archive.to_bytes()?);

        let jmod = JmodFile::from_bytes(bytes)?;
        assert_eq!(
            jmod.read_class("sample.Thing")?.class_name()?,
            "sample/Thing"
        );
        assert_eq!(jmod.entries()?[1].section, JmodSection::Legal);
        Ok(())
    }

    #[test]
    fn rejects_plain_zip_input() -> Result<()> {
        let bytes = JarFile::new().to_bytes()?;
        assert!(matches!(
            JmodFile::from_bytes(bytes),
            Err(Error::InvalidJmod { offset: 0, .. })
        ));
        Ok(())
    }
}

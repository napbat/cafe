//! Class-entry discovery and single-pass parsing.

use crate::classfile::{ClassAccessFlags, ClassFile};
use crate::{Error, Result};

use super::entry::JarEntry;
use super::reader::EntryReader;
use super::{EntryId, EntryKind, JarFile, is_class_entry, normalize_class_entry};

/// Borrowed metadata for one physical class entry in a JAR.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClassEntry<'a> {
    /// Stable identity of the physical archive entry.
    pub id: EntryId,
    /// Exact current entry name using the JAR's forward-slash separator.
    pub name: &'a str,
    /// Uncompressed class-file length.
    pub size: u64,
}

/// Control returned by a class visitor after processing one parsed class.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClassVisitControl {
    /// Continue with the next selected class entry.
    Continue,
    /// Stop successfully without reading any remaining entries.
    Stop,
}

/// Metadata obtained by parsing one class entry during archive enumeration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClassSummary {
    /// Stable identity of the class entry.
    pub entry_id: EntryId,
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
    /// Resolves a dotted, internal, or `.class` name and parses that class.
    ///
    /// Use [`Self::visit_classes`] when reading multiple classes so the ZIP
    /// directory is parsed only once.
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

    /// Iterates borrowed metadata for all physical class entries in archive
    /// order without reading their payloads.
    pub fn class_entries(&self) -> impl Iterator<Item = ClassEntry<'_>> + '_ {
        self.entries
            .iter()
            .filter(|entry| is_physical_class(entry))
            .map(class_entry)
    }

    /// Returns all `.class` entry names in archive order.
    #[must_use]
    pub fn class_entry_names(&self) -> Vec<String> {
        self.class_entries()
            .map(|entry| entry.name.to_owned())
            .collect()
    }

    /// Returns the number of `.class` entries without reading their payloads.
    #[must_use]
    pub fn class_entry_count(&self) -> usize {
        self.class_entries().count()
    }

    /// Visits selected classes in archive order using one ZIP reader.
    ///
    /// `select` receives entry identity, name, and uncompressed size before the
    /// payload is decompressed or parsed. Returning `false` skips that entry.
    /// `visit` receives the same entry metadata and the owned parsed class. It
    /// may return [`ClassVisitControl::Stop`] to finish successfully without
    /// reading later entries.
    ///
    /// The callback error is generic so a consumer can use its own error type;
    /// it must be constructible from this crate's [`Error`]. ZIP and class-file
    /// failures are qualified with the exact physical entry name before they
    /// are converted.
    ///
    /// # Errors
    ///
    /// Returns the first selected entry's decompression or class-file error, or
    /// an error returned by `visit`.
    pub fn visit_classes<S, V, E>(&self, select: S, visit: V) -> std::result::Result<(), E>
    where
        for<'entry> S: FnMut(ClassEntry<'entry>) -> bool,
        for<'entry> V:
            FnMut(ClassEntry<'entry>, ClassFile) -> std::result::Result<ClassVisitControl, E>,
        E: From<Error>,
    {
        let mut reader = EntryReader::new(self);
        self.visit_classes_with_reader(&mut reader, select, visit)
    }

    /// Parses and returns metadata for every class entry in archive order.
    ///
    /// # Errors
    ///
    /// Returns an error identifying the first unreadable or invalid class
    /// entry.
    pub fn class_summaries(&self) -> Result<Vec<ClassSummary>> {
        let mut summaries = Vec::new();
        self.visit_classes(
            |_| true,
            |entry, class| -> Result<ClassVisitControl> {
                let internal_name = class
                    .class_name()
                    .map_err(|error| error.in_jar_entry(entry.name))?
                    .to_owned();
                summaries.push(ClassSummary {
                    entry_id: entry.id,
                    entry_name: entry.name.to_owned(),
                    internal_name,
                    minor_version: class.minor_version,
                    major_version: class.major_version,
                    access_flags: class.access_flags,
                    fields: class.fields.len(),
                    methods: class.methods.len(),
                    size: entry.size,
                });
                Ok(ClassVisitControl::Continue)
            },
        )?;
        Ok(summaries)
    }

    fn visit_classes_with_reader<S, V, E>(
        &self,
        reader: &mut EntryReader,
        mut select: S,
        mut visit: V,
    ) -> std::result::Result<(), E>
    where
        for<'entry> S: FnMut(ClassEntry<'entry>) -> bool,
        for<'entry> V:
            FnMut(ClassEntry<'entry>, ClassFile) -> std::result::Result<ClassVisitControl, E>,
        E: From<Error>,
    {
        for entry in &self.entries {
            if !is_physical_class(entry) {
                continue;
            }
            let info = class_entry(entry);
            if !select(info) {
                continue;
            }
            let bytes = reader
                .read(entry)
                .map_err(|error| E::from(error.in_jar_entry(entry.name.clone())))?;
            let class = ClassFile::parse(&bytes)
                .map_err(|error| E::from(error.in_jar_entry(entry.name.clone())))?;
            if visit(info, class)? == ClassVisitControl::Stop {
                break;
            }
        }
        Ok(())
    }
}

fn is_physical_class(entry: &JarEntry) -> bool {
    entry.kind == EntryKind::File && is_class_entry(&entry.name)
}

fn class_entry(entry: &JarEntry) -> ClassEntry<'_> {
    ClassEntry {
        id: entry.id,
        name: &entry.name,
        size: entry.uncompressed_size(),
    }
}

#[cfg(test)]
mod tests {
    use crate::classfile::{ClassAccessFlags, ClassFile, JAVA_8_MAJOR_VERSION};
    use crate::{Error, Result};

    use super::super::reader::EntryReader;
    use super::{ClassVisitControl, JarFile};

    const BULK_CLASS_COUNT: usize = 128;

    #[test]
    fn bulk_visitation_constructs_one_archive_reader() -> Result<()> {
        let mut source = JarFile::new();
        for index in 0..BULK_CLASS_COUNT {
            source.add_class(&new_class(&format!("sample/Type{index}"))?)?;
        }
        let jar = JarFile::from_bytes(source.to_bytes()?)?;
        let mut reader = EntryReader::new(&jar);
        let mut visited = 0;

        jar.visit_classes_with_reader(
            &mut reader,
            |_| true,
            |entry, class| -> Result<ClassVisitControl> {
                assert_eq!(entry.name, format!("{}.class", class.class_name()?));
                visited += 1;
                Ok(ClassVisitControl::Continue)
            },
        )?;

        assert_eq!(visited, BULK_CLASS_COUNT);
        assert_eq!(reader.archive_constructions(), 1);
        Ok(())
    }

    #[test]
    fn selection_skips_parsing_and_visitors_stop_before_later_entries() -> Result<()> {
        let mut jar = JarFile::new();
        jar.add_class(&new_class("sample/First")?)?;
        jar.add_file("sample/Skipped.class", b"not a class".to_vec())?;
        jar.add_class(&new_class("sample/Last")?)?;
        jar.add_file("sample/Unread.class", b"also not a class".to_vec())?;
        let mut visited = Vec::new();

        jar.visit_classes(
            |entry| entry.name != "sample/Skipped.class",
            |entry, _| -> Result<ClassVisitControl> {
                visited.push(entry.name.to_owned());
                Ok(if entry.name == "sample/Last.class" {
                    ClassVisitControl::Stop
                } else {
                    ClassVisitControl::Continue
                })
            },
        )?;

        assert_eq!(visited, ["sample/First.class", "sample/Last.class"]);
        Ok(())
    }

    #[test]
    fn selected_parse_errors_retain_the_physical_entry_name() -> Result<()> {
        let mut jar = JarFile::new();
        jar.add_file("sample/Broken.class", b"not a class".to_vec())?;

        let result: Result<()> = jar.visit_classes(
            |_| true,
            |_, _| -> Result<ClassVisitControl> { Ok(ClassVisitControl::Continue) },
        );

        assert!(matches!(
            result,
            Err(Error::JarEntry { entry, .. }) if entry == "sample/Broken.class"
        ));
        Ok(())
    }

    fn new_class(name: &str) -> Result<ClassFile> {
        ClassFile::new(
            JAVA_8_MAJOR_VERSION,
            name,
            Some("java/lang/Object"),
            ClassAccessFlags::PUBLIC,
        )
    }
}

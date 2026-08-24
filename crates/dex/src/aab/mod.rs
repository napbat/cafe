//! Android App Bundle discovery and module-qualified DEX provenance.
//!
//! An AAB is treated solely as a ZIP discovery boundary. DEX payloads use the
//! existing canonical frontend and retain their module, multidex ordinal, and
//! exact archive entry name.

mod discovery;

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::Cursor;
use std::path::Path;
use std::sync::Arc;

use disassembler::Disassembly;
use program::Module;
use zip::ZipArchive;

use crate::apk::{DexOrdinal, parse_dex_entry_name};
use crate::disassembly::lower_file_named;
use crate::program::{ProgramOptions, lower_file_named_with_options};
use crate::{DexFile, Error, Result};

pub use self::discovery::{AAB_EXTENSION, Traversal, discover_aabs, is_aab_path};

const DEX_DIRECTORY: &str = "dex";

/// Stable physical identity of one entry in an open App Bundle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct AabEntryId(u64);

impl AabEntryId {
    /// Returns the numeric entry identity.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// Deterministic provenance for one DEX resource in an App Bundle module.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct AabDexEntry {
    /// Bundle module directory, such as `base` or a feature-module name.
    pub module: String,
    /// One-based multidex ordinal within the module.
    pub ordinal: DexOrdinal,
    /// Stable physical ZIP entry identity.
    pub entry_id: AabEntryId,
    /// Exact archive-relative path.
    pub entry_name: String,
    archive_index: usize,
}

/// Parsed DEX file paired with exact App Bundle provenance.
#[derive(Debug, Clone)]
pub struct AabDexArtifact {
    /// Module and physical-entry origin.
    pub origin: AabDexEntry,
    /// Parsed canonical DEX file.
    pub file: DexFile,
}

impl AabDexArtifact {
    /// Lowers this artifact using its exact bundle entry path.
    ///
    /// # Errors
    ///
    /// Returns an entry-scoped DEX or disassembly error.
    pub fn disassemble(&self) -> Result<Disassembly> {
        lower_file_named(&self.file, &self.origin.entry_name)
            .map_err(|error| error.in_aab_entry(self.origin.entry_name.clone()))
    }

    /// Builds a Program module using its exact bundle entry path.
    ///
    /// # Errors
    ///
    /// Returns an entry-scoped definition or body-loading error.
    pub fn to_module(&self, options: ProgramOptions) -> Result<Module> {
        lower_file_named_with_options(&self.file, &self.origin.entry_name, options)
            .map_err(|error| error.in_aab_entry(self.origin.entry_name.clone()))
    }
}

/// Control returned by an App Bundle DEX visitor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AabDexVisitControl {
    /// Continue in module/ordinal order.
    Continue,
    /// Stop successfully.
    Stop,
}

#[derive(Debug, Clone)]
struct AabEntry {
    id: AabEntryId,
    name: String,
    archive_index: usize,
    is_file: bool,
}

/// Read-only Android App Bundle retaining exact original bytes.
#[derive(Debug, Clone)]
pub struct AabFile {
    bytes: Arc<[u8]>,
    entries: Vec<AabEntry>,
}

impl AabFile {
    /// Opens an App Bundle from disk.
    ///
    /// # Errors
    ///
    /// Returns an error for unreadable or malformed ZIP data.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        Self::from_bytes(fs::read(path)?)
    }

    /// Parses an App Bundle ZIP directory while retaining lazy payloads.
    ///
    /// # Errors
    ///
    /// Returns an error for malformed ZIP metadata or an entry count beyond
    /// stable identity limits.
    pub fn from_bytes(bytes: impl Into<Vec<u8>>) -> Result<Self> {
        let bytes: Arc<[u8]> = bytes.into().into();
        let mut archive = ZipArchive::new(Cursor::new(Arc::clone(&bytes)))?;
        let mut entries = Vec::with_capacity(archive.len());
        for index in 0..archive.len() {
            let file = archive.by_index(index)?;
            entries.push(AabEntry {
                id: AabEntryId(u64::try_from(index).map_err(|_| {
                    Error::invalid_aab("entry position does not fit stable identity")
                })?),
                name: file.name().to_owned(),
                archive_index: index,
                is_file: file.is_file(),
            });
        }
        Ok(Self { bytes, entries })
    }

    /// Returns exact original bundle bytes.
    #[must_use]
    pub fn original_bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Returns physical member names in archive order.
    #[must_use]
    pub fn entry_names(&self) -> Vec<&str> {
        self.entries
            .iter()
            .map(|entry| entry.name.as_str())
            .collect()
    }

    /// Discovers module-qualified DEX entries in deterministic module and
    /// numeric multidex order.
    ///
    /// # Errors
    ///
    /// Returns an error for duplicates or a missing/gapped ordinal within any
    /// module that contains DEX entries.
    pub fn dex_entries(&self) -> Result<Vec<AabDexEntry>> {
        let mut seen_names = BTreeSet::new();
        let mut entries = Vec::new();
        for entry in &self.entries {
            if !seen_names.insert(entry.name.as_str()) {
                return Err(Error::invalid_aab(format!(
                    "duplicate ZIP entry name `{}`",
                    entry.name
                )));
            }
            if !entry.is_file {
                continue;
            }
            let Some((module, ordinal)) = parse_aab_dex_entry_name(&entry.name) else {
                continue;
            };
            entries.push(AabDexEntry {
                module: module.to_owned(),
                ordinal,
                entry_id: entry.id,
                entry_name: entry.name.clone(),
                archive_index: entry.archive_index,
            });
        }
        entries.sort_by(|left, right| {
            left.module
                .cmp(&right.module)
                .then(left.ordinal.cmp(&right.ordinal))
                .then(left.entry_name.cmp(&right.entry_name))
        });
        validate_layout(&entries)?;
        Ok(entries)
    }

    /// Reads one module DEX by numeric ordinal.
    ///
    /// # Errors
    ///
    /// Returns an error if the layout is invalid, the requested origin is
    /// absent, or its payload is unreadable or malformed.
    pub fn read_dex(&self, module: &str, ordinal: DexOrdinal) -> Result<AabDexArtifact> {
        let origin = self
            .dex_entries()?
            .into_iter()
            .find(|entry| entry.module == module && entry.ordinal == ordinal)
            .ok_or_else(|| {
                Error::invalid_aab(format!(
                    "module `{module}` has no DEX ordinal {}",
                    ordinal.get()
                ))
            })?;
        let mut reader = AabReader::new(self);
        reader.read(origin)
    }

    /// Visits selected DEX artifacts using one ZIP reader.
    ///
    /// # Errors
    ///
    /// Returns a layout error, the first selected entry-scoped read or parse
    /// error, or a visitor error.
    pub fn visit_dex<S, V, E>(&self, mut select: S, mut visit: V) -> std::result::Result<(), E>
    where
        S: FnMut(&AabDexEntry) -> bool,
        V: FnMut(AabDexArtifact) -> std::result::Result<AabDexVisitControl, E>,
        E: From<Error>,
    {
        let entries = self.dex_entries().map_err(E::from)?;
        let mut reader = AabReader::new(self);
        for entry in entries {
            if !select(&entry) {
                continue;
            }
            let artifact = reader.read(entry).map_err(E::from)?;
            if visit(artifact)? == AabDexVisitControl::Stop {
                break;
            }
        }
        Ok(())
    }

    /// Visits selected raw DEX payloads in module/ordinal order with one ZIP reader.
    ///
    /// Entry read failures are passed to `visit`, allowing deterministic corpus
    /// consumers to continue with later independent members.
    ///
    /// # Errors
    ///
    /// Returns a bundle-layout error or an error returned by `visit`.
    pub fn visit_dex_bytes<S, V, E>(
        &self,
        mut select: S,
        mut visit: V,
    ) -> std::result::Result<(), E>
    where
        S: FnMut(&AabDexEntry) -> bool,
        V: FnMut(AabDexEntry, Result<Vec<u8>>) -> std::result::Result<AabDexVisitControl, E>,
        E: From<Error>,
    {
        let entries = self.dex_entries().map_err(E::from)?;
        let mut reader = AabReader::new(self);
        for entry in entries {
            if !select(&entry) {
                continue;
            }
            let bytes = reader.read_bytes(&entry);
            if visit(entry, bytes)? == AabDexVisitControl::Stop {
                break;
            }
        }
        Ok(())
    }

    /// Parses all DEX artifacts in deterministic provenance order.
    ///
    /// # Errors
    ///
    /// Returns the first layout, decompression, or DEX error.
    pub fn read_all_dex(&self) -> Result<Vec<AabDexArtifact>> {
        let mut output = Vec::new();
        self.visit_dex(
            |_| true,
            |artifact| -> Result<AabDexVisitControl> {
                output.push(artifact);
                Ok(AabDexVisitControl::Continue)
            },
        )?;
        Ok(output)
    }
}

struct AabReader<'a> {
    archive: Option<ZipArchive<Cursor<Arc<[u8]>>>>,
    file: &'a AabFile,
}

impl<'a> AabReader<'a> {
    const fn new(file: &'a AabFile) -> Self {
        Self {
            archive: None,
            file,
        }
    }

    fn read(&mut self, origin: AabDexEntry) -> Result<AabDexArtifact> {
        let bytes = self.read_bytes(&origin)?;
        let file = DexFile::parse(&bytes)
            .map_err(|error| error.in_aab_entry(origin.entry_name.clone()))?;
        Ok(AabDexArtifact { origin, file })
    }

    fn read_bytes(&mut self, origin: &AabDexEntry) -> Result<Vec<u8>> {
        if self.archive.is_none() {
            self.archive = Some(ZipArchive::new(Cursor::new(Arc::clone(&self.file.bytes)))?);
        }
        let archive = self.archive.as_mut().expect("archive initialized above");
        let mut member = archive
            .by_index(origin.archive_index)
            .map_err(|error| Error::from(error).in_aab_entry(origin.entry_name.clone()))?;
        let mut bytes = Vec::new();
        std::io::Read::read_to_end(&mut member, &mut bytes)
            .map_err(|error| Error::from(error).in_aab_entry(origin.entry_name.clone()))?;
        Ok(bytes)
    }
}

/// Parses `<module>/dex/classes[ordinal].dex` provenance.
#[must_use]
pub fn parse_aab_dex_entry_name(name: &str) -> Option<(&str, DexOrdinal)> {
    let mut components = name.split('/');
    let module = components.next()?;
    let directory = components.next()?;
    let file = components.next()?;
    if components.next().is_some()
        || module.is_empty()
        || directory != DEX_DIRECTORY
        || !module
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
    {
        return None;
    }
    parse_dex_entry_name(file).map(|ordinal| (module, ordinal))
}

fn validate_layout(entries: &[AabDexEntry]) -> Result<()> {
    let mut by_module = BTreeMap::<&str, Vec<&AabDexEntry>>::new();
    for entry in entries {
        by_module.entry(&entry.module).or_default().push(entry);
    }
    for (module, entries) in by_module {
        for (position, entry) in entries.into_iter().enumerate() {
            let expected = u32::try_from(position)
                .ok()
                .and_then(|value| value.checked_add(DexOrdinal::PRIMARY.get()))
                .and_then(DexOrdinal::new)
                .ok_or_else(|| Error::invalid_aab("multidex ordinal exceeds 32 bits"))?;
            if entry.ordinal != expected {
                return Err(Error::invalid_aab(format!(
                    "module `{module}` expected DEX ordinal {} but found `{}`",
                    expected.get(),
                    entry.entry_name
                )));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::apk::ApkFile;
    use crate::{DexVersion, Result};

    use super::*;

    #[test]
    fn discovers_module_qualified_multidex_in_stable_order() -> Result<()> {
        let dex = DexFile::new(DexVersion::V040).to_bytes()?;
        let mut archive = ApkFile::new();
        archive.add_file("feature/dex/classes.dex", dex.clone())?;
        archive.add_file("base/dex/classes2.dex", dex.clone())?;
        archive.add_file("base/dex/classes.dex", dex)?;
        archive.add_file("base/manifest/AndroidManifest.xml", vec![1])?;

        let bundle = AabFile::from_bytes(archive.to_bytes()?)?;
        let entries = bundle.dex_entries()?;
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].module, "base");
        assert_eq!(entries[0].ordinal, DexOrdinal::PRIMARY);
        assert_eq!(entries[1].ordinal.get(), 2);
        assert_eq!(entries[2].module, "feature");
        assert_eq!(bundle.read_all_dex()?.len(), 3);
        Ok(())
    }

    #[test]
    fn rejects_gapped_module_layouts() -> Result<()> {
        let dex = DexFile::new(DexVersion::V040).to_bytes()?;
        let mut archive = ApkFile::new();
        archive.add_file("base/dex/classes2.dex", dex)?;
        let bundle = AabFile::from_bytes(archive.to_bytes()?)?;
        assert!(bundle.dex_entries().is_err());
        Ok(())
    }

    #[test]
    fn recognizes_only_canonical_bundle_paths() {
        assert_eq!(
            parse_aab_dex_entry_name("base/dex/classes.dex"),
            Some(("base", DexOrdinal::PRIMARY))
        );
        assert!(parse_aab_dex_entry_name("base/classes.dex").is_none());
        assert!(parse_aab_dex_entry_name("base/dex/classes01.dex").is_none());
        assert!(parse_aab_dex_entry_name("bad/module/dex/classes.dex").is_none());
    }
}

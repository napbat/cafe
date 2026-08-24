//! Deterministic multidex naming, discovery, parsing, and provenance.

use std::num::NonZeroU32;

use ::cafe::Module;
use disassembler::Disassembly;

use super::{ApkFile, EntryId, EntryKind};
use crate::cafe::{CafeOptions, lower_file_named_with_options};
use crate::disassembly::lower_file_named;
use crate::{DexFile, Error, Result};

const DEX_ENTRY_STEM: &str = "classes";
const DEX_ENTRY_SUFFIX: &str = ".dex";
const PRIMARY_DEX_ENTRY: &str = "classes.dex";
const FIRST_SECONDARY_ORDINAL: u32 = 2;

/// One-based ordinal encoded by an APK multidex entry name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct DexOrdinal(NonZeroU32);

impl DexOrdinal {
    /// Primary `classes.dex` ordinal.
    pub const PRIMARY: Self = Self(NonZeroU32::MIN);

    /// Creates a nonzero DEX ordinal.
    #[must_use]
    pub const fn new(value: u32) -> Option<Self> {
        match NonZeroU32::new(value) {
            Some(value) => Some(Self(value)),
            None => None,
        }
    }

    /// Returns the one-based numeric ordinal.
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0.get()
    }

    /// Returns whether this is the primary `classes.dex` artifact.
    #[must_use]
    pub const fn is_primary(self) -> bool {
        self.0.get() == Self::PRIMARY.0.get()
    }
}

/// Deterministic provenance for one DEX member in an APK.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DexEntry {
    /// One-based multidex ordinal.
    pub ordinal: DexOrdinal,
    /// Stable archive entry identity.
    pub entry_id: EntryId,
    /// Exact archive-relative entry name.
    pub entry_name: String,
}

/// Parsed DEX file paired with its exact APK origin.
#[derive(Debug, Clone)]
pub struct DexArtifact {
    /// Stable APK entry provenance.
    pub origin: DexEntry,
    /// Parsed logical DEX file.
    pub file: DexFile,
}

impl DexArtifact {
    /// Lowers this DEX artifact using its exact APK entry name.
    ///
    /// # Errors
    ///
    /// Returns an error when DEX definitions or executable bodies are invalid.
    pub fn disassemble(&self) -> Result<Disassembly> {
        lower_file_named(&self.file, &self.origin.entry_name)
            .map_err(|error| error.in_apk_entry(self.origin.entry_name.clone()))
    }

    /// Builds a Cafe module using this artifact's exact APK entry name.
    ///
    /// # Errors
    ///
    /// Returns an error when DEX definitions or requested bodies are invalid.
    pub fn to_module(&self, options: CafeOptions) -> Result<Module> {
        lower_file_named_with_options(&self.file, &self.origin.entry_name, options)
            .map_err(|error| error.in_apk_entry(self.origin.entry_name.clone()))
    }
}

/// Produces the canonical root entry name for a DEX ordinal.
#[must_use]
pub fn dex_entry_name(ordinal: DexOrdinal) -> String {
    if ordinal.is_primary() {
        PRIMARY_DEX_ENTRY.to_owned()
    } else {
        format!("{DEX_ENTRY_STEM}{}{DEX_ENTRY_SUFFIX}", ordinal.get())
    }
}

/// Parses a canonical root multidex name.
///
/// Names with leading zeroes, zero/one suffixes, nested paths, or trailing
/// characters are not classified as DEX artifacts.
#[must_use]
pub fn parse_dex_entry_name(name: &str) -> Option<DexOrdinal> {
    if name == PRIMARY_DEX_ENTRY {
        return Some(DexOrdinal::PRIMARY);
    }
    let digits = name
        .strip_prefix(DEX_ENTRY_STEM)?
        .strip_suffix(DEX_ENTRY_SUFFIX)?;
    let value = digits.parse::<u32>().ok()?;
    if value < FIRST_SECONDARY_ORDINAL || value.to_string() != digits {
        return None;
    }
    DexOrdinal::new(value)
}

impl ApkFile {
    /// Discovers DEX entries in numeric multidex order.
    ///
    /// # Errors
    ///
    /// Returns an error if duplicate ZIP names claim the same ordinal.
    pub fn dex_entries(&self) -> Result<Vec<DexEntry>> {
        let mut entries = self
            .entries
            .iter()
            .filter(|entry| entry.kind == EntryKind::File)
            .filter_map(|entry| {
                parse_dex_entry_name(&entry.name).map(|ordinal| DexEntry {
                    ordinal,
                    entry_id: entry.id,
                    entry_name: entry.name.clone(),
                })
            })
            .collect::<Vec<_>>();
        entries.sort_by_key(|entry| entry.ordinal);
        if let Some(pair) = entries
            .windows(2)
            .find(|pair| pair[0].ordinal == pair[1].ordinal)
        {
            return Err(Error::AmbiguousApkEntry {
                name: pair[0].entry_name.clone(),
                count: self.entry_ids_named(&pair[0].entry_name).len(),
            });
        }
        Ok(entries)
    }

    /// Validates that discovered DEX ordinals start at one and are contiguous.
    ///
    /// Empty resource-only APKs are accepted.
    ///
    /// # Errors
    ///
    /// Returns an error for duplicate, missing-primary, or gapped ordinals.
    pub fn validate_dex_layout(&self) -> Result<()> {
        let entries = self.dex_entries()?;
        for (position, entry) in entries.iter().enumerate() {
            let expected = u32::try_from(position)
                .ok()
                .and_then(|value| value.checked_add(DexOrdinal::PRIMARY.get()))
                .and_then(DexOrdinal::new)
                .ok_or_else(|| Error::invalid_apk("multidex ordinal exceeds 32 bits"))?;
            if entry.ordinal != expected {
                return Err(Error::invalid_apk(format!(
                    "multidex layout expected `{}` but found `{}`",
                    dex_entry_name(expected),
                    entry.entry_name
                )));
            }
        }
        Ok(())
    }

    /// Parses one DEX artifact by multidex ordinal.
    ///
    /// # Errors
    ///
    /// Returns an error when the entry is absent, ambiguous, unreadable, or invalid.
    pub fn read_dex(&self, ordinal: DexOrdinal) -> Result<DexArtifact> {
        let name = dex_entry_name(ordinal);
        let entry_id = self.unique_entry_id(&name)?;
        self.read_dex_entry(DexEntry {
            ordinal,
            entry_id,
            entry_name: name,
        })
    }

    /// Parses every DEX artifact in numeric multidex order.
    ///
    /// # Errors
    ///
    /// Returns an entry-scoped error for the first unreadable or invalid file.
    pub fn read_all_dex(&self) -> Result<Vec<DexArtifact>> {
        self.validate_dex_layout()?;
        self.dex_entries()?
            .into_iter()
            .map(|entry| self.read_dex_entry(entry))
            .collect()
    }

    /// Adds or replaces a canonical DEX entry from a structured file.
    ///
    /// # Errors
    ///
    /// Returns an error if the DEX cannot be assembled or the entry is ambiguous.
    pub fn put_dex(&mut self, ordinal: DexOrdinal, file: &DexFile) -> Result<EntryId> {
        let name = dex_entry_name(ordinal);
        let bytes = file
            .to_bytes()
            .map_err(|error| error.in_apk_entry(name.clone()))?;
        self.put_file(name, bytes)
    }

    /// Removes one canonical DEX entry and returns its uncompressed bytes.
    ///
    /// # Errors
    ///
    /// Returns an error if the entry is absent, ambiguous, or unreadable.
    pub fn remove_dex(&mut self, ordinal: DexOrdinal) -> Result<Vec<u8>> {
        self.remove_entry(&dex_entry_name(ordinal))
    }

    fn read_dex_entry(&self, origin: DexEntry) -> Result<DexArtifact> {
        let bytes = self.read_entry_by_id(origin.entry_id)?;
        let file = DexFile::parse(&bytes)
            .map_err(|error| error.in_apk_entry(origin.entry_name.clone()))?;
        Ok(DexArtifact { origin, file })
    }
}

#[cfg(test)]
mod tests {
    use super::{DexOrdinal, dex_entry_name, parse_dex_entry_name};

    #[test]
    fn parses_only_canonical_multidex_names() {
        let secondary = DexOrdinal::new(2).unwrap();
        assert_eq!(
            parse_dex_entry_name("classes.dex"),
            Some(DexOrdinal::PRIMARY)
        );
        assert_eq!(parse_dex_entry_name("classes2.dex"), Some(secondary));
        assert_eq!(dex_entry_name(secondary), "classes2.dex");
        assert_eq!(parse_dex_entry_name("classes02.dex"), None);
        assert_eq!(parse_dex_entry_name("nested/classes.dex"), None);
        assert_eq!(parse_dex_entry_name("classes1.dex"), None);
    }
}

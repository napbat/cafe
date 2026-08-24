//! Deterministic multidex naming, discovery, parsing, and provenance.

use std::num::NonZeroU32;

use disassembler::Disassembly;
use program::Module;

use super::reader::EntryReader;
use super::{ApkFile, EntryId, EntryKind};
use crate::disassembly::lower_file_named;
use crate::program::{ProgramOptions, lower_file_named_with_options};
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

/// Control returned after visiting one parsed DEX artifact.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DexVisitControl {
    /// Continue with the next selected multidex entry.
    Continue,
    /// Stop successfully without reading later entries.
    Stop,
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

    /// Builds a program module using this artifact's exact APK entry name.
    ///
    /// # Errors
    ///
    /// Returns an error when DEX definitions or requested bodies are invalid.
    pub fn to_module(&self, options: ProgramOptions) -> Result<Module> {
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
        self.validated_dex_entries().map(drop)
    }

    /// Parses one DEX artifact by multidex ordinal.
    ///
    /// # Errors
    ///
    /// Returns an error when the entry is absent, ambiguous, unreadable, or invalid.
    pub fn read_dex(&self, ordinal: DexOrdinal) -> Result<DexArtifact> {
        let name = dex_entry_name(ordinal);
        let entry_id = self.unique_entry_id(&name)?;
        let mut reader = EntryReader::new(self);
        self.read_dex_entry(
            &mut reader,
            DexEntry {
                ordinal,
                entry_id,
                entry_name: name,
            },
        )
    }

    /// Visits selected DEX artifacts in numeric multidex order using one ZIP reader.
    ///
    /// The APK must have a contiguous canonical multidex layout. `select`
    /// receives artifact provenance before its payload is decompressed or
    /// parsed; returning `false` skips that entry. `visit` receives the owned
    /// parsed artifact and may return [`DexVisitControl::Stop`] to finish
    /// successfully without reading later entries.
    ///
    /// The callback error is generic so consumers can retain their own error
    /// type. It must be constructible from this crate's [`Error`]. Archive and
    /// parser failures retain the exact APK entry name before conversion.
    ///
    /// # Errors
    ///
    /// Returns a multidex-layout error, the first selected entry's scoped read
    /// or parse error, or an error returned by `visit`.
    pub fn visit_dex<S, V, E>(&self, select: S, visit: V) -> std::result::Result<(), E>
    where
        S: FnMut(&DexEntry) -> bool,
        V: FnMut(DexArtifact) -> std::result::Result<DexVisitControl, E>,
        E: From<Error>,
    {
        let mut reader = EntryReader::new(self);
        self.visit_dex_with_reader(&mut reader, select, visit)
    }

    /// Parses every DEX artifact in numeric multidex order.
    ///
    /// # Errors
    ///
    /// Returns an entry-scoped error for the first unreadable or invalid file.
    pub fn read_all_dex(&self) -> Result<Vec<DexArtifact>> {
        let mut artifacts = Vec::new();
        self.visit_dex(
            |_| true,
            |artifact| -> Result<DexVisitControl> {
                artifacts.push(artifact);
                Ok(DexVisitControl::Continue)
            },
        )?;
        Ok(artifacts)
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

    fn validated_dex_entries(&self) -> Result<Vec<DexEntry>> {
        let entries = self.dex_entries()?;
        validate_dex_entries(&entries)?;
        Ok(entries)
    }

    fn visit_dex_with_reader<S, V, E>(
        &self,
        reader: &mut EntryReader,
        mut select: S,
        mut visit: V,
    ) -> std::result::Result<(), E>
    where
        S: FnMut(&DexEntry) -> bool,
        V: FnMut(DexArtifact) -> std::result::Result<DexVisitControl, E>,
        E: From<Error>,
    {
        for origin in self.validated_dex_entries().map_err(E::from)? {
            if !select(&origin) {
                continue;
            }
            let artifact = self.read_dex_entry(reader, origin).map_err(E::from)?;
            if visit(artifact)? == DexVisitControl::Stop {
                break;
            }
        }
        Ok(())
    }

    fn read_dex_entry(&self, reader: &mut EntryReader, origin: DexEntry) -> Result<DexArtifact> {
        let entry = self
            .entry_record(origin.entry_id)
            .map_err(|error| error.in_apk_entry(origin.entry_name.clone()))?;
        let bytes = reader
            .read(entry)
            .map_err(|error| error.in_apk_entry(origin.entry_name.clone()))?;
        let file = DexFile::parse(&bytes)
            .map_err(|error| error.in_apk_entry(origin.entry_name.clone()))?;
        Ok(DexArtifact { origin, file })
    }
}

fn validate_dex_entries(entries: &[DexEntry]) -> Result<()> {
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

#[cfg(test)]
mod tests {
    use super::super::reader::EntryReader;
    use super::{
        ApkFile, DexFile, DexOrdinal, DexVisitControl, dex_entry_name, parse_dex_entry_name,
    };
    use crate::{DexVersion, Error, Result};

    const BULK_DEX_COUNT: u32 = 128;

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

    #[test]
    fn bulk_visitation_constructs_one_archive_reader() -> Result<()> {
        let mut source = ApkFile::new();
        let file = DexFile::new(DexVersion::V040);
        for value in DexOrdinal::PRIMARY.get()..=BULK_DEX_COUNT {
            source.put_dex(
                DexOrdinal::new(value).expect("test ordinal is nonzero"),
                &file,
            )?;
        }
        let apk = ApkFile::from_bytes(source.to_bytes()?)?;
        let mut reader = EntryReader::new(&apk);
        let mut visited = 0_u32;

        apk.visit_dex_with_reader(
            &mut reader,
            |_| true,
            |artifact| -> Result<DexVisitControl> {
                assert_eq!(artifact.file.version(), DexVersion::V040);
                visited += 1;
                Ok(DexVisitControl::Continue)
            },
        )?;

        assert_eq!(visited, BULK_DEX_COUNT);
        assert_eq!(reader.archive_constructions(), 1);
        Ok(())
    }

    #[test]
    fn selection_skips_parsing_and_visitors_stop_before_later_entries() -> Result<()> {
        let mut apk = ApkFile::new();
        let file = DexFile::new(DexVersion::V040);
        let second = DexOrdinal::new(2).expect("test ordinal is nonzero");
        let third = DexOrdinal::new(3).expect("test ordinal is nonzero");
        let fourth = DexOrdinal::new(4).expect("test ordinal is nonzero");
        apk.put_dex(DexOrdinal::PRIMARY, &file)?;
        apk.add_file(dex_entry_name(second), b"not a DEX file".to_vec())?;
        apk.put_dex(third, &file)?;
        apk.add_file(dex_entry_name(fourth), b"also not a DEX file".to_vec())?;
        let mut visited = Vec::new();

        apk.visit_dex(
            |entry| entry.ordinal != second,
            |artifact| -> Result<DexVisitControl> {
                visited.push(artifact.origin.ordinal);
                Ok(if artifact.origin.ordinal == third {
                    DexVisitControl::Stop
                } else {
                    DexVisitControl::Continue
                })
            },
        )?;

        assert_eq!(visited, [DexOrdinal::PRIMARY, third]);
        Ok(())
    }

    #[test]
    fn selected_parse_errors_retain_the_apk_entry_name() -> Result<()> {
        let mut apk = ApkFile::new();
        apk.add_file(
            dex_entry_name(DexOrdinal::PRIMARY),
            b"not a DEX file".to_vec(),
        )?;

        let result: Result<()> = apk.visit_dex(
            |_| true,
            |_| -> Result<DexVisitControl> { Ok(DexVisitControl::Continue) },
        );

        assert!(matches!(
            result,
            Err(Error::ApkEntry { entry, .. }) if entry == "classes.dex"
        ));
        Ok(())
    }
}

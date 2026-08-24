//! Multi-release JAR entry construction and effective-view resolution.

use std::collections::HashMap;

use crate::{Error, Result};

use super::entry::validate_entry_name;
use super::{EntryId, EntryKind, JarFile, Manifest};

const VERSIONED_PREFIX: &str = "META-INF/versions/";
const FIRST_VERSIONED_RELEASE: u16 = 9;

/// One entry selected for an effective multi-release JAR view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedEntry {
    /// Stable identity of the physical archive entry.
    pub id: EntryId,
    /// Name presented by the effective view.
    pub logical_name: String,
    /// Exact physical name stored in the JAR.
    pub physical_name: String,
    /// Java feature release of an overriding entry, or `None` for a base entry.
    pub release: Option<u16>,
}

/// Parses a physical multi-release path into its release and logical name.
///
/// Releases below Java 9 and paths without a logical entry are rejected.
#[must_use]
pub fn parse_versioned_entry(name: &str) -> Option<(u16, &str)> {
    let remainder = name.strip_prefix(VERSIONED_PREFIX)?;
    let (release, logical_name) = remainder.split_once('/')?;
    let release = release.parse().ok()?;
    (release >= FIRST_VERSIONED_RELEASE && !logical_name.is_empty())
        .then_some((release, logical_name))
}

/// Constructs a validated physical path for a multi-release entry.
///
/// # Errors
///
/// Returns an error for releases below Java 9, unsafe logical names, or names
/// inside `META-INF`, which versioned entries may not override.
pub fn versioned_entry_name(release: u16, logical_name: &str) -> Result<String> {
    if release < FIRST_VERSIONED_RELEASE {
        return Err(Error::InvalidJar(format!(
            "multi-release entries require Java {FIRST_VERSIONED_RELEASE} or newer"
        )));
    }
    validate_entry_name(logical_name, EntryKind::File)?;
    if starts_with_ascii_case(logical_name, "META-INF/") {
        return Err(Error::InvalidJar(
            "multi-release entries cannot override `META-INF`".to_owned(),
        ));
    }
    Ok(format!("{VERSIONED_PREFIX}{release}/{logical_name}"))
}

impl JarFile {
    /// Enables or disables multi-release lookup in the manifest.
    ///
    /// Enabling creates a manifest when necessary. Disabling does not remove
    /// physical versioned entries.
    ///
    /// # Errors
    ///
    /// Returns an error if the manifest is ambiguous, malformed, or cannot be
    /// assembled.
    pub fn set_multi_release(&mut self, enabled: bool) -> Result<()> {
        let Some(mut manifest) = self.manifest()? else {
            if !enabled {
                return Ok(());
            }
            let mut manifest = Manifest::new();
            manifest.main_mut().set("Multi-Release", "true")?;
            self.set_manifest(&manifest)?;
            return Ok(());
        };
        if enabled {
            manifest.main_mut().set("Multi-Release", "true")?;
        } else {
            manifest.main_mut().remove("Multi-Release");
        }
        self.set_manifest(&manifest)?;
        Ok(())
    }

    /// Adds a file in the tree for one Java feature release.
    ///
    /// # Errors
    ///
    /// Returns an error for an invalid release or logical name, or a duplicate
    /// physical entry.
    pub fn add_versioned_file(
        &mut self,
        release: u16,
        logical_name: &str,
        data: impl Into<Vec<u8>>,
    ) -> Result<EntryId> {
        self.add_file(versioned_entry_name(release, logical_name)?, data)
    }

    /// Adds or replaces a file in the tree for one Java feature release.
    ///
    /// # Errors
    ///
    /// Returns an error for an invalid release or logical name, or an
    /// ambiguous existing physical entry.
    pub fn put_versioned_file(
        &mut self,
        release: u16,
        logical_name: &str,
        data: impl Into<Vec<u8>>,
    ) -> Result<EntryId> {
        self.put_file(versioned_entry_name(release, logical_name)?, data)
    }

    /// Removes and returns one physical versioned entry.
    ///
    /// # Errors
    ///
    /// Returns an error for an invalid release or logical name, or an absent
    /// or ambiguous physical entry.
    pub fn remove_versioned_entry(&mut self, release: u16, logical_name: &str) -> Result<Vec<u8>> {
        self.remove_entry(&versioned_entry_name(release, logical_name)?)
    }

    /// Resolves one logical name for a target Java feature release.
    ///
    /// # Errors
    ///
    /// Returns an error if the manifest is malformed or a selected physical
    /// name is duplicated.
    pub fn resolve_entry(
        &self,
        logical_name: &str,
        target_release: u16,
    ) -> Result<Option<ResolvedEntry>> {
        let mut selected = self
            .unique_optional_entry(logical_name)?
            .map(|id| ResolvedEntry {
                id,
                logical_name: logical_name.to_owned(),
                physical_name: logical_name.to_owned(),
                release: None,
            });
        if target_release < FIRST_VERSIONED_RELEASE || !self.is_multi_release()? {
            return Ok(selected);
        }
        let mut best_release = 0;
        for entry in &self.entries {
            let Some((release, candidate)) = parse_versioned_entry(&entry.name) else {
                continue;
            };
            if candidate != logical_name || release > target_release || release < best_release {
                continue;
            }
            if release == best_release
                && selected.as_ref().is_some_and(|item| item.release.is_some())
            {
                return Err(Error::AmbiguousJarEntry {
                    name: entry.name.clone(),
                    count: 2,
                });
            }
            best_release = release;
            selected = Some(ResolvedEntry {
                id: entry.id,
                logical_name: logical_name.to_owned(),
                physical_name: entry.name.clone(),
                release: Some(release),
            });
        }
        Ok(selected)
    }

    /// Builds the deterministic effective archive view for a Java release.
    ///
    /// Base entries retain their order. Version-only logical names are
    /// appended in first-encounter order, and the highest eligible release
    /// replaces each logical entry.
    ///
    /// # Errors
    ///
    /// Returns an error if the manifest is malformed or duplicate physical
    /// candidates make the view ambiguous.
    pub fn effective_entries(&self, target_release: u16) -> Result<Vec<ResolvedEntry>> {
        if target_release < FIRST_VERSIONED_RELEASE || !self.is_multi_release()? {
            return Ok(self
                .entries
                .iter()
                .map(|entry| ResolvedEntry {
                    id: entry.id,
                    logical_name: entry.name.clone(),
                    physical_name: entry.name.clone(),
                    release: None,
                })
                .collect());
        }
        let mut result = Vec::new();
        let mut positions = HashMap::new();
        for entry in &self.entries {
            if parse_versioned_entry(&entry.name).is_some() {
                continue;
            }
            if positions.insert(entry.name.clone(), result.len()).is_some() {
                return Err(Error::AmbiguousJarEntry {
                    name: entry.name.clone(),
                    count: 2,
                });
            }
            result.push(ResolvedEntry {
                id: entry.id,
                logical_name: entry.name.clone(),
                physical_name: entry.name.clone(),
                release: None,
            });
        }
        for entry in &self.entries {
            let Some((release, logical_name)) = parse_versioned_entry(&entry.name) else {
                continue;
            };
            if release > target_release {
                continue;
            }
            let position = if let Some(position) = positions.get(logical_name) {
                *position
            } else {
                let position = result.len();
                positions.insert(logical_name.to_owned(), position);
                result.push(ResolvedEntry {
                    id: entry.id,
                    logical_name: logical_name.to_owned(),
                    physical_name: entry.name.clone(),
                    release: Some(release),
                });
                continue;
            };
            let previous_release = result[position].release.unwrap_or(0);
            if release > previous_release {
                result[position] = ResolvedEntry {
                    id: entry.id,
                    logical_name: logical_name.to_owned(),
                    physical_name: entry.name.clone(),
                    release: Some(release),
                };
            } else if release == previous_release && previous_release != 0 {
                return Err(Error::AmbiguousJarEntry {
                    name: entry.name.clone(),
                    count: 2,
                });
            }
        }
        Ok(result)
    }

    fn unique_optional_entry(&self, name: &str) -> Result<Option<EntryId>> {
        let matches = self.entry_ids_named(name);
        match matches.as_slice() {
            [] => Ok(None),
            [id] => Ok(Some(*id)),
            _ => Err(Error::AmbiguousJarEntry {
                name: name.to_owned(),
                count: matches.len(),
            }),
        }
    }
}

fn starts_with_ascii_case(value: &str, prefix: &str) -> bool {
    value
        .get(..prefix.len())
        .is_some_and(|candidate| candidate.eq_ignore_ascii_case(prefix))
}

#[cfg(test)]
mod tests {
    use super::{parse_versioned_entry, versioned_entry_name};

    #[test]
    fn validates_and_parses_versioned_names() {
        let name = versioned_entry_name(17, "sample/Thing.class").expect("valid path");
        assert_eq!(
            parse_versioned_entry(&name),
            Some((17, "sample/Thing.class"))
        );
        assert!(versioned_entry_name(8, "sample/Thing.class").is_err());
        assert!(versioned_entry_name(17, "META-INF/services/x").is_err());
    }
}

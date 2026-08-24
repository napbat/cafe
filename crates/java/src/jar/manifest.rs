//! Typed JAR manifest parsing, editing, and canonical assembly.

use crate::{Error, Result};

use super::layout::{NUL_CHARACTER, ZIP_U16_MAXIMUM};
use super::{
    EntryId, EntryKind, EntryMetadata, JarFile, MANIFEST_ENTRY, META_INF_DIRECTORY,
    MULTI_RELEASE_ENABLED_VALUE, MULTI_RELEASE_HEADER,
};

const MAX_PHYSICAL_LINE: usize = 72;
const MANIFEST_HEADER_SEPARATOR: &str = ": ";
const MANIFEST_LINE_ENDING: &[u8] = b"\r\n";
const CONTINUATION_MARKER: u8 = b' ';
const CONTINUATION_PREFIX_WIDTH: usize = size_of::<u8>();
const MAIN_SECTION_COUNT: usize = 1;
const FIRST_HEADER_INDEX: usize = 0;
const FOLLOWING_HEADER_INDEX: usize = FIRST_HEADER_INDEX + 1;
const FIRST_PHYSICAL_LINE_NUMBER: usize = 1;
const LINE_NUMBER_INCREMENT: usize = 1;
const BYTE_OFFSET_INCREMENT: usize = size_of::<u8>();
const INITIAL_BYTE_OFFSET: usize = 0;
const MAX_ATTRIBUTE_NAME_LENGTH: usize = MAX_PHYSICAL_LINE - MANIFEST_HEADER_SEPARATOR.len();
const MIN_ATTRIBUTE_NAME_LENGTH: usize = 1;
const MAX_ATTRIBUTE_VALUE_LENGTH: usize = ZIP_U16_MAXIMUM;
const CARRIAGE_RETURN: u8 = b'\r';
const LINE_FEED: u8 = b'\n';
const ATTRIBUTE_NAME_UNDERSCORE: u8 = b'_';
const ATTRIBUTE_NAME_HYPHEN: u8 = b'-';
const DIGEST_ATTRIBUTE_MARKER: &str = "-digest";

/// Mandatory first header of a JAR manifest.
pub const MANIFEST_VERSION_HEADER: &str = "Manifest-Version";
/// Default manifest format version emitted by [`Manifest::new`].
pub const DEFAULT_MANIFEST_VERSION: &str = "1.0";
/// Header naming a non-main manifest section.
pub const NAME_HEADER: &str = "Name";

/// One case-insensitive manifest header and its UTF-8 value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManifestAttribute {
    name: String,
    value: String,
}

impl ManifestAttribute {
    /// Creates a validated manifest attribute.
    ///
    /// # Errors
    ///
    /// Returns an error if the name or value violates the manifest grammar.
    pub fn new(name: impl Into<String>, value: impl Into<String>) -> Result<Self> {
        let name = name.into();
        let value = value.into();
        validate_attribute_name(&name)?;
        validate_attribute_value(&value)?;
        Ok(Self { name, value })
    }

    /// Returns the original spelling of the attribute name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the unfolded attribute value.
    #[must_use]
    pub fn value(&self) -> &str {
        &self.value
    }

    /// Replaces the unfolded attribute value.
    ///
    /// # Errors
    ///
    /// Returns an error if the value contains a line break or NUL.
    pub fn set_value(&mut self, value: impl Into<String>) -> Result<()> {
        let value = value.into();
        validate_attribute_value(&value)?;
        self.value = value;
        Ok(())
    }
}

/// Ordered, case-insensitive manifest attributes for one section.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ManifestSection {
    attributes: Vec<ManifestAttribute>,
}

impl ManifestSection {
    /// Creates an empty section.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            attributes: Vec::new(),
        }
    }

    /// Returns the number of attributes.
    #[must_use]
    pub fn len(&self) -> usize {
        self.attributes.len()
    }

    /// Returns whether the section has no attributes.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.attributes.is_empty()
    }

    /// Iterates over attributes in serialization order.
    #[must_use]
    pub fn iter(&self) -> impl ExactSizeIterator<Item = &ManifestAttribute> {
        self.attributes.iter()
    }

    /// Returns a value by ASCII case-insensitive attribute name.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&str> {
        self.attributes
            .iter()
            .find(|attribute| attribute.name.eq_ignore_ascii_case(name))
            .map(|attribute| attribute.value.as_str())
    }

    /// Returns whether an attribute is present.
    #[must_use]
    pub fn contains(&self, name: &str) -> bool {
        self.get(name).is_some()
    }

    /// Adds or replaces an attribute without changing an existing position.
    ///
    /// Existing attributes retain their original name spelling.
    ///
    /// # Errors
    ///
    /// Returns an error if the name or value violates the manifest grammar.
    pub fn set(
        &mut self,
        name: impl Into<String>,
        value: impl Into<String>,
    ) -> Result<Option<String>> {
        let attribute = ManifestAttribute::new(name, value)?;
        if let Some(existing) = self
            .attributes
            .iter_mut()
            .find(|existing| existing.name.eq_ignore_ascii_case(&attribute.name))
        {
            return Ok(Some(std::mem::replace(
                &mut existing.value,
                attribute.value,
            )));
        }
        self.attributes.push(attribute);
        Ok(None)
    }

    /// Removes an attribute by ASCII case-insensitive name.
    pub fn remove(&mut self, name: &str) -> Option<ManifestAttribute> {
        let index = self
            .attributes
            .iter()
            .position(|attribute| attribute.name.eq_ignore_ascii_case(name))?;
        Some(self.attributes.remove(index))
    }

    pub(crate) fn retain(&mut self, predicate: impl FnMut(&ManifestAttribute) -> bool) {
        self.attributes.retain(predicate);
    }

    fn push_unique(&mut self, attribute: ManifestAttribute, line: usize) -> Result<()> {
        if self.contains(attribute.name()) {
            return Err(invalid_manifest(
                line,
                format!("duplicate attribute `{}`", attribute.name()),
            ));
        }
        self.attributes.push(attribute);
        Ok(())
    }
}

/// One named manifest section.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NamedManifestSection {
    name: String,
    attributes: ManifestSection,
}

impl NamedManifestSection {
    /// Creates an empty named section.
    ///
    /// # Errors
    ///
    /// Returns an error if the section name contains a line break or NUL.
    pub fn new(name: impl Into<String>) -> Result<Self> {
        let name = name.into();
        validate_attribute_value(&name)?;
        if name.is_empty() {
            return Err(Error::InvalidJar(
                "manifest section name must not be empty".to_owned(),
            ));
        }
        Ok(Self {
            name,
            attributes: ManifestSection::new(),
        })
    }

    /// Returns the exact section name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Replaces the exact section name.
    ///
    /// # Errors
    ///
    /// Returns an error if the name is empty or contains a line break or NUL.
    pub fn set_name(&mut self, name: impl Into<String>) -> Result<()> {
        let replacement = Self::new(name)?;
        self.name = replacement.name;
        Ok(())
    }

    /// Returns this section's non-`Name` attributes.
    #[must_use]
    pub const fn attributes(&self) -> &ManifestSection {
        &self.attributes
    }

    /// Returns this section's non-`Name` attributes for editing.
    #[must_use]
    pub const fn attributes_mut(&mut self) -> &mut ManifestSection {
        &mut self.attributes
    }
}

/// Parsed and editable `META-INF/MANIFEST.MF` contents.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Manifest {
    main: ManifestSection,
    sections: Vec<NamedManifestSection>,
}

impl Manifest {
    /// Creates a manifest containing `Manifest-Version: 1.0`.
    #[must_use]
    pub fn new() -> Self {
        let mut main = ManifestSection::new();
        main.attributes.push(ManifestAttribute {
            name: MANIFEST_VERSION_HEADER.to_owned(),
            value: DEFAULT_MANIFEST_VERSION.to_owned(),
        });
        Self {
            main,
            sections: Vec::new(),
        }
    }

    /// Parses manifest bytes, accepting CRLF, LF, or CR line endings.
    ///
    /// Continuation lines are unfolded and duplicate named sections are
    /// merged in encounter order, with later values replacing earlier ones.
    ///
    /// # Errors
    ///
    /// Returns an error with a physical line number for malformed UTF-8,
    /// continuations, headers, duplicates, or section structure.
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        let groups = logical_sections(bytes)?;
        let Some(main_group) = groups.first() else {
            return Err(Error::InvalidJar("manifest is empty".to_owned()));
        };
        let mut main = ManifestSection::new();
        for header in main_group {
            main.push_unique(
                ManifestAttribute::new(&header.name, &header.value)
                    .map_err(|error| with_manifest_line(error, header.line))?,
                header.line,
            )?;
        }
        validate_main_section(&main)?;

        let mut sections: Vec<NamedManifestSection> = Vec::new();
        for group in groups.iter().skip(MAIN_SECTION_COUNT) {
            if group.is_empty() {
                continue;
            }
            let first = &group[FIRST_HEADER_INDEX];
            if !first.name.eq_ignore_ascii_case(NAME_HEADER) {
                return Err(invalid_manifest(
                    first.line,
                    "named section does not begin with `Name`",
                ));
            }
            let section_index = if let Some(index) = sections
                .iter()
                .position(|section| section.name == first.value)
            {
                index
            } else {
                sections.push(
                    NamedManifestSection::new(&first.value)
                        .map_err(|error| with_manifest_line(error, first.line))?,
                );
                sections.len() - 1
            };
            let section = &mut sections[section_index];
            let mut encountered = ManifestSection::new();
            for header in group.iter().skip(FOLLOWING_HEADER_INDEX) {
                if header.name.eq_ignore_ascii_case(NAME_HEADER) {
                    return Err(invalid_manifest(
                        header.line,
                        "`Name` may only be the first header in a named section",
                    ));
                }
                let attribute = ManifestAttribute::new(&header.name, &header.value)
                    .map_err(|error| with_manifest_line(error, header.line))?;
                encountered.push_unique(attribute.clone(), header.line)?;
                section.attributes.set(attribute.name, attribute.value)?;
            }
        }
        Ok(Self { main, sections })
    }

    /// Returns the main section.
    #[must_use]
    pub const fn main(&self) -> &ManifestSection {
        &self.main
    }

    /// Returns the main section for editing.
    #[must_use]
    pub const fn main_mut(&mut self) -> &mut ManifestSection {
        &mut self.main
    }

    /// Returns named sections in serialization order.
    #[must_use]
    pub fn sections(&self) -> &[NamedManifestSection] {
        &self.sections
    }

    /// Returns named sections for editing.
    #[must_use]
    pub fn sections_mut(&mut self) -> &mut [NamedManifestSection] {
        &mut self.sections
    }

    /// Finds a named section by its exact, case-sensitive name.
    #[must_use]
    pub fn section(&self, name: &str) -> Option<&NamedManifestSection> {
        self.sections.iter().find(|section| section.name == name)
    }

    /// Finds a mutable named section by its exact, case-sensitive name.
    #[must_use]
    pub fn section_mut(&mut self, name: &str) -> Option<&mut NamedManifestSection> {
        self.sections
            .iter_mut()
            .find(|section| section.name == name)
    }

    /// Returns an existing named section or appends a new empty one.
    ///
    /// # Errors
    ///
    /// Returns an error if the section name is invalid.
    pub fn ensure_section(&mut self, name: impl Into<String>) -> Result<&mut NamedManifestSection> {
        let name = name.into();
        if let Some(index) = self
            .sections
            .iter()
            .position(|section| section.name == name)
        {
            return Ok(&mut self.sections[index]);
        }
        self.sections.push(NamedManifestSection::new(name)?);
        let index = self.sections.len() - 1;
        Ok(&mut self.sections[index])
    }

    /// Removes a named section by its exact, case-sensitive name.
    pub fn remove_section(&mut self, name: &str) -> Option<NamedManifestSection> {
        let index = self
            .sections
            .iter()
            .position(|section| section.name == name)?;
        Some(self.sections.remove(index))
    }

    /// Removes digest attributes used by JAR signing.
    ///
    /// Named sections left with no attributes are removed as stale signature
    /// bookkeeping.
    pub fn strip_digest_attributes(&mut self) {
        self.main
            .retain(|attribute| !is_digest_attribute(attribute.name()));
        for section in &mut self.sections {
            section
                .attributes
                .retain(|attribute| !is_digest_attribute(attribute.name()));
        }
        self.sections
            .retain(|section| !section.attributes.is_empty());
    }

    /// Assembles a canonical manifest using CRLF and 72-byte wrapped lines.
    ///
    /// # Errors
    ///
    /// Returns an error if editing left the mandatory version header missing
    /// or introduced a reserved `Name` attribute in the wrong location.
    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        validate_main_section(&self.main)?;
        let mut output = Vec::new();
        for attribute in &self.main.attributes {
            write_attribute(&mut output, attribute.name(), attribute.value())?;
        }
        output.extend_from_slice(MANIFEST_LINE_ENDING);
        for section in &self.sections {
            write_attribute(&mut output, NAME_HEADER, &section.name)?;
            for attribute in &section.attributes.attributes {
                if attribute.name.eq_ignore_ascii_case(NAME_HEADER) {
                    return Err(Error::InvalidJar(
                        "named-section attributes must not contain `Name`".to_owned(),
                    ));
                }
                write_attribute(&mut output, attribute.name(), attribute.value())?;
            }
            output.extend_from_slice(MANIFEST_LINE_ENDING);
        }
        Ok(output)
    }
}

impl Default for Manifest {
    fn default() -> Self {
        Self::new()
    }
}

impl JarFile {
    /// Parses the archive manifest when one is present.
    ///
    /// Manifest lookup is ASCII case-insensitive for compatibility with JAR
    /// signing rules.
    ///
    /// # Errors
    ///
    /// Returns an error for duplicate manifest entries, unreadable data, or an
    /// invalid manifest.
    pub fn manifest(&self) -> Result<Option<Manifest>> {
        let Some(id) = self.manifest_entry_id()? else {
            return Ok(None);
        };
        Ok(Some(Manifest::parse(&self.read_entry_by_id(id)?)?))
    }

    /// Adds or replaces `META-INF/MANIFEST.MF`.
    ///
    /// A new manifest is placed first, after an existing leading `META-INF/`
    /// directory marker.
    ///
    /// # Errors
    ///
    /// Returns an error if the manifest cannot be assembled or duplicate
    /// manifest entries make replacement ambiguous.
    pub fn set_manifest(&mut self, manifest: &Manifest) -> Result<EntryId> {
        let bytes = manifest.to_bytes()?;
        if let Some(id) = self.manifest_entry_id()? {
            self.replace_entry_by_id(id, bytes)?;
            return Ok(id);
        }
        let insertion = usize::from(self.entries.first().is_some_and(|entry| {
            entry.kind == EntryKind::Directory && entry.name == META_INF_DIRECTORY
        }));
        self.insert_file(insertion, MANIFEST_ENTRY, bytes, EntryMetadata::default())
    }

    /// Removes and returns the parsed manifest when present.
    ///
    /// # Errors
    ///
    /// Returns an error for duplicate or malformed manifests or unreadable
    /// payload data.
    pub fn remove_manifest(&mut self) -> Result<Option<Manifest>> {
        let Some(id) = self.manifest_entry_id()? else {
            return Ok(None);
        };
        let manifest = Manifest::parse(&self.read_entry_by_id(id)?)?;
        self.remove_entry_by_id(id)?;
        Ok(Some(manifest))
    }

    /// Returns whether the manifest enables multi-release JAR lookup.
    ///
    /// # Errors
    ///
    /// Returns an error if the manifest is ambiguous or malformed.
    pub fn is_multi_release(&self) -> Result<bool> {
        Ok(self
            .manifest()?
            .and_then(|manifest| manifest.main().get(MULTI_RELEASE_HEADER).map(str::to_owned))
            .is_some_and(|value| value.eq_ignore_ascii_case(MULTI_RELEASE_ENABLED_VALUE)))
    }

    pub(crate) fn manifest_entry_id(&self) -> Result<Option<EntryId>> {
        let matches: Vec<_> = self
            .entries
            .iter()
            .filter(|entry| entry.name.eq_ignore_ascii_case(MANIFEST_ENTRY))
            .map(|entry| entry.id)
            .collect();
        match matches.as_slice() {
            [] => Ok(None),
            [id] => Ok(Some(*id)),
            _ => Err(Error::AmbiguousJarEntry {
                name: MANIFEST_ENTRY.to_owned(),
                count: matches.len(),
            }),
        }
    }
}

#[derive(Debug)]
struct LogicalHeader {
    line: usize,
    name: String,
    value: String,
}

fn logical_sections(bytes: &[u8]) -> Result<Vec<Vec<LogicalHeader>>> {
    let physical = physical_lines(bytes);
    let mut groups = vec![Vec::new()];
    let mut pending: Option<(usize, Vec<u8>)> = None;
    for (line_number, line) in physical {
        if line.len() > MAX_PHYSICAL_LINE {
            return Err(invalid_manifest(
                line_number,
                format!("physical line exceeds {MAX_PHYSICAL_LINE} bytes"),
            ));
        }
        if line.is_empty() {
            let group = groups.last_mut().ok_or_else(|| {
                Error::InvalidJar("internal manifest section state is empty".to_owned())
            })?;
            flush_header(&mut pending, group)?;
            if !group.is_empty() {
                groups.push(Vec::new());
            }
            continue;
        }
        if line.first() == Some(&CONTINUATION_MARKER) {
            let Some((_, header)) = pending.as_mut() else {
                return Err(invalid_manifest(
                    line_number,
                    "continuation line has no preceding header",
                ));
            };
            header.extend_from_slice(&line[CONTINUATION_PREFIX_WIDTH..]);
            continue;
        }
        let group = groups.last_mut().ok_or_else(|| {
            Error::InvalidJar("internal manifest section state is empty".to_owned())
        })?;
        flush_header(&mut pending, group)?;
        pending = Some((line_number, line.to_vec()));
    }
    let group = groups
        .last_mut()
        .ok_or_else(|| Error::InvalidJar("internal manifest section state is empty".to_owned()))?;
    flush_header(&mut pending, group)?;
    while groups.last().is_some_and(Vec::is_empty) {
        groups.pop();
    }
    Ok(groups)
}

fn physical_lines(bytes: &[u8]) -> Vec<(usize, &[u8])> {
    let mut lines = Vec::new();
    let mut start = INITIAL_BYTE_OFFSET;
    let mut line_number = FIRST_PHYSICAL_LINE_NUMBER;
    let mut index = INITIAL_BYTE_OFFSET;
    while index < bytes.len() {
        if bytes[index] == CARRIAGE_RETURN || bytes[index] == LINE_FEED {
            lines.push((line_number, &bytes[start..index]));
            if bytes[index] == CARRIAGE_RETURN
                && bytes.get(index + BYTE_OFFSET_INCREMENT) == Some(&LINE_FEED)
            {
                index += BYTE_OFFSET_INCREMENT;
            }
            index += BYTE_OFFSET_INCREMENT;
            start = index;
            line_number += LINE_NUMBER_INCREMENT;
        } else {
            index += BYTE_OFFSET_INCREMENT;
        }
    }
    if start < bytes.len() {
        lines.push((line_number, &bytes[start..]));
    }
    lines
}

fn flush_header(
    pending: &mut Option<(usize, Vec<u8>)>,
    output: &mut Vec<LogicalHeader>,
) -> Result<()> {
    let Some((line, bytes)) = pending.take() else {
        return Ok(());
    };
    let text = std::str::from_utf8(&bytes)
        .map_err(|_| invalid_manifest(line, "header is not valid UTF-8"))?;
    let Some((name, value)) = text.split_once(MANIFEST_HEADER_SEPARATOR) else {
        return Err(invalid_manifest(line, "header is missing `: `"));
    };
    output.push(LogicalHeader {
        line,
        name: name.to_owned(),
        value: value.to_owned(),
    });
    Ok(())
}

fn validate_main_section(main: &ManifestSection) -> Result<()> {
    let Some(first) = main.attributes.first() else {
        return Err(Error::InvalidJar(
            "manifest main section is empty".to_owned(),
        ));
    };
    if !first.name.eq_ignore_ascii_case(MANIFEST_VERSION_HEADER) {
        return Err(Error::InvalidJar(
            "manifest must begin with `Manifest-Version`".to_owned(),
        ));
    }
    Ok(())
}

fn validate_attribute_name(name: &str) -> Result<()> {
    if name.is_empty() || name.len() > MAX_ATTRIBUTE_NAME_LENGTH {
        return Err(Error::InvalidJar(format!(
            "manifest attribute name must contain {MIN_ATTRIBUTE_NAME_LENGTH} through {MAX_ATTRIBUTE_NAME_LENGTH} ASCII bytes"
        )));
    }
    if !name.bytes().all(|byte| {
        byte.is_ascii_alphanumeric()
            || matches!(byte, ATTRIBUTE_NAME_UNDERSCORE | ATTRIBUTE_NAME_HYPHEN)
    }) {
        return Err(Error::InvalidJar(format!(
            "invalid manifest attribute name `{name}`"
        )));
    }
    Ok(())
}

fn validate_attribute_value(value: &str) -> Result<()> {
    if value.contains([
        NUL_CHARACTER,
        char::from(CARRIAGE_RETURN),
        char::from(LINE_FEED),
    ]) {
        return Err(Error::InvalidJar(
            "manifest values must not contain NUL or line breaks".to_owned(),
        ));
    }
    if value.len() > MAX_ATTRIBUTE_VALUE_LENGTH {
        return Err(Error::InvalidJar(format!(
            "manifest value exceeds the {MAX_ATTRIBUTE_VALUE_LENGTH}-byte limit"
        )));
    }
    Ok(())
}

fn write_attribute(output: &mut Vec<u8>, name: &str, value: &str) -> Result<()> {
    validate_attribute_name(name)?;
    validate_attribute_value(value)?;
    let prefix = format!("{name}{MANIFEST_HEADER_SEPARATOR}");
    let mut remainder = value;
    let first_capacity = MAX_PHYSICAL_LINE - prefix.len();
    output.extend_from_slice(prefix.as_bytes());
    let first = utf8_prefix(remainder, first_capacity);
    output.extend_from_slice(first.as_bytes());
    output.extend_from_slice(MANIFEST_LINE_ENDING);
    remainder = &remainder[first.len()..];
    while !remainder.is_empty() {
        output.push(CONTINUATION_MARKER);
        let chunk = utf8_prefix(remainder, MAX_PHYSICAL_LINE - CONTINUATION_PREFIX_WIDTH);
        output.extend_from_slice(chunk.as_bytes());
        output.extend_from_slice(MANIFEST_LINE_ENDING);
        remainder = &remainder[chunk.len()..];
    }
    Ok(())
}

fn utf8_prefix(value: &str, limit: usize) -> &str {
    let mut end = value.len().min(limit);
    while !value.is_char_boundary(end) {
        end -= BYTE_OFFSET_INCREMENT;
    }
    &value[..end]
}

fn is_digest_attribute(name: &str) -> bool {
    let marker = DIGEST_ATTRIBUTE_MARKER.as_bytes();
    name.as_bytes()
        .windows(marker.len())
        .any(|candidate| candidate.eq_ignore_ascii_case(marker))
}

fn invalid_manifest(line: usize, message: impl std::fmt::Display) -> Error {
    Error::InvalidJar(format!("manifest line {line}: {message}"))
}

fn with_manifest_line(error: Error, line: usize) -> Error {
    invalid_manifest(line, error)
}

#[cfg(test)]
mod tests {
    use super::Manifest;

    #[test]
    fn unfolds_and_canonically_wraps_utf8_values() {
        let input = b"Manifest-Version: 1.0\nLong: abc\n def\n\nName: x\nSHA-256-Digest: yes\n\n";
        let mut manifest = Manifest::parse(input).expect("valid manifest");
        assert_eq!(manifest.main().get("long"), Some("abcdef"));
        manifest
            .main_mut()
            .set("Unicode", "\u{1f980}".repeat(40))
            .expect("valid UTF-8 value");
        let bytes = manifest.to_bytes().expect("manifest assembles");
        assert!(
            bytes
                .split(|byte| *byte == b'\n')
                .all(|line| line.len() <= 73)
        );
        assert_eq!(Manifest::parse(&bytes).expect("round trip"), manifest);
    }

    #[test]
    fn strips_digest_sections() {
        let input = b"Manifest-Version: 1.0\r\nSHA-256-Digest-Manifest: x\r\n\r\nName: a.class\r\nSHA-256-Digest: y\r\n\r\n";
        let mut manifest = Manifest::parse(input).expect("valid manifest");
        manifest.strip_digest_attributes();
        assert!(!manifest.main().contains("SHA-256-Digest-Manifest"));
        assert!(manifest.sections().is_empty());
    }
}

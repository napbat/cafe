//! DEX version 041 multi-header container parsing and assembly.

use crate::file::header::CONTAINER_HEADER_SIZE;
use crate::{Error, Result};

use super::{DexFile, DexVersion, parse, write};

const CONTAINER_ALIGNMENT: u32 = 4;

/// Parsed and editable DEX version 041 multi-header container.
///
/// Each member retains a normal [`DexFile`] model while all binary offsets are
/// interpreted relative to the complete container. Pristine input bytes are
/// emitted exactly; an edited container is rebuilt deterministically.
#[derive(Debug, Clone)]
pub struct DexContainer {
    members: Vec<DexFile>,
    original: Option<Vec<u8>>,
    dirty: bool,
}

impl DexContainer {
    /// Creates an empty editable container.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            members: Vec::new(),
            original: None,
            dirty: true,
        }
    }

    /// Parses every header in a complete DEX 041 container.
    ///
    /// # Errors
    ///
    /// Returns an error for an empty input, a non-041 member, inconsistent
    /// container coordinates, unaligned or overlapping headers, a truncated
    /// member, invalid integrity fields, or malformed member content.
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        if bytes.is_empty() {
            return Err(Error::invalid_dex(0, "empty DEX 041 container"));
        }
        let physical_size = u32::try_from(bytes.len())
            .map_err(|_| Error::invalid_dex(0, "DEX container exceeds 32-bit address space"))?;
        let mut members = Vec::new();
        let mut header_offset = 0_u32;
        loop {
            if !header_offset.is_multiple_of(CONTAINER_ALIGNMENT) {
                return Err(Error::invalid_dex(
                    usize::try_from(header_offset).unwrap_or(usize::MAX),
                    "DEX 041 header is not four-byte aligned",
                ));
            }
            let offset = usize::try_from(header_offset)
                .map_err(|_| Error::invalid_dex(0, "header offset does not fit this platform"))?;
            let member = parse::parse(bytes, offset)?;
            if member.version() != DexVersion::V041 {
                return Err(Error::invalid_dex(
                    offset,
                    "every multi-header container member must use DEX version 041",
                ));
            }
            if member.header.container_size != physical_size {
                return Err(Error::invalid_dex(
                    offset,
                    "DEX 041 members disagree about the physical container size",
                ));
            }
            let file_size = member.header.file_size;
            if file_size < CONTAINER_HEADER_SIZE {
                return Err(Error::invalid_dex(
                    offset,
                    "DEX 041 member is smaller than its header",
                ));
            }
            members.push(member);
            let next = header_offset
                .checked_add(file_size)
                .ok_or_else(|| Error::invalid_dex(offset, "member end offset overflowed"))?;
            if next == physical_size {
                break;
            }
            if next > physical_size {
                return Err(Error::invalid_dex(
                    offset,
                    "member extends beyond the container",
                ));
            }
            header_offset = next;
        }
        Ok(Self {
            members,
            original: Some(bytes.to_vec()),
            dirty: false,
        })
    }

    /// Returns members in physical header order.
    #[must_use]
    pub fn members(&self) -> &[DexFile] {
        &self.members
    }

    /// Returns mutable members and marks the container as edited.
    pub fn members_mut(&mut self) -> &mut Vec<DexFile> {
        self.dirty = true;
        &mut self.members
    }

    /// Appends one version 041 member.
    ///
    /// # Errors
    ///
    /// Returns an error when the member uses a legacy DEX version.
    pub fn push(&mut self, member: DexFile) -> Result<usize> {
        if member.version() != DexVersion::V041 {
            return Err(Error::invalid_assembly(
                "DEX container members must use version 041",
            ));
        }
        let index = self.members.len();
        self.members.push(member);
        self.dirty = true;
        Ok(index)
    }

    /// Returns whether neither the container nor any member has been edited.
    #[must_use]
    pub fn is_pristine(&self) -> bool {
        !self.dirty && self.members.iter().all(DexFile::is_pristine)
    }

    /// Returns exact original container bytes while pristine.
    #[must_use]
    pub fn original_bytes(&self) -> Option<&[u8]> {
        self.is_pristine()
            .then_some(self.original.as_deref())
            .flatten()
    }

    /// Assembles all members with container-relative offsets and fresh
    /// per-member integrity fields.
    ///
    /// # Errors
    ///
    /// Returns an error for an empty container, a legacy member, an invalid
    /// member model, overflow, or output that fails complete self-validation.
    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        if let Some(original) = self.original_bytes() {
            return Ok(original.to_vec());
        }
        if self.members.is_empty() {
            return Err(Error::invalid_assembly(
                "a DEX 041 container must have at least one member",
            ));
        }

        let mut offsets = Vec::with_capacity(self.members.len());
        let mut sizes = Vec::with_capacity(self.members.len());
        let mut header_offset = 0_u32;
        for member in &self.members {
            if member.version() != DexVersion::V041 {
                return Err(Error::invalid_assembly(
                    "DEX container members must use version 041",
                ));
            }
            if !header_offset.is_multiple_of(CONTAINER_ALIGNMENT) {
                return Err(Error::invalid_assembly(
                    "canonical DEX 041 member is not four-byte aligned",
                ));
            }
            offsets.push(header_offset);
            let provisional = write::assemble_member(member, header_offset, u32::MAX)?;
            let size = u32::try_from(provisional.len())
                .map_err(|_| Error::invalid_assembly("DEX 041 member exceeds 32-bit size"))?;
            if !size.is_multiple_of(CONTAINER_ALIGNMENT) {
                return Err(Error::invalid_assembly(
                    "canonical DEX 041 member size is not four-byte aligned",
                ));
            }
            sizes.push(size);
            header_offset = header_offset
                .checked_add(size)
                .ok_or_else(|| Error::invalid_assembly("DEX container size overflowed"))?;
        }
        let container_size = header_offset;
        let mut bytes = Vec::with_capacity(usize::try_from(container_size).map_err(|_| {
            Error::invalid_assembly("DEX container size does not fit this platform")
        })?);
        for ((member, &offset), &expected_size) in self.members.iter().zip(&offsets).zip(&sizes) {
            let encoded = write::assemble_member(member, offset, container_size)?;
            let expected_size = usize::try_from(expected_size).map_err(|_| {
                Error::invalid_assembly("DEX member size does not fit this platform")
            })?;
            if encoded.len() != expected_size {
                return Err(Error::invalid_assembly(
                    "DEX 041 member size changed between layout passes",
                ));
            }
            bytes.extend_from_slice(&encoded);
        }
        Self::parse(&bytes).map_err(|error| {
            Error::invalid_assembly(format!(
                "canonical DEX 041 container failed self-validation: {error}"
            ))
        })?;
        Ok(bytes)
    }

    /// Applies an edit transaction and restores the prior container if either
    /// the closure or complete reassembly fails.
    ///
    /// # Errors
    ///
    /// Returns the edit or assembly-validation error.
    pub fn try_edit<T>(&mut self, edit: impl FnOnce(&mut Self) -> Result<T>) -> Result<T> {
        let previous = self.clone();
        let result = edit(self).and_then(|value| self.to_bytes().map(|_| value));
        if result.is_err() {
            *self = previous;
        }
        result
    }
}

impl Default for DexContainer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn assembles_parses_and_preserves_multiple_members() -> Result<()> {
        let mut container = DexContainer::new();
        container.push(DexFile::new(DexVersion::V041))?;
        container.push(DexFile::new(DexVersion::V041))?;

        let bytes = container.to_bytes()?;
        let parsed = DexContainer::parse(&bytes)?;

        assert_eq!(parsed.members().len(), 2);
        assert_eq!(parsed.members()[0].header().header_offset, 0);
        assert_eq!(
            parsed.members()[1].header().header_offset,
            parsed.members()[0].header().file_size
        );
        assert_eq!(parsed.to_bytes()?, bytes);
        assert!(parsed.is_pristine());
        Ok(())
    }

    #[test]
    fn rejects_legacy_members() {
        let mut container = DexContainer::new();
        assert!(container.push(DexFile::new(DexVersion::V040)).is_err());
    }

    #[test]
    fn single_file_parser_redirects_multi_header_inputs() -> Result<()> {
        let mut container = DexContainer::new();
        container.push(DexFile::new(DexVersion::V041))?;
        container.push(DexFile::new(DexVersion::V041))?;
        let bytes = container.to_bytes()?;
        assert!(DexFile::parse(&bytes).is_err());
        Ok(())
    }
}

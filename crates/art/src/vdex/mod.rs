//! VDEX versions 009, 012, 020, 021, and sectioned 027.

mod build;
mod canonical;
mod parse;

use std::collections::BTreeMap;
use std::ops::Range;

use dex::file::{CompactDexFile, DexSourceFormat};

use crate::{Error, Result};

/// VDEX file signature.
pub const VDEX_MAGIC: &[u8; 4] = b"vdex";

/// Explicitly supported VDEX layout version.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum VdexVersion {
    /// Android 8-era legacy method/code-offset quickening table.
    V009,
    /// Android 9 compact per-method quickening offset tables.
    V012,
    /// Early split verifier/dex-section header layout.
    V020,
    /// Android 10 split layout with context strings.
    V021,
    /// Current section-directory layout.
    V027,
}

impl VdexVersion {
    /// Every layout accepted by this crate.
    pub const ALL: &[Self] = &[Self::V009, Self::V012, Self::V020, Self::V021, Self::V027];

    /// Returns the primary three-digit version.
    #[must_use]
    pub const fn digits(self) -> [u8; 3] {
        match self {
            Self::V009 => *b"009",
            Self::V012 => *b"012",
            Self::V020 => *b"020",
            Self::V021 => *b"021",
            Self::V027 => *b"027",
        }
    }
}

/// Physical family selected by a VDEX version.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum VdexLayout {
    /// Single fixed header followed by checksums and sequential payloads.
    Legacy,
    /// Separate verifier-dependency and optional DEX-section headers.
    Split,
    /// Version 027 section directory.
    Sectioned,
}

/// Typed VDEX section kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum VdexSection {
    /// Per-DEX location checksums.
    Checksums,
    /// Main standard or `CompactDex` bytes.
    DexFiles,
    /// Shared `CompactDex` data.
    SharedData,
    /// Verifier dependency payload.
    VerifierDependencies,
    /// ART quickening records and offset tables.
    Quickening,
    /// Boot-class-path checksum string.
    BootClasspathChecksums,
    /// Class-loader context string.
    ClassLoaderContext,
    /// Per-DEX type lookup tables.
    TypeLookupTables,
    /// Section-directory kind not interpreted by this crate.
    Unknown(u32),
}

/// One deterministic DEX member inventory record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VdexDexMember {
    /// Zero-based native VDEX member index.
    pub index: u32,
    /// Location checksum stored by VDEX.
    pub checksum: u32,
    /// Exact main-section byte range in the VDEX container.
    pub main_range: Range<usize>,
    /// Standard DEX or `CompactDex` source identity.
    pub source_format: DexSourceFormat,
    /// Offset of this member's embedded quickening offset table, when present.
    pub quickening_table_offset: Option<u32>,
}

/// Parsed runtime DEX member retaining its source encoding.
#[derive(Debug, Clone)]
pub enum RuntimeDex {
    /// Canonical standard DEX, dequickened when needed.
    Standard(dex::DexFile),
    /// `CompactDex` with its explicit shared-data section.
    Compact(CompactDexFile),
}

/// Parsed immutable VDEX container.
#[derive(Debug, Clone)]
pub struct VdexFile {
    version: VdexVersion,
    layout: VdexLayout,
    bytes: Vec<u8>,
    sections: BTreeMap<VdexSection, Range<usize>>,
    members: Vec<VdexDexMember>,
}

impl VdexFile {
    /// Parses one supported VDEX layout and validates all declared ranges,
    /// member counts, alignments, versions, and DEX source signatures.
    ///
    /// # Errors
    ///
    /// Returns a contextual error for unsupported or malformed input.
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        parse::parse(bytes)
    }

    /// Builds deterministic sectioned VDEX 027 from canonical standard DEX files.
    ///
    /// # Errors
    ///
    /// Returns an error when a DEX fails assembly or section coordinates exceed
    /// 32 bits.
    pub fn from_standard_dex_files(
        files: &[dex::DexFile],
        verifier_dependencies: &[u8],
        type_lookup_tables: &[u8],
    ) -> Result<Self> {
        build::sectioned(files, verifier_dependencies, type_lookup_tables)
    }

    /// Returns the exact supported version.
    #[must_use]
    pub const fn version(&self) -> VdexVersion {
        self.version
    }

    /// Returns the physical layout family.
    #[must_use]
    pub const fn layout(&self) -> VdexLayout {
        self.layout
    }

    /// Returns DEX members in deterministic native order.
    #[must_use]
    pub fn members(&self) -> &[VdexDexMember] {
        &self.members
    }

    /// Returns one exact section, or `None` when the layout omits it.
    #[must_use]
    pub fn section(&self, kind: VdexSection) -> Option<&[u8]> {
        self.sections
            .get(&kind)
            .and_then(|range| self.bytes.get(range.clone()))
    }

    /// Returns one member's exact main bytes.
    #[must_use]
    pub fn member_bytes(&self, index: u32) -> Option<&[u8]> {
        self.members
            .get(usize::try_from(index).ok()?)
            .and_then(|member| self.bytes.get(member.main_range.clone()))
    }

    /// Parses one member and retains standard-versus-`CompactDex` identity.
    ///
    /// Standard DEX is dequickened before parsing. `CompactDex` remains in its
    /// physical split representation and exposes its own canonical code-item
    /// codec.
    ///
    /// # Errors
    ///
    /// Returns an error for an invalid index, malformed DEX, malformed
    /// quickening data, or inconsistent shared `CompactDex` data.
    pub fn runtime_dex(&self, index: u32) -> Result<RuntimeDex> {
        let member = self.member(index)?;
        match member.source_format {
            DexSourceFormat::Standard(_) => {
                Ok(RuntimeDex::Standard(self.canonical_standard_dex(index)?))
            }
            DexSourceFormat::Compact(_) => {
                let main = self
                    .member_bytes(index)
                    .ok_or_else(|| Error::invalid("VDEX", 0, "member main range disappeared"))?;
                let shared = self.section(VdexSection::SharedData).ok_or_else(|| {
                    Error::invalid(
                        "VDEX",
                        member.main_range.start,
                        "CompactDex member has no shared data section",
                    )
                })?;
                Ok(RuntimeDex::Compact(CompactDexFile::parse_sections(
                    main, shared,
                )?))
            }
        }
    }

    /// Restores and parses one standard DEX member.
    ///
    /// # Errors
    ///
    /// Returns an error for `CompactDex` members, malformed method tables,
    /// missing/mismatched quickening data, or invalid canonical DEX.
    pub fn canonical_standard_dex(&self, index: u32) -> Result<dex::DexFile> {
        canonical::standard_dex(self, index)
    }

    /// Parses the saved quickening indices for one method.
    ///
    /// Empty information is returned when the member or method was not
    /// quickened.
    ///
    /// # Errors
    ///
    /// Returns an error for invalid indices or malformed compact offset tables.
    pub fn quickening_info(
        &self,
        dex_index: u32,
        method_index: u32,
    ) -> Result<crate::quickening::QuickeningInfo> {
        canonical::quickening_info(self, dex_index, method_index)
    }

    /// Lowers every standard DEX member into Program modules after canonicalization.
    ///
    /// # Errors
    ///
    /// Returns an explicit error when the VDEX contains `CompactDex`, because its
    /// complete identifier/data layout must remain format-qualified rather than
    /// being silently normalized by an ART container adapter.
    pub fn to_modules(
        &self,
        method_bodies: dex::program::MethodBodyMode,
    ) -> Result<Vec<program::Module>> {
        let mut modules = Vec::with_capacity(self.members.len());
        for member in &self.members {
            if matches!(member.source_format, DexSourceFormat::Compact(_)) {
                return Err(Error::MissingCanonicalizationMetadata {
                    artifact: format!("VDEX member {}", member.index),
                    message: "complete CompactDex-to-canonical DEX layout conversion is not implied by container parsing"
                        .to_owned(),
                });
            }
            let file = self.canonical_standard_dex(member.index)?;
            modules.push(dex::program::lower_file_named_with_options(
                &file,
                format!("vdex!classes{}.dex", member.index + 1),
                dex::program::ProgramOptions { method_bodies },
            )?);
        }
        Ok(modules)
    }

    /// Lowers every standard member into verified shared disassembly after
    /// canonicalization.
    ///
    /// # Errors
    ///
    /// Returns the same `CompactDex` boundary error as [`Self::to_modules`] or a
    /// DEX/disassembly validation error.
    pub fn disassemblies(&self) -> Result<Vec<disassembler::Disassembly>> {
        let mut output = Vec::with_capacity(self.members.len());
        for member in &self.members {
            if matches!(member.source_format, DexSourceFormat::Compact(_)) {
                return Err(Error::MissingCanonicalizationMetadata {
                    artifact: format!("VDEX member {}", member.index),
                    message:
                        "CompactDex bodies remain available through the CompactDex code-item codec"
                            .to_owned(),
                });
            }
            let file = self.canonical_standard_dex(member.index)?;
            output.push(dex::disassembly::lower_file_named(
                &file,
                format!("vdex!classes{}.dex", member.index + 1),
            )?);
        }
        Ok(output)
    }

    /// Reassembles the exact immutable VDEX bytes.
    ///
    /// # Errors
    ///
    /// This immutable operation currently cannot fail; the result form keeps
    /// serialization APIs uniform if checked rewriting is added later.
    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        Ok(self.bytes.clone())
    }

    pub(super) fn member(&self, index: u32) -> Result<&VdexDexMember> {
        self.members
            .get(
                usize::try_from(index).map_err(|_| {
                    Error::invalid("VDEX", 0, "DEX member index does not fit platform")
                })?,
            )
            .ok_or_else(|| Error::invalid("VDEX", 0, format!("DEX member {index} is absent")))
    }
}

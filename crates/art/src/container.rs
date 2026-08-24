//! Runtime-container detection without native executable decoding.

use crate::{Error, Result};

/// Supported Android runtime container kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ArtFormat {
    /// ART verifier/Dex container.
    Vdex,
    /// Dalvik optimized DEX container.
    Odex,
    /// ART ahead-of-time metadata embedded in an OAT or ELF file.
    Oat,
}

/// Parsed runtime container selected by its exact signature.
#[derive(Debug, Clone)]
pub enum ArtContainer {
    /// VDEX container.
    Vdex(crate::vdex::VdexFile),
    /// Legacy ODEX container.
    Odex(crate::odex::OdexFile),
    /// OAT metadata and opaque native payload.
    Oat(crate::oat::OatFile),
}

impl ArtContainer {
    /// Detects and parses one supported runtime container.
    ///
    /// # Errors
    ///
    /// Returns an error for unknown signatures or malformed selected formats.
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        if bytes.starts_with(crate::vdex::VDEX_MAGIC) {
            Ok(Self::Vdex(crate::vdex::VdexFile::parse(bytes)?))
        } else if bytes.starts_with(crate::odex::ODEX_MAGIC) {
            Ok(Self::Odex(crate::odex::OdexFile::parse(bytes)?))
        } else if crate::oat::OatFile::contains_header(bytes) {
            Ok(Self::Oat(crate::oat::OatFile::parse(bytes)?))
        } else {
            Err(Error::UnrecognizedContainer)
        }
    }

    /// Returns the selected container kind.
    #[must_use]
    pub const fn format(&self) -> ArtFormat {
        match self {
            Self::Vdex(_) => ArtFormat::Vdex,
            Self::Odex(_) => ArtFormat::Odex,
            Self::Oat(_) => ArtFormat::Oat,
        }
    }

    /// Reassembles the container exactly.
    ///
    /// # Errors
    ///
    /// Returns an error if an edited format-specific model is inconsistent.
    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        match self {
            Self::Vdex(file) => file.to_bytes(),
            Self::Odex(file) => file.to_bytes(),
            Self::Oat(file) => file.to_bytes(),
        }
    }
}

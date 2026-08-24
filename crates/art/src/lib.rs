//! Android runtime containers and canonical DEX restoration.
//!
//! This crate keeps VDEX, legacy ODEX, OAT metadata, and quickening state out
//! of the canonical DEX frontend. It never decodes native instructions. DEX
//! bodies are exposed to shared disassembly only after quickened opcodes have
//! been restored to their standard encodings.

mod binary;
mod container;
mod error;
pub mod oat;
pub mod odex;
pub mod quickening;
pub mod vdex;

pub use self::container::{ArtContainer, ArtFormat};
pub use self::error::{Error, Result};
pub use self::oat::{OatFile, OatHeader, OatVersion};
pub use self::odex::{OdexFile, OdexFlags, OdexHeader};
pub use self::quickening::{
    CanonicalCodeUnits, CanonicalPatch, QuickOpcode, QuickeningInfo, canonicalize_with_patches,
    dequicken_code_units,
};
pub use self::vdex::{RuntimeDex, VdexDexMember, VdexFile, VdexLayout, VdexSection, VdexVersion};

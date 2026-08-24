//! Lossless DEX models, Dalvik instruction handling, and Android containers.
//!
//! This crate keeps DEX-native tables, indices, code-unit addresses, encoded
//! values, annotations, and debugging state separate from the shared Cafe and
//! disassembly models. APK support is exposed as archive provenance around one
//! or more DEX artifacts; an APK is not treated as another instruction set.

pub mod apk;
pub mod cafe;
pub mod disassembly;
mod error;
pub mod file;
pub mod instruction;

pub use self::error::{Error, IdentifierTable, Result};
pub use self::file::{DexFile, DexVersion};

/// Conventional name of the primary DEX artifact in an Android archive.
pub const DEFAULT_DEX_FILE_NAME: &str = "classes.dex";

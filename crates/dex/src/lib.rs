//! Lossless DEX models, reversible Dalvik LLIL, bidirectional MLIL adaptation,
//! and Android containers.
//!
//! This crate keeps DEX-native tables, indices, code-unit addresses, encoded
//! values, annotations, and debugging state separate from the shared program
//! and disassembly models. APK support is exposed as archive provenance around
//! one or more DEX artifacts; an APK is not treated as another instruction set.

pub mod aab;
pub mod analysis;
pub mod apk;
pub mod corpus;
pub mod disassembly;
mod error;
pub mod file;
pub mod instruction;
pub mod llil;
pub mod mlil;
pub mod program;
pub mod rtl;

pub use self::error::{Error, IdentifierTable, Result};
pub use self::file::{CompactDexFile, CompactDexVersion, DexFile, DexSourceFormat, DexVersion};
pub use self::program::{
    DexEmissionError, DexEmissionOptions, DexEmitter, DexReferenceHandle,
    DexReferenceResolutionError, DexReferenceResolver, MethodBodyMode, ProgramOptions,
    SymbolicDexReferenceResolver, emit_module, lift_file, lift_file_named,
    lift_file_named_with_options, lift_file_with_options,
};

/// Conventional name of the primary DEX artifact in an Android archive.
pub const DEFAULT_DEX_FILE_NAME: &str = "classes.dex";

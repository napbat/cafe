//! Complete public entry point for Cafe's Java ecosystem tooling.
//!
//! Consumers need only this crate. Format-specific capabilities remain grouped
//! under [`java`], [`dex`], and [`jni`]; shared instruction and graph APIs live
//! under [`disassembler`]; and the owned definition model is available both at
//! this crate's root and under [`program`].

/// Android DEX files, Dalvik instructions, APKs, and program adapters.
pub use ::dex;
/// Shared Java-ecosystem disassembly IR and control-flow graphs.
pub use ::disassembler;
/// JVM class files, bytecode, JARs, and program adapters.
pub use ::java;
/// Typed Java Native Interface declarations, symbols, and artifact adapters.
pub use ::jni;
/// Owned modules, definitions, identities, and cross-module resolution.
pub use ::program;

/// Graph algorithms and data structures used by shared control-flow graphs.
pub use ::disassembler::cfglib;
pub use ::program::*;

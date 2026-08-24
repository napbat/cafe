//! Complete public entry point for Cafe's Java ecosystem tooling.
//!
//! Consumers need only this crate. Format-specific capabilities remain grouped
//! under [`java`], [`dex`], [`art`], and [`jni`]; shared instruction and graph
//! APIs live under [`disassembler`]; and the owned definition model is available
//! both at this crate's root and under [`program`].
//!
//! ```
//! use cafe::{Program, art, cfglib, dex, disassembler, java, jni, program};
//!
//! let owned = Program::new();
//! let dex_file = dex::DexFile::new(dex::DexVersion::V040);
//! assert!(owned.modules().next().is_none());
//! assert_eq!(dex_file.version(), dex::DexVersion::V040);
//! assert_eq!(
//!     cafe::BinaryFormat::JavaClass,
//!     disassembler::BinaryFormat::JavaClass,
//! );
//! let _: program::Program = owned;
//! let _ = std::any::type_name::<java::jar::JarFile>();
//! let _ = std::any::type_name::<jni::NativeMethod>();
//! let _ = std::any::type_name::<art::VdexFile>();
//! let _ = std::any::type_name::<cfglib::BlockId>();
//! ```

/// Android runtime VDEX, ODEX, OAT, and canonical dequickening support.
pub use ::art;
/// Android DEX/CompactDex, Dalvik instructions, APK/AAB, and shared adapters.
pub use ::dex;
/// Shared Java-ecosystem disassembly IR and control-flow graphs.
pub use ::disassembler;
/// JVM class files, bytecode, JAR/JMOD/JIMAGE, corpus, and shared adapters.
pub use ::java;
/// Typed Java Native Interface declarations, symbols, and artifact adapters.
pub use ::jni;
/// Owned modules, definitions, identities, and cross-module resolution.
pub use ::program;

/// Graph algorithms and data structures used by shared control-flow graphs.
pub use ::disassembler::cfglib;
pub use ::program::*;

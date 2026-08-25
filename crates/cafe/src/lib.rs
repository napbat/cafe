//! Complete public entry point for Cafe's Java ecosystem tooling.
//!
//! Consumers need only this crate. Format-specific capabilities remain grouped
//! under [`java`], [`dex`], [`art`], and [`jni`], including ISA-specific JVM and
//! Dalvik LLIL; their shared typed semantic representation lives under [`mlil`];
//! unified cross-format hierarchy aggregation lives under [`classpath`]; Java
//! source recovery lives under [`decompiler`]; shared
//! instruction and graph APIs, including exact and conservatively recovered
//! exception structure, live under [`disassembler`]; and the owned definition
//! model is available both at this crate's root and under [`program`].
//!
//! ```
//! use cafe::{Program, art, cfglib, classpath, decompiler, dex, disassembler, java, jni, mlil, program};
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
//! let _ = std::any::type_name::<classpath::ClasspathHierarchy>();
//! let _ = std::any::type_name::<decompiler::DecompiledClass>();
//! let _ = std::any::type_name::<cfglib::BlockId>();
//! let _ = std::any::type_name::<mlil::Function>();
//! ```

/// Android runtime VDEX, ODEX, OAT, and canonical dequickening support.
pub use ::art;
/// Unified JVM/DEX classpath declarations and native hierarchy views.
pub use ::classpath;
/// Verified MLIL-backed JVM class-file to Java source decompilation.
pub use ::decompiler;
/// Android DEX/CompactDex, Dalvik instructions and LLIL, APK/AAB, and adapters.
pub use ::dex;
/// Shared Java-ecosystem disassembly IR and control-flow graphs.
pub use ::disassembler;
/// JVM class files, bytecode and LLIL, JAR/JMOD/JIMAGE, corpus, and adapters.
pub use ::java;
/// Typed Java Native Interface declarations, symbols, and artifact adapters.
pub use ::jni;
/// Shared typed semantic IL, verification, provenance, dominance, and SSA views.
pub use ::mlil;
/// Owned modules, definitions, identities, and cross-module resolution.
pub use ::program;

/// Graph algorithms and data structures used by shared control-flow graphs.
pub use ::disassembler::cfglib;
pub use ::program::*;

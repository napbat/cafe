//! Java source recovery from verified semantic MLIL.
//!
//! Class-file declarations remain owned by [`java`], while this focused layer
//! lifts executable methods to [`mlil`], applies reusable cfglib analyses, and
//! renders Java source. Reducible control flow is structured when safe. Exact
//! exception dispatch and irreducible graphs use a Java-valid state machine;
//! unsupported semantics produce an explicit diagnostic and throwing body
//! instead of guessed source.

mod class;
mod compilation_unit;
mod diagnostic;
mod environment;
mod error;
mod method;
mod model;
mod names;
mod options;
mod signature;
mod writer;

pub use self::class::{
    decompile_class, decompile_class_bytes, decompile_class_with_hierarchy,
    decompile_class_with_options,
};
pub use self::compilation_unit::{
    decompile_compilation_unit, decompile_compilation_unit_with_environment,
    decompile_compilation_unit_with_hierarchy, decompile_compilation_unit_with_options,
};
pub use self::diagnostic::{Diagnostic, DiagnosticCode, DiagnosticSeverity, MethodIdentity};
pub use self::environment::MethodExceptionCatalog;
pub use self::error::{Error, Result};
pub use self::method::decompile_function;
pub use self::model::{DecompiledBody, DecompiledClass, GeneratedSpan, SourceMapEntry};
pub use self::names::compilation_unit_path;
pub use self::options::{
    ControlFlowPreference, DecompilerOptions, DecompilerPass, DecompilerPasses, SourceMapPolicy,
};

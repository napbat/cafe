//! Java source recovery from verified semantic MLIL.
//!
//! Class-file declarations remain owned by [`java`], while this focused layer
//! lifts executable methods to [`mlil`], applies reusable cfglib analyses, and
//! renders Java source. Reducible control flow is structured when safe. Exact
//! exception dispatch and irreducible graphs use a Java-valid state machine;
//! unsupported semantics produce an explicit diagnostic and throwing body
//! instead of guessed source.

mod class;
mod diagnostic;
mod error;
mod method;
mod model;
mod names;
mod options;
mod writer;

pub use self::class::{
    decompile_class, decompile_class_bytes, decompile_class_with_hierarchy,
    decompile_class_with_options,
};
pub use self::diagnostic::{Diagnostic, DiagnosticCode, DiagnosticSeverity, MethodIdentity};
pub use self::error::{Error, Result};
pub use self::method::decompile_function;
pub use self::model::{DecompiledBody, DecompiledClass, GeneratedSpan, SourceMapEntry};
pub use self::names::compilation_unit_path;
pub use self::options::{ControlFlowPreference, DecompilerOptions};

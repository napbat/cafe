//! Classpath construction failures.

use disassembler::BinaryFormat;
use thiserror::Error;

use crate::DirectParents;

/// Result type used by unified classpath operations.
pub type Result<T> = std::result::Result<T, Error>;

/// Failure to normalize or merge classpath declarations.
#[derive(Debug, Error)]
pub enum Error {
    /// A JVM class or container could not be read.
    #[error(transparent)]
    Java(#[from] java::Error),
    /// A DEX file or Android container could not be read.
    #[error(transparent)]
    Dex(#[from] dex::Error),
    /// A format-native type name is not an object-class name.
    #[error("invalid {format} class name `{name}`")]
    InvalidClassName {
        /// Source format whose naming rules were applied.
        format: BinaryFormat,
        /// Rejected native name.
        name: String,
    },
    /// Equivalent canonical names carry incompatible direct parents.
    #[error("conflicting classpath declarations for `{descriptor}`")]
    ConflictingDeclaration {
        /// Canonical DEX-style object descriptor.
        descriptor: String,
        /// Parents already registered for this type.
        existing: DirectParents,
        /// Parents declared by the incoming artifact.
        incoming: DirectParents,
    },
}

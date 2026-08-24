//! Crate-level error and result types.

use thiserror::Error;

use crate::descriptor::DescriptorError;

/// Error produced while modeling a JNI declaration.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum Error {
    /// A JVM method descriptor is malformed.
    #[error(transparent)]
    Descriptor(#[from] DescriptorError),
}

/// Result alias used by JNI operations.
pub type Result<T> = std::result::Result<T, Error>;

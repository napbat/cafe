//! Typed ART container and dequickening errors.

/// Result type returned by this crate.
pub type Result<T> = std::result::Result<T, Error>;

/// ART parsing, canonicalization, and adapter failures.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The input does not begin with a supported ART container signature.
    #[error("unrecognized Android runtime container")]
    UnrecognizedContainer,

    /// A runtime container is truncated or violates its declared layout.
    #[error("invalid {format} at byte {offset}: {message}")]
    InvalidContainer {
        /// Container label used by the diagnostic.
        format: &'static str,
        /// Absolute byte offset of the malformed structure.
        offset: usize,
        /// Contextual explanation of the violated constraint.
        message: String,
    },

    /// The signature is known but the version-specific layout is unsupported.
    #[error("unsupported {format} version `{version}`")]
    UnsupportedVersion {
        /// Runtime container kind.
        format: &'static str,
        /// Exact printable version bytes.
        version: String,
    },

    /// Quickening information is malformed or does not match the code stream.
    #[error("invalid quickening data at byte {offset}: {message}")]
    InvalidQuickening {
        /// Byte or code-unit coordinate described by `message`.
        offset: usize,
        /// Contextual explanation of the malformed mapping.
        message: String,
    },

    /// Canonical restoration needs metadata not contained in this artifact.
    #[error("cannot canonicalize {artifact}: {message}")]
    MissingCanonicalizationMetadata {
        /// Artifact or member being restored.
        artifact: String,
        /// Required external metadata.
        message: String,
    },

    /// An embedded standard DEX file is malformed.
    #[error(transparent)]
    Dex(#[from] dex::Error),

    /// Shared program lifting failed.
    #[error(transparent)]
    Program(#[from] program::Error),
}

impl Error {
    pub(crate) fn invalid(format: &'static str, offset: usize, message: impl Into<String>) -> Self {
        Self::InvalidContainer {
            format,
            offset,
            message: message.into(),
        }
    }

    pub(crate) fn quickening(offset: usize, message: impl Into<String>) -> Self {
        Self::InvalidQuickening {
            offset,
            message: message.into(),
        }
    }
}

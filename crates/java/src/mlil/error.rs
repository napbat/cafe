//! JVM-to-MLIL adapter errors.

/// Error produced while lifting verified JVM LLIL into MLIL.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// JVM parsing, resolution, LLIL, or frame analysis failed.
    #[error(transparent)]
    Java(#[from] crate::Error),
    /// Shared MLIL construction or verification failed.
    #[error(transparent)]
    Mlil(#[from] ::mlil::Error),
    /// A JVM feature cannot be represented by the current semantic adapter.
    #[error("unsupported JVM MLIL feature at bytecode offset {offset}: {feature}")]
    Unsupported {
        /// Native bytecode offset.
        offset: usize,
        /// Precise unsupported feature.
        feature: String,
    },
    /// An analyzed control-flow target has no corresponding LLIL block.
    #[error("JVM MLIL target {target} from bytecode offset {source_offset} is missing")]
    MissingTarget {
        /// Source bytecode offset.
        source_offset: usize,
        /// Missing target bytecode offset.
        target: usize,
    },
}

impl Error {
    pub(super) fn unsupported(offset: usize, feature: impl Into<String>) -> Self {
        Self::Unsupported {
            offset,
            feature: feature.into(),
        }
    }
}

/// Result type returned by JVM MLIL adapters.
pub type Result<T> = std::result::Result<T, Error>;

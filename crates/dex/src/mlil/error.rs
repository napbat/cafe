//! Dalvik-to-MLIL adapter errors.

/// Error produced while lifting verified Dalvik LLIL into MLIL.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// DEX resolution, LLIL, body, or register analysis failed.
    #[error(transparent)]
    Dex(#[from] crate::Error),
    /// Shared MLIL construction or verification failed.
    #[error(transparent)]
    Mlil(#[from] ::mlil::Error),
    /// A valid native relationship cannot be represented by this adapter.
    #[error("unsupported Dalvik MLIL feature at code-unit offset {offset}: {feature}")]
    Unsupported {
        /// Native code-unit offset.
        offset: u32,
        /// Precise unsupported feature.
        feature: String,
    },
    /// A control-flow or payload target has no corresponding native item.
    #[error("Dalvik MLIL target {target} from code-unit offset {source_offset} is missing")]
    MissingTarget {
        /// Source code-unit offset.
        source_offset: u32,
        /// Missing target code-unit offset.
        target: u32,
    },
}

impl Error {
    pub(super) fn unsupported(offset: u32, feature: impl Into<String>) -> Self {
        Self::Unsupported {
            offset,
            feature: feature.into(),
        }
    }
}

/// Result type returned by Dalvik MLIL adapters.
pub type Result<T> = std::result::Result<T, Error>;

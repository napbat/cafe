//! Dalvik/MLIL adapter errors.

/// Error produced while lifting or lowering between Dalvik LLIL and MLIL.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// DEX resolution, LLIL, body, or register analysis failed.
    #[error(transparent)]
    Dex(#[from] crate::Error),
    /// Shared MLIL construction or verification failed.
    #[error(transparent)]
    Mlil(#[from] ::mlil::Error),
    /// Source-index reuse was requested for non-DEX provenance.
    #[error("cannot reuse DEX source-table indices from {actual} MLIL")]
    WrongFormat {
        /// Source bytecode family recorded by MLIL.
        actual: disassembler::BinaryFormat,
    },
    /// A symbolic MLIL reference cannot be represented in the target DEX tables.
    #[error("cannot lower MLIL instruction {instruction}: {source}")]
    Reference {
        /// Stable semantic instruction identity.
        instruction: ::mlil::InstructionId,
        /// Reference-resolution explanation.
        #[source]
        source: crate::DexReferenceResolutionError,
    },
    /// A verified semantic construct has no valid canonical Dalvik encoding.
    #[error("cannot lower MLIL instruction {instruction}: {message}")]
    Lowering {
        /// Stable semantic instruction identity.
        instruction: ::mlil::InstructionId,
        /// Violated target constraint.
        message: String,
    },
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

    pub(super) fn lowering(instruction: ::mlil::InstructionId, message: impl Into<String>) -> Self {
        Self::Lowering {
            instruction,
            message: message.into(),
        }
    }
}

/// Result type returned by Dalvik MLIL adapters.
pub type Result<T> = std::result::Result<T, Error>;

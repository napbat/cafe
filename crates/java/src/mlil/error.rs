//! JVM/MLIL adapter errors.

/// Error produced while lifting or lowering between JVM LLIL and MLIL.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// JVM parsing, resolution, LLIL, or frame analysis failed.
    #[error(transparent)]
    Java(#[from] crate::Error),
    /// Shared MLIL construction or verification failed.
    #[error(transparent)]
    Mlil(#[from] ::mlil::Error),
    /// Source-index reuse was requested for non-JVM provenance.
    #[error("cannot reuse JVM source-pool indices from {actual} MLIL")]
    WrongFormat {
        /// Source bytecode family recorded by MLIL.
        actual: disassembler::BinaryFormat,
    },
    /// A symbolic MLIL reference cannot be represented in the target pool.
    #[error("cannot lower MLIL instruction {instruction}: {source}")]
    Reference {
        /// Stable semantic instruction identity.
        instruction: ::mlil::InstructionId,
        /// Reference-resolution explanation.
        #[source]
        source: crate::JavaReferenceResolutionError,
    },
    /// A verified semantic construct has no valid canonical JVM encoding.
    #[error("cannot lower MLIL instruction {instruction}: {message}")]
    Lowering {
        /// Stable semantic instruction identity.
        instruction: ::mlil::InstructionId,
        /// Violated target constraint.
        message: String,
    },
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

    pub(super) fn lowering(instruction: ::mlil::InstructionId, message: impl Into<String>) -> Self {
        Self::Lowering {
            instruction,
            message: message.into(),
        }
    }
}

/// Result type returned by JVM MLIL adapters.
pub type Result<T> = std::result::Result<T, Error>;

//! Errors produced by DEX and Android-container operations.

use std::io;

/// Result type returned by this crate.
pub type Result<T> = std::result::Result<T, Error>;

/// DEX parsing, assembly, lowering, and APK errors.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// An operating-system I/O operation failed.
    #[error(transparent)]
    Io(#[from] io::Error),

    /// A ZIP or APK operation failed.
    #[error(transparent)]
    Zip(#[from] zip::result::ZipError),

    /// A DEX structure is truncated or violates the format constraints.
    #[error("invalid DEX at byte {offset}: {message}")]
    InvalidDex {
        /// Absolute byte offset of the malformed structure.
        offset: usize,
        /// Contextual explanation of the violated constraint.
        message: String,
    },

    /// A Dalvik instruction stream is malformed.
    #[error("invalid DEX bytecode at code-unit {offset}: {message}")]
    InvalidInstruction {
        /// Code-unit offset of the malformed instruction.
        offset: u32,
        /// Contextual explanation of the malformed encoding.
        message: String,
    },

    /// An edited DEX model cannot be represented in binary form.
    #[error("cannot assemble DEX data: {0}")]
    InvalidAssembly(String),

    /// A DEX method-scoped error with its native identity retained.
    #[error("in DEX class `{class}`, method `{method}{signature}`: {source}")]
    Method {
        /// Declaring type descriptor.
        class: String,
        /// Method name.
        method: String,
        /// DEX method descriptor.
        signature: String,
        /// Underlying instruction, graph, or model error.
        #[source]
        source: Box<Error>,
    },

    /// An APK entry-scoped DEX error.
    #[error("in APK entry `{entry}`: {source}")]
    ApkEntry {
        /// Exact archive-relative entry name.
        entry: String,
        /// Underlying archive or DEX error.
        #[source]
        source: Box<Error>,
    },

    /// Shared disassembly could not be represented as a valid graph.
    #[error(transparent)]
    Graph(#[from] disassembler::GraphError),

    /// DEX metadata could not be represented in Cafe's owned model.
    #[error(transparent)]
    Cafe(#[from] cafe::Error),

    /// Saving would silently invalidate an APK signature.
    #[error("refusing to rewrite a signed APK without an explicit signature policy")]
    SignedApkMutation,
}

impl Error {
    pub(crate) fn invalid_dex(offset: usize, message: impl Into<String>) -> Self {
        Self::InvalidDex {
            offset,
            message: message.into(),
        }
    }

    pub(crate) fn invalid_instruction(offset: u32, message: impl Into<String>) -> Self {
        Self::InvalidInstruction {
            offset,
            message: message.into(),
        }
    }

    pub(crate) fn invalid_assembly(message: impl Into<String>) -> Self {
        Self::InvalidAssembly(message.into())
    }

    pub(crate) fn in_method(
        self,
        class: impl Into<String>,
        method: impl Into<String>,
        signature: impl Into<String>,
    ) -> Self {
        Self::Method {
            class: class.into(),
            method: method.into(),
            signature: signature.into(),
            source: Box::new(self),
        }
    }
}

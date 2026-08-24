//! Errors produced by DEX and Android-container operations.

use std::fmt;
use std::io;

/// Result type returned by this crate.
pub type Result<T> = std::result::Result<T, Error>;

/// DEX identifier table named by an index-resolution error.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IdentifierTable {
    /// String identifier table.
    String,
    /// Type identifier table.
    Type,
    /// Prototype identifier table.
    Prototype,
    /// Field identifier table.
    Field,
    /// Method identifier table.
    Method,
    /// Call-site identifier table.
    CallSite,
    /// Method-handle table.
    MethodHandle,
}

impl fmt::Display for IdentifierTable {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::String => "string",
            Self::Type => "type",
            Self::Prototype => "prototype",
            Self::Field => "field",
            Self::Method => "method",
            Self::CallSite => "call-site",
            Self::MethodHandle => "method-handle",
        })
    }
}

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

    /// An APK or its ZIP container is structurally invalid.
    #[error("invalid APK: {0}")]
    InvalidApk(String),

    /// An APK entry name is unsafe or inconsistent with its entry kind.
    #[error("invalid APK entry name `{name}`: {message}")]
    InvalidApkEntryName {
        /// Rejected archive-relative name.
        name: String,
        /// Explanation of the violated path rule.
        message: String,
    },

    /// No APK entry has the requested name.
    #[error("APK entry `{0}` was not found")]
    ApkEntryNotFound(String),

    /// A name-based operation found more than one matching APK entry.
    #[error("APK entry name `{name}` is ambiguous ({count} entries)")]
    AmbiguousApkEntry {
        /// Duplicate entry name.
        name: String,
        /// Number of entries with that name.
        count: usize,
    },

    /// A stable APK entry identifier is no longer present.
    #[error("APK entry id {0} was not found")]
    ApkEntryIdNotFound(u64),

    /// A mutation would introduce a duplicate APK entry name.
    #[error("APK already contains an entry named `{0}`")]
    DuplicateApkEntry(String),

    /// An APK entry cannot be rewritten by the configured ZIP codecs.
    #[error("cannot rewrite APK entry `{entry}`: {message}")]
    UnsupportedApkEntry {
        /// Entry that cannot be rewritten.
        entry: String,
        /// Unsupported feature or encoding.
        message: String,
    },

    /// An editable model contains an index outside its identifier table.
    #[error("{table} identifier index {index} is out of bounds")]
    InvalidIndex {
        /// Identifier table selected by the reference.
        table: IdentifierTable,
        /// Invalid native index.
        index: u32,
    },

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

    /// DEX metadata could not be represented in the shared program model.
    #[error(transparent)]
    Program(#[from] program::Error),

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

    pub(crate) fn invalid_apk(message: impl Into<String>) -> Self {
        Self::InvalidApk(message.into())
    }

    pub(crate) fn invalid_apk_entry_name(
        name: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self::InvalidApkEntryName {
            name: name.into(),
            message: message.into(),
        }
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

    pub(crate) fn in_apk_entry(self, entry: impl Into<String>) -> Self {
        Self::ApkEntry {
            entry: entry.into(),
            source: Box::new(self),
        }
    }
}

use std::io;

/// The result type returned by this crate.
pub type Result<T> = std::result::Result<T, Error>;

/// Errors produced while reading archives, class files, or bytecode.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// An operating-system I/O operation failed.
    #[error(transparent)]
    Io(#[from] io::Error),

    /// A ZIP/JAR operation failed.
    #[error(transparent)]
    Zip(#[from] zip::result::ZipError),

    /// A class file is truncated or structurally invalid.
    #[error("invalid class file at byte {offset}: {message}")]
    InvalidClass {
        /// Byte offset at which the problem was found.
        offset: usize,
        /// Human-readable explanation of the problem.
        message: String,
    },

    /// A method contains malformed bytecode.
    #[error("invalid bytecode at offset {offset}: {message}")]
    InvalidBytecode {
        /// Bytecode offset at which the problem was found.
        offset: usize,
        /// Human-readable explanation of the problem.
        message: String,
    },

    /// A field or method descriptor is malformed.
    #[error("invalid JVM descriptor at byte {offset}: {message}")]
    InvalidDescriptor {
        /// Byte offset within the descriptor.
        offset: usize,
        /// Human-readable explanation of the problem.
        message: String,
    },

    /// A structured class or instruction sequence cannot be encoded.
    #[error("cannot assemble JVM data: {0}")]
    InvalidAssembly(String),

    /// A class-file error scoped to one overload-qualified method.
    #[error("in class `{class}`, method `{method}{descriptor}`: {source}")]
    ClassMethod {
        /// Internal JVM class name or an index placeholder if unresolved.
        class: String,
        /// JVM method name or an index placeholder if unresolved.
        method: String,
        /// JVM method descriptor, or an empty string if unresolved.
        descriptor: String,
        /// Underlying structural, descriptor, bytecode, or assembly error.
        #[source]
        source: Box<Error>,
    },

    /// Shared disassembly IR could not be converted into a valid control-flow graph.
    #[error(transparent)]
    DisassemblyGraph(#[from] disassembler::GraphError),

    /// JVM metadata could not be represented in the shared program model.
    #[error(transparent)]
    Program(#[from] program::Error),

    /// A requested class is not present in the archive.
    #[error("class `{0}` was not found in the JAR")]
    ClassNotFound(String),

    /// A requested method was not declared by the selected class.
    #[error("method `{method}`{descriptor} was not found in class `{class}`")]
    MethodNotFound {
        /// Internal class name.
        class: String,
        /// Requested method name.
        method: String,
        /// Optional descriptor rendered for the error message.
        descriptor: String,
    },

    /// A particular JAR entry could not be parsed.
    #[error("failed to parse JAR entry `{entry}`: {source}")]
    JarEntry {
        /// Name of the failing JAR entry.
        entry: String,
        /// Underlying class-file or bytecode error.
        #[source]
        source: Box<Error>,
    },

    /// A JAR entry name is unsafe or does not match its entry kind.
    #[error("invalid JAR entry name `{name}`: {message}")]
    InvalidJarEntryName {
        /// Rejected archive-relative name.
        name: String,
        /// Explanation of the violated JAR path rule.
        message: String,
    },

    /// No JAR entry has the requested name.
    #[error("JAR entry `{0}` was not found")]
    JarEntryNotFound(String),

    /// A name-based operation found more than one matching JAR entry.
    #[error("JAR entry name `{name}` is ambiguous ({count} entries)")]
    AmbiguousJarEntry {
        /// Duplicate entry name.
        name: String,
        /// Number of entries with that name.
        count: usize,
    },

    /// A stable JAR entry identifier is no longer present.
    #[error("JAR entry id {0} was not found")]
    JarEntryIdNotFound(u64),

    /// A mutation would introduce a duplicate archive name.
    #[error("JAR already contains an entry named `{0}`")]
    DuplicateJarEntry(String),

    /// JAR metadata or a manifest is malformed.
    #[error("invalid JAR metadata: {0}")]
    InvalidJar(String),

    /// A JMOD header or section path is malformed.
    #[error("invalid JMOD at byte {offset}: {message}")]
    InvalidJmod {
        /// Byte offset at which the problem was detected.
        offset: usize,
        /// Human-readable explanation of the violated container contract.
        message: String,
    },

    /// A JIMAGE header, index, location, or resource is malformed.
    #[error("invalid JIMAGE at byte {offset}: {message}")]
    InvalidJimage {
        /// Byte offset at which the problem was detected.
        offset: usize,
        /// Human-readable explanation of the violated image contract.
        message: String,
    },

    /// A compressed JIMAGE resource names an unknown decompressor.
    #[error("JIMAGE resource `{entry}` uses unsupported decompressor `{decompressor}`")]
    UnsupportedJimageCompression {
        /// Fully qualified JIMAGE resource name.
        entry: String,
        /// Name stored in the JIMAGE string table.
        decompressor: String,
    },

    /// No JIMAGE resource has the requested fully qualified name.
    #[error("JIMAGE resource `{0}` was not found")]
    JimageEntryNotFound(String),

    /// A JAR entry cannot be rewritten with the configured ZIP codecs.
    #[error("cannot rewrite JAR entry `{entry}`: {message}")]
    UnsupportedJarEntry {
        /// Entry that cannot be rewritten.
        entry: String,
        /// Unsupported feature or encoding.
        message: String,
    },

    /// Saving would silently invalidate existing JAR signatures.
    #[error("refusing to rewrite a signed JAR without an explicit signature policy")]
    SignedJarMutation,
}

impl Error {
    pub(crate) fn invalid_class(offset: usize, message: impl Into<String>) -> Self {
        Self::InvalidClass {
            offset,
            message: message.into(),
        }
    }

    pub(crate) fn invalid_bytecode(offset: usize, message: impl Into<String>) -> Self {
        Self::InvalidBytecode {
            offset,
            message: message.into(),
        }
    }

    pub(crate) fn invalid_descriptor(offset: usize, message: impl Into<String>) -> Self {
        Self::InvalidDescriptor {
            offset,
            message: message.into(),
        }
    }

    pub(crate) fn invalid_assembly(message: impl Into<String>) -> Self {
        Self::InvalidAssembly(message.into())
    }

    /// Qualifies this error with an exact physical archive entry name.
    #[must_use]
    pub fn in_jar_entry(self, entry: impl Into<String>) -> Self {
        Self::JarEntry {
            entry: entry.into(),
            source: Box::new(self),
        }
    }

    pub(crate) fn in_class_method(
        self,
        class: impl Into<String>,
        method: impl Into<String>,
        descriptor: impl Into<String>,
    ) -> Self {
        Self::ClassMethod {
            class: class.into(),
            method: method.into(),
            descriptor: descriptor.into(),
            source: Box::new(self),
        }
    }

    pub(crate) fn invalid_jar_entry_name(
        name: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self::InvalidJarEntryName {
            name: name.into(),
            message: message.into(),
        }
    }

    pub(crate) fn invalid_jmod(offset: usize, message: impl Into<String>) -> Self {
        Self::InvalidJmod {
            offset,
            message: message.into(),
        }
    }

    pub(crate) fn invalid_jimage(offset: usize, message: impl Into<String>) -> Self {
        Self::InvalidJimage {
            offset,
            message: message.into(),
        }
    }
}

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

    /// Shared disassembly IR could not be converted into a valid control-flow graph.
    #[error(transparent)]
    DisassemblyGraph(#[from] disassembler::GraphError),

    /// JVM metadata could not be represented in the shared Cafe model.
    #[error(transparent)]
    Cafe(#[from] cafe::Error),

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

    pub(crate) fn in_jar_entry(self, entry: impl Into<String>) -> Self {
        Self::JarEntry {
            entry: entry.into(),
            source: Box::new(self),
        }
    }
}

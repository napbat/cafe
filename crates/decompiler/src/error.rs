//! Fatal decompilation failures.

/// Result returned by Java source decompilation.
pub type Result<T> = std::result::Result<T, Error>;

/// Fatal failure that prevents producing a class declaration.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Invalid JVM metadata, descriptor, or class-file structure.
    #[error(transparent)]
    Java(#[from] java::Error),

    /// The artifact has no Java class-declaration representation.
    #[error("cannot decompile artifact as a Java class: {0}")]
    UnsupportedArtifact(String),
}

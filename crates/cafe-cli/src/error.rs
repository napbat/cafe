//! Fatal command failures.

use std::io;
use std::path::PathBuf;

pub(crate) type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, thiserror::Error)]
pub(crate) enum Error {
    #[error("failed to open JAR `{path}`: {source}")]
    OpenJar {
        path: PathBuf,
        #[source]
        source: cafe::java::Error,
    },

    #[error("failed to select the effective view of JAR `{path}`: {source}")]
    SelectJarView {
        path: PathBuf,
        #[source]
        source: cafe::java::Error,
    },

    #[error("failed to create output directory `{path}`: {source}")]
    CreateOutputDirectory {
        path: PathBuf,
        #[source]
        source: io::Error,
    },

    #[error("failed to resolve output directory `{path}`: {source}")]
    ResolveOutputDirectory {
        path: PathBuf,
        #[source]
        source: io::Error,
    },

    #[error("output path `{0}` is not a directory")]
    OutputNotDirectory(PathBuf),

    #[error("refusing unsafe output path `{path}` for class `{class_name}`")]
    UnsafeOutputPath { class_name: String, path: PathBuf },

    #[error("failed to create source directory `{path}`: {source}")]
    CreateSourceDirectory {
        path: PathBuf,
        #[source]
        source: io::Error,
    },

    #[error("failed to inspect source path `{path}`: {source}")]
    InspectSource {
        path: PathBuf,
        #[source]
        source: io::Error,
    },

    #[error("source file `{0}` already exists; pass `--force` to overwrite it")]
    SourceExists(PathBuf),

    #[error("refusing to write through symbolic link `{0}`")]
    SourceIsSymlink(PathBuf),

    #[error("source path `{0}` is a directory")]
    SourceIsDirectory(PathBuf),

    #[error("failed to write source file `{path}`: {source}")]
    WriteSource {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
}

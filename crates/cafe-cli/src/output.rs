//! Safe package-qualified source output.

use std::ffi::OsStr;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Component, Path, PathBuf};

use crate::error::{Error, Result};

#[cfg(windows)]
pub(crate) fn collision_key(path: &Path) -> String {
    path.as_os_str().to_string_lossy().to_lowercase()
}

#[cfg(not(windows))]
pub(crate) fn collision_key(path: &Path) -> PathBuf {
    path.to_path_buf()
}

pub(crate) fn prepare_directory(path: &Path) -> Result<PathBuf> {
    fs::create_dir_all(path).map_err(|source| Error::CreateOutputDirectory {
        path: path.to_path_buf(),
        source,
    })?;
    let metadata = fs::metadata(path).map_err(|source| Error::ResolveOutputDirectory {
        path: path.to_path_buf(),
        source,
    })?;
    if !metadata.is_dir() {
        return Err(Error::OutputNotDirectory(path.to_path_buf()));
    }
    fs::canonicalize(path).map_err(|source| Error::ResolveOutputDirectory {
        path: path.to_path_buf(),
        source,
    })
}

pub(crate) fn destination_path(root: &Path, class_name: &str, relative: &str) -> Result<PathBuf> {
    let relative_path = Path::new(relative);
    if relative_path.as_os_str().is_empty()
        || relative_path.is_absolute()
        || relative_path.components().any(|component| {
            !matches!(component, Component::Normal(value) if value != OsStr::new(".") && value != OsStr::new(".."))
        })
    {
        return Err(Error::UnsafeOutputPath {
            class_name: class_name.to_owned(),
            path: relative_path.to_path_buf(),
        });
    }
    let destination = root.join(relative_path);
    let parent = destination
        .parent()
        .ok_or_else(|| Error::UnsafeOutputPath {
            class_name: class_name.to_owned(),
            path: relative_path.to_path_buf(),
        })?;
    fs::create_dir_all(parent).map_err(|source| Error::CreateSourceDirectory {
        path: parent.to_path_buf(),
        source,
    })?;
    let canonical_parent =
        fs::canonicalize(parent).map_err(|source| Error::ResolveOutputDirectory {
            path: parent.to_path_buf(),
            source,
        })?;
    if !canonical_parent.starts_with(root) {
        return Err(Error::UnsafeOutputPath {
            class_name: class_name.to_owned(),
            path: destination,
        });
    }
    Ok(canonical_parent.join(
        destination
            .file_name()
            .expect("validated relative paths have a final component"),
    ))
}

pub(crate) fn write_source(path: &Path, source: &str, force: bool) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            return Err(Error::SourceIsSymlink(path.to_path_buf()));
        }
        Ok(metadata) if metadata.is_dir() => {
            return Err(Error::SourceIsDirectory(path.to_path_buf()));
        }
        Ok(_) if !force => return Err(Error::SourceExists(path.to_path_buf())),
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(source) => {
            return Err(Error::InspectSource {
                path: path.to_path_buf(),
                source,
            });
        }
    }
    let mut options = OpenOptions::new();
    options.write(true);
    if force {
        options.create(true).truncate(true);
    } else {
        options.create_new(true);
    }
    let mut file = options.open(path).map_err(|source| Error::WriteSource {
        path: path.to_path_buf(),
        source,
    })?;
    file.write_all(source.as_bytes())
        .map_err(|source| Error::WriteSource {
            path: path.to_path_buf(),
            source,
        })
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::destination_path;

    #[test]
    fn destination_paths_cannot_escape_the_output_root() {
        let root = fs::canonicalize(std::env::temp_dir()).expect("resolve temporary directory");
        assert!(destination_path(&root, "sample/Escape", "../Escape.java").is_err());
        assert!(destination_path(&root, "sample/Escape", "/Escape.java").is_err());
    }
}

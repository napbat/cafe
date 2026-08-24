//! Deterministic filesystem discovery for APK archives.

use std::fs;
use std::path::{Path, PathBuf};

use crate::Result;

/// Conventional file extension for an Android application package.
pub const APK_EXTENSION: &str = "apk";

/// Controls filesystem traversal for APK discovery.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Traversal {
    /// Inspect only the supplied directory's direct children.
    #[default]
    Direct,
    /// Recursively inspect descendants without following symlinked directories.
    Recursive,
}

/// Discovers APK files beneath a path in deterministic order.
///
/// An APK path supplied directly is returned as a one-element list. Directory
/// traversal does not follow symlinked directories.
///
/// # Errors
///
/// Returns an error when filesystem metadata or a directory cannot be read.
pub fn discover_apks(root: impl AsRef<Path>, traversal: Traversal) -> Result<Vec<PathBuf>> {
    let root = root.as_ref();
    let metadata = fs::metadata(root)?;
    let mut apks = Vec::new();
    if metadata.is_file() {
        if is_apk_path(root) {
            apks.push(root.to_path_buf());
        }
    } else if metadata.is_dir() {
        discover_in_directory(root, traversal, &mut apks)?;
    }
    apks.sort();
    Ok(apks)
}

/// Returns whether a path has a case-insensitive APK extension.
#[must_use]
pub fn is_apk_path(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case(APK_EXTENSION))
}

fn discover_in_directory(
    directory: &Path,
    traversal: Traversal,
    apks: &mut Vec<PathBuf>,
) -> Result<()> {
    let mut entries = fs::read_dir(directory)?.collect::<std::result::Result<Vec<_>, _>>()?;
    entries.sort_by_key(std::fs::DirEntry::path);
    for entry in entries {
        let file_type = entry.file_type()?;
        let path = entry.path();
        if file_type.is_file() && is_apk_path(&path) {
            apks.push(path);
        } else if file_type.is_dir() && traversal == Traversal::Recursive {
            discover_in_directory(&path, traversal, apks)?;
        }
    }
    Ok(())
}

//! Deterministic filesystem discovery for Android App Bundles.

use std::fs;
use std::path::{Path, PathBuf};

use crate::Result;

/// Conventional file extension for an Android App Bundle.
pub const AAB_EXTENSION: &str = "aab";

/// Controls filesystem traversal for bundle discovery.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Traversal {
    /// Inspect only direct children.
    #[default]
    Direct,
    /// Recursively inspect descendants without following symlinked directories.
    Recursive,
}

/// Discovers App Bundles beneath a path in deterministic order.
///
/// # Errors
///
/// Returns an error when filesystem metadata or a directory cannot be read.
pub fn discover_aabs(root: impl AsRef<Path>, traversal: Traversal) -> Result<Vec<PathBuf>> {
    let root = root.as_ref();
    let metadata = fs::metadata(root)?;
    let mut output = Vec::new();
    if metadata.is_file() {
        if is_aab_path(root) {
            output.push(root.to_path_buf());
        }
    } else if metadata.is_dir() {
        discover_directory(root, traversal, &mut output)?;
    }
    output.sort();
    Ok(output)
}

/// Returns whether a path has a case-insensitive `.aab` extension.
#[must_use]
pub fn is_aab_path(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case(AAB_EXTENSION))
}

fn discover_directory(
    directory: &Path,
    traversal: Traversal,
    output: &mut Vec<PathBuf>,
) -> Result<()> {
    let mut entries = fs::read_dir(directory)?.collect::<std::result::Result<Vec<_>, _>>()?;
    entries.sort_by_key(std::fs::DirEntry::path);
    for entry in entries {
        let path = entry.path();
        let kind = entry.file_type()?;
        if kind.is_file() && is_aab_path(&path) {
            output.push(path);
        } else if kind.is_dir() && traversal == Traversal::Recursive {
            discover_directory(&path, traversal, output)?;
        }
    }
    Ok(())
}

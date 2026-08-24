//! Deterministic filesystem discovery for JAR archives.

use std::fs;
use std::path::{Path, PathBuf};

use crate::Result;

/// Conventional file extension for a Java archive.
pub const JAR_EXTENSION: &str = "jar";

/// Controls filesystem traversal for archive discovery.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Traversal {
    /// Inspect only the supplied directory's direct children.
    #[default]
    Direct,
    /// Recursively inspect descendant directories without following symlinks.
    Recursive,
}

/// Discovers JAR files beneath a filesystem path in deterministic order.
///
/// A JAR path supplied directly is returned as a one-element list. Directory
/// traversal does not follow symlinked directories.
///
/// # Errors
///
/// Returns an error if filesystem metadata or a traversed directory cannot be
/// read.
pub fn discover_jars(root: impl AsRef<Path>, traversal: Traversal) -> Result<Vec<PathBuf>> {
    let root = root.as_ref();
    let metadata = fs::metadata(root)?;
    let mut jars = Vec::new();
    if metadata.is_file() {
        if is_jar_path(root) {
            jars.push(root.to_path_buf());
        }
    } else if metadata.is_dir() {
        discover_in_directory(root, traversal, &mut jars)?;
    }
    jars.sort();
    Ok(jars)
}

/// Returns whether a filesystem path has a case-insensitive JAR extension.
#[must_use]
pub fn is_jar_path(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case(JAR_EXTENSION))
}

fn discover_in_directory(
    directory: &Path,
    traversal: Traversal,
    jars: &mut Vec<PathBuf>,
) -> Result<()> {
    let mut entries = fs::read_dir(directory)?.collect::<std::result::Result<Vec<_>, _>>()?;
    entries.sort_by_key(std::fs::DirEntry::path);
    for entry in entries {
        let file_type = entry.file_type()?;
        let path = entry.path();
        if file_type.is_file() && is_jar_path(&path) {
            jars.push(path);
        } else if file_type.is_dir() && traversal == Traversal::Recursive {
            discover_in_directory(&path, traversal, jars)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::{Traversal, discover_jars, is_jar_path};

    #[test]
    fn recognizes_jar_extensions_without_case_sensitivity() {
        assert!(is_jar_path(Path::new("library.jar")));
        assert!(is_jar_path(Path::new("library.JAR")));
        assert!(!is_jar_path(Path::new("library.zip")));
    }

    #[test]
    fn discovers_jars_with_typed_traversal() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock is after the Unix epoch")
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "java-jar-discovery-{}-{unique}",
            std::process::id()
        ));
        let nested = root.join("nested");
        fs::create_dir_all(&nested).expect("create test directories");
        fs::write(root.join("one.jar"), []).expect("create direct JAR");
        fs::write(root.join("ignored.txt"), []).expect("create non-JAR");
        fs::write(nested.join("two.JAR"), []).expect("create nested JAR");

        let direct = discover_jars(&root, Traversal::Direct).expect("direct discovery");
        assert_eq!(direct, [root.join("one.jar")]);
        let recursive = discover_jars(&root, Traversal::Recursive).expect("recursive discovery");
        assert_eq!(
            recursive,
            [root.join("nested").join("two.JAR"), root.join("one.jar")]
        );

        fs::remove_dir_all(&root).expect("remove test directory");
    }
}

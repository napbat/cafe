//! Repository-level architectural constraints.

use std::fs;
use std::path::{Path, PathBuf};

const MAX_SOURCE_LINES: usize = 1_000;
const SOURCE_EXTENSIONS: &[&str] = &[
    "c", "cc", "cpp", "cts", "cxx", "h", "hh", "hpp", "hxx", "java", "js", "jsx", "lua", "mjs",
    "mts", "ps1", "rs", "sh", "ts", "tsx",
];
const EXCLUDED_DIRECTORY_NAMES: &[&str] = &[
    ".generated",
    ".git",
    ".stage",
    "dist",
    "node_modules",
    "target",
];

#[test]
fn source_files_do_not_exceed_line_limit() {
    let package_root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let workspace_root = package_root
        .parent()
        .and_then(Path::parent)
        .filter(|path| path.is_dir())
        .unwrap_or(package_root);
    let mut files = Vec::new();
    collect_source_files(workspace_root, &mut files);
    files.sort();

    let oversized = files
        .into_iter()
        .filter_map(|path| {
            let source = fs::read_to_string(&path).expect("read Rust source");
            let line_count = source.lines().count();
            (line_count > MAX_SOURCE_LINES).then_some((path, line_count))
        })
        .collect::<Vec<_>>();

    assert!(
        oversized.is_empty(),
        "source files exceed the {MAX_SOURCE_LINES}-line limit: {oversized:#?}"
    );
}

fn collect_source_files(directory: &Path, files: &mut Vec<PathBuf>) {
    let mut entries = fs::read_dir(directory)
        .expect("read source directory")
        .collect::<std::io::Result<Vec<_>>>()
        .expect("read source entries");
    entries.sort_by_key(fs::DirEntry::path);

    for entry in entries {
        let path = entry.path();
        let file_type = entry.file_type().expect("read source entry type");
        if file_type.is_dir() {
            if !is_excluded_directory(&path) {
                collect_source_files(&path, files);
            }
        } else if file_type.is_file() && is_source_file(&path) {
            files.push(path);
        }
    }
}

fn is_excluded_directory(path: &Path) -> bool {
    path.file_name()
        .and_then(|value| value.to_str())
        .is_some_and(|name| EXCLUDED_DIRECTORY_NAMES.contains(&name))
}

fn is_source_file(path: &Path) -> bool {
    path.extension()
        .and_then(|value| value.to_str())
        .is_some_and(|extension| SOURCE_EXTENSIONS.contains(&extension))
}

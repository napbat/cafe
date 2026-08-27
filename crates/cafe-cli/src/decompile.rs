//! Whole-archive Java source decompilation.

use std::collections::{BTreeMap, BTreeSet};
use std::num::NonZeroUsize;
use std::panic::{self, AssertUnwindSafe};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;
use std::thread;

use cafe::classpath::ClasspathHierarchy;
use cafe::decompiler::{
    ControlFlowPreference, DecompiledClass, DecompilerOptions, Diagnostic, DiagnosticSeverity,
    compilation_unit_path, decompile_class_with_hierarchy, decompile_class_with_options,
};
use cafe::java::classfile::{ClassAccessFlags, ClassFile, MODULE_INFO_CLASS_NAME};
use cafe::java::jar::{CLASS_ENTRY_SUFFIX, ClassVisitControl, JarFile, parse_versioned_entry};

use crate::cli::JarCommand;
use crate::error::{Error, Result};
use crate::output;

const NEWEST_AVAILABLE_RELEASE: u16 = u16::MAX;
const MAX_AUTOMATIC_WORKERS: usize = 8;

#[derive(Debug)]
pub(crate) struct RunReport {
    pub(crate) output: PathBuf,
    pub(crate) selected: usize,
    pub(crate) written: usize,
    pub(crate) skipped: usize,
    pub(crate) diagnostics: Vec<ArchiveDiagnostic>,
    pub(crate) notices: Vec<ArchiveNotice>,
    pub(crate) failures: Vec<ClassFailure>,
}

impl RunReport {
    pub(crate) fn is_complete(&self) -> bool {
        self.failures.is_empty()
            && self
                .diagnostics
                .iter()
                .all(|item| item.diagnostic.severity != DiagnosticSeverity::Error)
    }
}

#[derive(Debug)]
pub(crate) struct ArchiveDiagnostic {
    pub(crate) entry: String,
    pub(crate) diagnostic: Diagnostic,
}

#[derive(Debug)]
pub(crate) struct ArchiveNotice {
    pub(crate) scope: String,
    pub(crate) message: String,
}

#[derive(Debug)]
pub(crate) struct ClassFailure {
    pub(crate) entry: String,
    pub(crate) stage: FailureStage,
    pub(crate) message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum FailureStage {
    Read,
    Parse,
    Metadata,
    Decompile,
    Output,
}

impl FailureStage {
    pub(crate) const fn name(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Parse => "parse",
            Self::Metadata => "metadata",
            Self::Decompile => "decompile",
            Self::Output => "output",
        }
    }
}

#[derive(Debug)]
struct SelectedClass {
    logical_name: String,
}

#[derive(Debug)]
struct LoadedClass {
    entry: String,
    internal_name: String,
    class: ClassFile,
}

#[derive(Debug)]
struct PreparedClass {
    entry: String,
    destination: PathBuf,
    class: ClassFile,
}

#[derive(Debug)]
struct WorkerResult {
    entry: String,
    destination: PathBuf,
    recovered: std::result::Result<DecompiledClass, String>,
}

pub(crate) fn jar(command: &JarCommand) -> Result<RunReport> {
    let archive = JarFile::open(&command.input).map_err(|source| Error::OpenJar {
        path: command.input.clone(),
        source,
    })?;
    let selected = selected_classes(&archive, command)?;
    let mut report = RunReport {
        output: command.output.clone(),
        selected: selected.len(),
        written: 0,
        skipped: archive.class_entry_count().saturating_sub(selected.len()),
        diagnostics: Vec::new(),
        notices: Vec::new(),
        failures: Vec::new(),
    };
    let mut classes = Vec::with_capacity(selected.len());

    archive.visit_class_bytes(
        |entry| selected.contains_key(entry.name),
        |entry, payload| {
            load_class(
                entry.name,
                selected
                    .get(entry.name)
                    .expect("visitor selected only effective entries"),
                payload,
                &mut classes,
                &mut report,
            );
            Ok::<ClassVisitControl, Error>(ClassVisitControl::Continue)
        },
    )?;

    let hierarchy = match ClasspathHierarchy::from_java_classes(
        classes.iter().map(|loaded| &loaded.class),
    ) {
        Ok(hierarchy) => Some(hierarchy),
        Err(error) => {
            report.notices.push(ArchiveNotice {
                scope: command.input.display().to_string(),
                message: format!(
                    "could not build one consistent JAR hierarchy; using conservative frame merges: {error}"
                ),
            });
            None
        }
    };
    let output_root = output::prepare_directory(&command.output)?;
    let options = DecompilerOptions {
        control_flow: if command.state_machine {
            ControlFlowPreference::StateMachine
        } else {
            ControlFlowPreference::StructuredWhenReducible
        },
        include_synthetic_members: !command.exclude_synthetic,
    };
    let mut destinations = BTreeMap::new();
    let mut prepared = Vec::with_capacity(classes.len());
    for loaded in classes {
        let destination = output::destination_path(
            &output_root,
            &loaded.internal_name,
            &compilation_unit_path(&loaded.internal_name),
        )?;
        if let Some(previous) =
            destinations.insert(output::collision_key(&destination), loaded.entry.clone())
        {
            report.failures.push(ClassFailure {
                entry: loaded.entry,
                stage: FailureStage::Output,
                message: format!(
                    "source path `{}` is already claimed by archive entry `{previous}`",
                    destination.display()
                ),
            });
            continue;
        }
        prepared.push(PreparedClass {
            entry: loaded.entry,
            destination,
            class: loaded.class,
        });
    }
    decompile_prepared(
        &prepared,
        hierarchy.as_ref(),
        &options,
        worker_count(command.jobs, prepared.len()),
        command.force,
        &mut report,
    )?;
    sort_report(&mut report);

    Ok(report)
}

fn decompile_prepared(
    classes: &[PreparedClass],
    hierarchy: Option<&ClasspathHierarchy>,
    options: &DecompilerOptions,
    workers: usize,
    force: bool,
    report: &mut RunReport,
) -> Result<()> {
    if classes.is_empty() {
        return Ok(());
    }
    let next_class = AtomicUsize::new(0);
    thread::scope(|scope| {
        let (sender, receiver) = mpsc::channel();
        for _ in 0..workers {
            let sender = sender.clone();
            let next_class = &next_class;
            scope.spawn(move || {
                loop {
                    let index = next_class.fetch_add(1, Ordering::Relaxed);
                    let Some(class) = classes.get(index) else {
                        break;
                    };
                    let recovered = recover_class(class, hierarchy, options);
                    if sender
                        .send(WorkerResult {
                            entry: class.entry.clone(),
                            destination: class.destination.clone(),
                            recovered,
                        })
                        .is_err()
                    {
                        break;
                    }
                }
            });
        }
        drop(sender);
        for result in receiver {
            match result.recovered {
                Ok(recovered) => {
                    output::write_source(&result.destination, &recovered.source, force)?;
                    record_diagnostics(report, &result.entry, recovered);
                    report.written += 1;
                }
                Err(message) => report.failures.push(ClassFailure {
                    entry: result.entry,
                    stage: FailureStage::Decompile,
                    message,
                }),
            }
        }
        Ok(())
    })
}

fn recover_class(
    class: &PreparedClass,
    hierarchy: Option<&ClasspathHierarchy>,
    options: &DecompilerOptions,
) -> std::result::Result<DecompiledClass, String> {
    match panic::catch_unwind(AssertUnwindSafe(|| match hierarchy {
        Some(hierarchy) => {
            let view = hierarchy.jvm_view();
            decompile_class_with_hierarchy(&class.class, &view, options)
        }
        None => decompile_class_with_options(&class.class, options),
    })) {
        Ok(result) => result.map_err(|error| error.to_string()),
        Err(payload) => Err(format!(
            "source recovery panicked: {}",
            panic_message(payload.as_ref())
        )),
    }
}

fn panic_message(payload: &(dyn std::any::Any + Send)) -> &str {
    payload
        .downcast_ref::<String>()
        .map(String::as_str)
        .or_else(|| payload.downcast_ref::<&str>().copied())
        .unwrap_or("non-string panic payload")
}

fn worker_count(requested: Option<NonZeroUsize>, classes: usize) -> usize {
    let requested = requested.map_or_else(
        || {
            thread::available_parallelism()
                .map_or(1, NonZeroUsize::get)
                .min(MAX_AUTOMATIC_WORKERS)
        },
        NonZeroUsize::get,
    );
    requested.min(classes.max(1))
}

fn sort_report(report: &mut RunReport) {
    report
        .notices
        .sort_by(|left, right| (&left.scope, &left.message).cmp(&(&right.scope, &right.message)));
    report.diagnostics.sort_by(|left, right| {
        (
            &left.entry,
            left.diagnostic.severity,
            left.diagnostic.code,
            &left.diagnostic.class_name,
            &left.diagnostic.method,
            &left.diagnostic.message,
        )
            .cmp(&(
                &right.entry,
                right.diagnostic.severity,
                right.diagnostic.code,
                &right.diagnostic.class_name,
                &right.diagnostic.method,
                &right.diagnostic.message,
            ))
    });
    report.failures.sort_by(|left, right| {
        (&left.entry, left.stage, &left.message).cmp(&(&right.entry, right.stage, &right.message))
    });
}

fn selected_classes(
    archive: &JarFile,
    command: &JarCommand,
) -> Result<BTreeMap<String, SelectedClass>> {
    let physical_classes = archive
        .class_entries()
        .map(|entry| entry.name.to_owned())
        .collect::<BTreeSet<_>>();
    let effective = archive
        .effective_entries(command.release.unwrap_or(NEWEST_AVAILABLE_RELEASE))
        .map_err(|source| Error::SelectJarView {
            path: command.input.clone(),
            source,
        })?;
    Ok(effective
        .into_iter()
        .filter(|entry| entry.logical_name.ends_with(CLASS_ENTRY_SUFFIX))
        .filter(|entry| physical_classes.contains(&entry.physical_name))
        .filter(|entry| {
            entry.release.is_some() || parse_versioned_entry(&entry.physical_name).is_none()
        })
        .map(|entry| {
            (
                entry.physical_name,
                SelectedClass {
                    logical_name: entry.logical_name,
                },
            )
        })
        .collect())
}

fn load_class(
    entry_name: &str,
    selected: &SelectedClass,
    payload: std::result::Result<&[u8], cafe::java::Error>,
    classes: &mut Vec<LoadedClass>,
    report: &mut RunReport,
) {
    let bytes = match payload {
        Ok(bytes) => bytes,
        Err(error) => {
            report.failures.push(ClassFailure {
                entry: entry_name.to_owned(),
                stage: FailureStage::Read,
                message: error.to_string(),
            });
            return;
        }
    };
    let class = match ClassFile::parse(bytes) {
        Ok(class) => class,
        Err(error) => {
            report.failures.push(ClassFailure {
                entry: entry_name.to_owned(),
                stage: FailureStage::Parse,
                message: error.to_string(),
            });
            return;
        }
    };
    let internal_name = match class.class_name() {
        Ok(name) => name.to_owned(),
        Err(error) => {
            report.failures.push(ClassFailure {
                entry: entry_name.to_owned(),
                stage: FailureStage::Metadata,
                message: error.to_string(),
            });
            return;
        }
    };
    if internal_name == MODULE_INFO_CLASS_NAME
        || class.access_flags.contains(ClassAccessFlags::MODULE)
    {
        report.skipped += 1;
        return;
    }
    let logical_internal_name = selected
        .logical_name
        .strip_suffix(CLASS_ENTRY_SUFFIX)
        .unwrap_or(&selected.logical_name);
    if logical_internal_name != internal_name {
        report.notices.push(ArchiveNotice {
            scope: entry_name.to_owned(),
            message: format!(
                "entry declares class `{internal_name}` instead of `{logical_internal_name}`; output follows the declaration"
            ),
        });
    }
    classes.push(LoadedClass {
        entry: entry_name.to_owned(),
        internal_name,
        class,
    });
}

fn record_diagnostics(report: &mut RunReport, entry: &str, recovered: DecompiledClass) {
    report
        .diagnostics
        .extend(
            recovered
                .diagnostics
                .into_iter()
                .map(|diagnostic| ArchiveDiagnostic {
                    entry: entry.to_owned(),
                    diagnostic,
                }),
        );
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};

    use cafe::java::classfile::{
        ClassAccessFlags, ClassFile, FieldAccessFlags, JAVA_8_MAJOR_VERSION, JAVA_17_MAJOR_VERSION,
    };
    use cafe::java::jar::JarFile;

    use super::{FailureStage, jar};
    use crate::cli::JarCommand;
    use crate::error::Error;

    static NEXT_TEMPORARY_DIRECTORY: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn decompiles_a_jar_into_a_package_tree() {
        let temporary = TemporaryDirectory::new();
        let input = temporary.path().join("application.jar");
        let output = temporary.path().join("source");
        write_jar(&input, &[new_class("sample/Hello", JAVA_8_MAJOR_VERSION)]);

        let report = jar(&command(&input, &output)).expect("decompile JAR");

        assert!(report.is_complete());
        assert_eq!(report.selected, 1);
        assert_eq!(report.written, 1);
        assert_eq!(report.skipped, 0);
        let source = fs::read_to_string(output.join("sample/Hello.java")).expect("read source");
        assert!(source.contains("package sample;"));
        assert!(source.contains("public class Hello"));
    }

    #[test]
    fn selects_the_requested_multi_release_view() {
        let temporary = TemporaryDirectory::new();
        let input = temporary.path().join("versions.jar");
        let mut archive = JarFile::new();
        let mut base = new_class("sample/Version", JAVA_8_MAJOR_VERSION);
        base.add_field(FieldAccessFlags::PUBLIC, "base", "I")
            .expect("add base field");
        let mut modern = new_class("sample/Version", JAVA_17_MAJOR_VERSION);
        modern
            .add_field(FieldAccessFlags::PUBLIC, "modern", "I")
            .expect("add modern field");
        archive.add_class(&base).expect("add base class");
        archive
            .add_versioned_file(
                17,
                "sample/Version.class",
                modern.to_bytes().expect("assemble modern class"),
            )
            .expect("add versioned class");
        archive
            .set_multi_release(true)
            .expect("enable multi-release");
        fs::write(&input, archive.to_bytes().expect("assemble JAR")).expect("write JAR");

        let newest_output = temporary.path().join("newest");
        let newest = jar(&command(&input, &newest_output)).expect("decompile newest view");
        assert!(newest.is_complete());
        let source = fs::read_to_string(newest_output.join("sample/Version.java"))
            .expect("read newest source");
        assert!(source.contains("int modern;"));
        assert!(!source.contains("int base;"));

        let base_output = temporary.path().join("base");
        let mut base_command = command(&input, &base_output);
        base_command.release = Some(11);
        let base_report = jar(&base_command).expect("decompile base view");
        assert!(base_report.is_complete());
        let source =
            fs::read_to_string(base_output.join("sample/Version.java")).expect("read base source");
        assert!(source.contains("int base;"));
        assert!(!source.contains("int modern;"));
    }

    #[test]
    fn continues_after_a_malformed_class_member() {
        let temporary = TemporaryDirectory::new();
        let input = temporary.path().join("mixed.jar");
        let output = temporary.path().join("source");
        let mut archive = JarFile::new();
        archive
            .add_file("sample/Broken.class", b"not a class".to_vec())
            .expect("add malformed member");
        archive
            .add_class(&new_class("sample/Good", JAVA_8_MAJOR_VERSION))
            .expect("add valid class");
        fs::write(&input, archive.to_bytes().expect("assemble JAR")).expect("write JAR");

        let report = jar(&command(&input, &output)).expect("aggregate decompilation");

        assert!(!report.is_complete());
        assert_eq!(report.written, 1);
        assert_eq!(report.failures.len(), 1);
        assert_eq!(report.failures[0].stage, FailureStage::Parse);
        assert!(output.join("sample/Good.java").is_file());
    }

    #[test]
    fn existing_sources_require_force_and_are_never_silently_replaced() {
        let temporary = TemporaryDirectory::new();
        let input = temporary.path().join("application.jar");
        let output = temporary.path().join("source");
        write_jar(&input, &[new_class("sample/Hello", JAVA_8_MAJOR_VERSION)]);
        let first = jar(&command(&input, &output)).expect("initial decompilation");
        assert!(first.is_complete());
        let source_path = output.join("sample/Hello.java");
        fs::write(&source_path, "owned by the user").expect("replace fixture source");

        let error = jar(&command(&input, &output)).expect_err("overwrite must be explicit");
        assert!(matches!(error, Error::SourceExists(_)), "{error:?}");
        assert_eq!(
            fs::read_to_string(&source_path).expect("read preserved source"),
            "owned by the user"
        );

        let mut forced = command(&input, &output);
        forced.force = true;
        let report = jar(&forced).expect("forced decompilation");
        assert!(report.is_complete());
        assert!(
            fs::read_to_string(source_path)
                .expect("read replaced source")
                .contains("public class Hello")
        );
    }

    #[cfg(windows)]
    #[test]
    fn case_insensitive_output_collisions_are_not_overwritten_by_force() {
        let temporary = TemporaryDirectory::new();
        let input = temporary.path().join("collision.jar");
        let output = temporary.path().join("source");
        write_jar(
            &input,
            &[
                new_class("sample/Name", JAVA_8_MAJOR_VERSION),
                new_class("sample/name", JAVA_8_MAJOR_VERSION),
            ],
        );
        let mut options = command(&input, &output);
        options.force = true;

        let report = jar(&options).expect("detect output collision");

        assert_eq!(report.written, 1);
        assert_eq!(report.failures.len(), 1);
        assert_eq!(report.failures[0].stage, FailureStage::Output);
        assert!(!report.is_complete());
    }

    fn command(input: &Path, output: &Path) -> JarCommand {
        JarCommand {
            input: input.to_path_buf(),
            output: output.to_path_buf(),
            release: None,
            force: false,
            exclude_synthetic: false,
            state_machine: false,
            jobs: None,
        }
    }

    fn new_class(name: &str, version: u16) -> ClassFile {
        ClassFile::new(
            version,
            name,
            Some("java/lang/Object"),
            ClassAccessFlags::PUBLIC,
        )
        .expect("build class")
    }

    fn write_jar(path: &Path, classes: &[ClassFile]) {
        let mut archive = JarFile::new();
        for class in classes {
            archive.add_class(class).expect("add class");
        }
        fs::write(path, archive.to_bytes().expect("assemble JAR")).expect("write JAR");
    }

    struct TemporaryDirectory {
        path: PathBuf,
    }

    impl TemporaryDirectory {
        fn new() -> Self {
            let base = std::env::temp_dir();
            loop {
                let id = NEXT_TEMPORARY_DIRECTORY.fetch_add(1, Ordering::Relaxed);
                let path = base.join(format!("cafe-cli-test-{}-{id}", std::process::id()));
                match fs::create_dir(&path) {
                    Ok(()) => return Self { path },
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                    Err(error) => panic!("create temporary directory: {error}"),
                }
            }
        }

        fn path(&self) -> &Path {
            &self.path
        }
    }

    impl Drop for TemporaryDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}

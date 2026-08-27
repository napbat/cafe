//! Whole-archive Java source decompilation.

use std::collections::{BTreeMap, BTreeSet};
use std::num::NonZeroUsize;
use std::panic::{self, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;
use std::thread;

use cafe::classpath::ClasspathHierarchy;
use cafe::decompiler::{
    ControlFlowPreference, DecompiledClass, DecompilerOptions, Diagnostic, DiagnosticSeverity,
    MethodExceptionCatalog, compilation_unit_path, decompile_compilation_unit_with_environment,
};
use cafe::java::classfile::{
    ClassAccessFlags, ClassFile, KnownAttribute, KnownAttributeKind, MODULE_INFO_CLASS_NAME,
};
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
struct ClassGroup {
    root: LoadedClass,
    members: Vec<LoadedClass>,
}

#[derive(Debug)]
struct PreparedUnit {
    entry: String,
    entries: BTreeMap<String, String>,
    destination: PathBuf,
    estimated_work: usize,
    root: ClassFile,
    members: Vec<ClassFile>,
}

#[derive(Debug)]
struct WorkerResult {
    index: usize,
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

    let method_exceptions = match MethodExceptionCatalog::from_classes(
        classes.iter().map(|loaded| &loaded.class),
    ) {
        Ok(catalog) => catalog,
        Err(error) => {
            report.notices.push(ArchiveNotice {
                    scope: command.input.display().to_string(),
                    message: format!(
                        "could not build archive method declarations; using conservative checked-exception rendering: {error}"
                    ),
                });
            MethodExceptionCatalog::default()
        }
    };

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
        ..DecompilerOptions::default()
    };
    let prepared = prepare_units(classes, &output_root, &mut report)?;
    decompile_prepared(
        &prepared,
        hierarchy.as_ref(),
        &method_exceptions,
        &options,
        worker_count(command.jobs, prepared.len()),
        command.force,
        &mut report,
    )?;
    sort_report(&mut report);

    Ok(report)
}

fn prepare_units(
    classes: Vec<LoadedClass>,
    output_root: &Path,
    report: &mut RunReport,
) -> Result<Vec<PreparedUnit>> {
    let mut destinations = BTreeMap::new();
    let groups = group_member_classes(classes);
    let mut prepared = Vec::with_capacity(groups.len());
    for group in groups {
        let estimated_work = class_work(&group.root.class).saturating_add(
            group
                .members
                .iter()
                .map(|member| class_work(&member.class))
                .fold(0usize, usize::saturating_add),
        );
        let destination = output::destination_path(
            output_root,
            &group.root.internal_name,
            &compilation_unit_path(&group.root.internal_name),
        )?;
        if let Some(previous) = destinations.insert(
            output::collision_key(&destination),
            group.root.entry.clone(),
        ) {
            report.failures.push(ClassFailure {
                entry: group.root.entry,
                stage: FailureStage::Output,
                message: format!(
                    "source path `{}` is already claimed by archive entry `{previous}`",
                    destination.display()
                ),
            });
            continue;
        }
        let mut entries = BTreeMap::new();
        entries.insert(group.root.internal_name, group.root.entry.clone());
        entries.extend(
            group
                .members
                .iter()
                .map(|member| (member.internal_name.clone(), member.entry.clone())),
        );
        prepared.push(PreparedUnit {
            entry: group.root.entry,
            entries,
            destination,
            estimated_work,
            root: group.root.class,
            members: group
                .members
                .into_iter()
                .map(|member| member.class)
                .collect(),
        });
    }
    // Long methods dominate archive completion. Starting large units first
    // overlaps them with ordinary classes instead of leaving a single-worker
    // tail after every small unit has already been written.
    schedule_units(&mut prepared);
    Ok(prepared)
}

fn schedule_units(units: &mut [PreparedUnit]) {
    units.sort_by(|left, right| {
        right
            .estimated_work
            .cmp(&left.estimated_work)
            .then_with(|| left.entry.cmp(&right.entry))
    });
}

fn class_work(class: &ClassFile) -> usize {
    let declarations = class.fields.len().saturating_add(class.methods.len());
    class
        .methods
        .iter()
        .filter_map(|method| method.code())
        .map(|code| code.code.len())
        .fold(declarations, usize::saturating_add)
}

fn decompile_prepared(
    classes: &[PreparedUnit],
    hierarchy: Option<&ClasspathHierarchy>,
    method_exceptions: &MethodExceptionCatalog,
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
                    let recovered = recover_unit(class, hierarchy, method_exceptions, options);
                    if sender.send(WorkerResult { index, recovered }).is_err() {
                        break;
                    }
                }
            });
        }
        drop(sender);
        for result in receiver {
            let class = &classes[result.index];
            match result.recovered {
                Ok(recovered) => {
                    output::write_source(&class.destination, &recovered.source, force)?;
                    record_diagnostics(report, &class.entry, &class.entries, recovered);
                    report.written += class.entries.len();
                }
                Err(message) => report.failures.push(ClassFailure {
                    entry: class.entry.clone(),
                    stage: FailureStage::Decompile,
                    message,
                }),
            }
        }
        Ok(())
    })
}

fn recover_unit(
    class: &PreparedUnit,
    hierarchy: Option<&ClasspathHierarchy>,
    method_exceptions: &MethodExceptionCatalog,
    options: &DecompilerOptions,
) -> std::result::Result<DecompiledClass, String> {
    match panic::catch_unwind(AssertUnwindSafe(|| {
        if let Some(hierarchy) = hierarchy {
            let view = hierarchy.jvm_view();
            let members = class.members.iter().collect::<Vec<_>>();
            decompile_compilation_unit_with_environment(
                &class.root,
                &members,
                Some(&view),
                method_exceptions,
                options,
            )
        } else {
            let members = class.members.iter().collect::<Vec<_>>();
            decompile_compilation_unit_with_environment(
                &class.root,
                &members,
                None,
                method_exceptions,
                options,
            )
        }
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

fn group_member_classes(classes: Vec<LoadedClass>) -> Vec<ClassGroup> {
    let mut unique_indices = BTreeMap::<String, Option<usize>>::new();
    for (index, class) in classes.iter().enumerate() {
        unique_indices
            .entry(class.internal_name.clone())
            .and_modify(|value| *value = None)
            .or_insert(Some(index));
    }
    let parents = classes
        .iter()
        .map(|loaded| direct_member_outer(&loaded.class))
        .collect::<Vec<_>>();
    let roots = (0..classes.len())
        .map(|index| member_root(index, &classes, &parents, &unique_indices))
        .collect::<Vec<_>>();
    let mut grouped = BTreeMap::<usize, Vec<(usize, LoadedClass)>>::new();
    for (index, class) in classes.into_iter().enumerate() {
        grouped
            .entry(roots[index])
            .or_default()
            .push((index, class));
    }
    let mut result = Vec::with_capacity(grouped.len());
    for (root_index, mut classes) in grouped {
        let root_position = classes
            .iter()
            .position(|(index, _)| *index == root_index)
            .expect("every member group retains its root class");
        let root = classes.swap_remove(root_position).1;
        let mut members = classes
            .into_iter()
            .map(|(_, class)| class)
            .collect::<Vec<_>>();
        members.sort_by(|left, right| left.internal_name.cmp(&right.internal_name));
        result.push(ClassGroup { root, members });
    }
    result.sort_by(|left, right| left.root.internal_name.cmp(&right.root.internal_name));
    result
}

fn member_root(
    original: usize,
    classes: &[LoadedClass],
    parents: &[Option<String>],
    unique_indices: &BTreeMap<String, Option<usize>>,
) -> usize {
    let mut current = original;
    let mut visited = BTreeSet::new();
    while let Some(parent) = parents.get(current).and_then(Option::as_ref) {
        if !visited.insert(current) {
            return original;
        }
        let Some(Some(parent_index)) = unique_indices.get(parent) else {
            return current;
        };
        if is_local_or_anonymous(&classes[*parent_index].class) {
            return original;
        }
        current = *parent_index;
    }
    current
}

fn direct_member_outer(class: &ClassFile) -> Option<String> {
    if is_local_or_anonymous(class) {
        return None;
    }
    let KnownAttribute::InnerClasses(attribute) =
        class.known_attribute(KnownAttributeKind::InnerClasses)?
    else {
        return None;
    };
    attribute
        .classes
        .iter()
        .find(|entry| {
            entry.inner_class_info_index == class.this_class
                && entry.outer_class_info_index != 0
                && entry.inner_name_index != 0
        })
        .and_then(|entry| {
            class
                .constant_pool
                .class_name(entry.outer_class_info_index)
                .ok()
                .map(str::to_owned)
        })
}

fn is_local_or_anonymous(class: &ClassFile) -> bool {
    if class
        .known_attribute(KnownAttributeKind::EnclosingMethod)
        .is_some()
    {
        return true;
    }
    let Some(KnownAttribute::InnerClasses(attribute)) =
        class.known_attribute(KnownAttributeKind::InnerClasses)
    else {
        return false;
    };
    attribute.classes.iter().any(|entry| {
        entry.inner_class_info_index == class.this_class
            && (entry.outer_class_info_index == 0 || entry.inner_name_index == 0)
    })
}

fn record_diagnostics(
    report: &mut RunReport,
    root_entry: &str,
    entries: &BTreeMap<String, String>,
    recovered: DecompiledClass,
) {
    report
        .diagnostics
        .extend(recovered.diagnostics.into_iter().map(|diagnostic| {
            ArchiveDiagnostic {
                entry: entries
                    .get(&diagnostic.class_name)
                    .map_or_else(|| root_entry.to_owned(), Clone::clone),
                diagnostic,
            }
        }));
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};

    use cafe::java::classfile::{
        Attribute, ClassAccessFlags, ClassFile, FieldAccessFlags, InnerClass,
        InnerClassAccessFlags, InnerClassesAttribute, JAVA_8_MAJOR_VERSION, JAVA_17_MAJOR_VERSION,
        KnownAttribute, KnownAttributeKind,
    };
    use cafe::java::jar::JarFile;

    use super::{FailureStage, PreparedUnit, jar, schedule_units};
    use crate::cli::JarCommand;
    use crate::error::Error;

    static NEXT_TEMPORARY_DIRECTORY: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn schedules_largest_units_first_with_deterministic_ties() {
        let unit = |entry: &str, estimated_work| PreparedUnit {
            entry: entry.to_owned(),
            entries: BTreeMap::new(),
            destination: PathBuf::from(entry),
            estimated_work,
            root: new_class(entry, JAVA_8_MAJOR_VERSION),
            members: Vec::new(),
        };
        let mut units = vec![unit("small", 10), unit("z-large", 20), unit("a-large", 20)];

        schedule_units(&mut units);

        assert_eq!(
            units
                .iter()
                .map(|prepared| prepared.entry.as_str())
                .collect::<Vec<_>>(),
            ["a-large", "z-large", "small"]
        );
    }

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
    fn writes_named_member_classes_inside_the_enclosing_source_file() {
        let temporary = TemporaryDirectory::new();
        let input = temporary.path().join("members.jar");
        let output = temporary.path().join("source");
        let flags = InnerClassAccessFlags::PRIVATE
            | InnerClassAccessFlags::STATIC
            | InnerClassAccessFlags::FINAL;
        let mut outer = new_class("sample/Outer", JAVA_8_MAJOR_VERSION);
        add_member_metadata(
            &mut outer,
            "sample/Outer$Inner",
            "sample/Outer",
            "Inner",
            flags,
        );
        let mut inner = ClassFile::new(
            JAVA_8_MAJOR_VERSION,
            "sample/Outer$Inner",
            Some("java/lang/Object"),
            ClassAccessFlags::FINAL | ClassAccessFlags::SUPER,
        )
        .expect("build member class");
        add_member_metadata(
            &mut inner,
            "sample/Outer$Inner",
            "sample/Outer",
            "Inner",
            flags,
        );
        write_jar(&input, &[outer, inner]);

        let report = jar(&command(&input, &output)).expect("decompile member classes");

        assert!(report.is_complete());
        assert_eq!(report.selected, 2);
        assert_eq!(report.written, 2);
        assert!(!output.join("sample/Outer$Inner.java").exists());
        let source = fs::read_to_string(output.join("sample/Outer.java")).expect("read source");
        assert!(
            source.contains("private static final class Inner"),
            "{source}"
        );
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

    fn add_member_metadata(
        class: &mut ClassFile,
        inner: &str,
        outer: &str,
        simple: &str,
        access_flags: InnerClassAccessFlags,
    ) {
        let name_index = class
            .constant_pool
            .intern_utf8(KnownAttributeKind::InnerClasses.name())
            .expect("attribute name");
        let inner_class_info_index = class
            .constant_pool
            .intern_class(inner)
            .expect("inner class");
        let outer_class_info_index = class
            .constant_pool
            .intern_class(outer)
            .expect("outer class");
        let inner_name_index = class.constant_pool.intern_utf8(simple).expect("inner name");
        class
            .attributes
            .push(Attribute::Known(KnownAttribute::InnerClasses(
                InnerClassesAttribute {
                    name_index,
                    classes: vec![InnerClass {
                        inner_class_info_index,
                        outer_class_info_index,
                        inner_name_index,
                        access_flags,
                    }],
                },
            )));
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

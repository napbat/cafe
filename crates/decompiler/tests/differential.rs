//! Differential execution harness: the original javac bytecode is the
//! oracle. Fixture sources compile with javac, every class decompiles and
//! recompiles, and both versions run under one reflective driver whose
//! transcripts must match byte-for-byte — return values and thrown
//! exception classes alike. Methods the decompiler stubs are excluded
//! from execution but pinned as an explicit coverage list, so a new stub
//! fails the harness instead of silently shrinking it.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use decompiler::{DiagnosticSeverity, decompile_class_bytes};

type TestResult<T> = Result<T, Box<dyn std::error::Error>>;

const PACKAGE: &str = "differential";
const FIXTURE_CLASSES: &[&str] = &["Flow", "Numbers", "Things"];

/// Methods the decompiler is expected to stub today, per fixture class.
/// Shrinking this list is progress; growing it is a regression.
const EXPECTED_STUBS: &[(&str, &[&str])] = &[("Flow", &[]), ("Numbers", &[]), ("Things", &[])];

fn fixtures_directory() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
}

fn temporary_directory() -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock follows Unix epoch")
        .as_nanos();
    std::env::temp_dir().join(format!("cafe-differential-{}-{nonce}", std::process::id()))
}

fn classpath_separator() -> &'static str {
    if cfg!(windows) { ";" } else { ":" }
}

/// Compiles Java sources targeting bytecode version 8 so string
/// concatenation stays `StringBuilder`-based instead of `invokedynamic`;
/// a javac without `--release` (JDK 8 itself) compiles at its default.
fn compile(sources: &[PathBuf], destination: &Path, context: &str) -> TestResult<()> {
    fs::create_dir_all(destination)?;
    let attempt = |release: bool| -> TestResult<std::process::Output> {
        let mut command = Command::new("javac");
        if release {
            command.arg("--release").arg("8");
        }
        command.arg("-d").arg(destination);
        for source in sources {
            command.arg(source);
        }
        Ok(command.output()?)
    };
    let mut output = attempt(true)?;
    if !output.status.success() && String::from_utf8_lossy(&output.stderr).contains("--release") {
        output = attempt(false)?;
    }
    if !output.status.success() {
        let mut rendered = String::new();
        for source in sources {
            rendered.push_str(&fs::read_to_string(source)?);
            rendered.push('\n');
        }
        return Err(format!(
            "javac failed for {context}:\n{}\nsources:\n{rendered}",
            String::from_utf8_lossy(&output.stderr)
        )
        .into());
    }
    Ok(())
}

/// Runs the driver against one classpath and returns its transcript.
fn transcript(driver: &Path, classes: &Path, target: &str, skips: &[String]) -> TestResult<String> {
    let classpath = format!(
        "{}{}{}",
        driver.display(),
        classpath_separator(),
        classes.display()
    );
    let mut command = Command::new("java");
    command
        .arg("-cp")
        .arg(&classpath)
        .arg("CafeDriver")
        .arg(target);
    for skip in skips {
        command.arg(skip);
    }
    let output = command.output()?;
    if !output.status.success() {
        return Err(format!(
            "driver failed for {target} on {}:\n{}",
            classes.display(),
            String::from_utf8_lossy(&output.stderr)
        )
        .into());
    }
    Ok(String::from_utf8(output.stdout)?)
}

#[test]
fn decompiled_fixtures_behave_identically_when_javac_is_available() -> TestResult<()> {
    if Command::new("javac").arg("-version").output().is_err()
        || Command::new("java").arg("-version").output().is_err()
    {
        return Ok(());
    }
    let root = temporary_directory();
    let original = root.join("original");
    let recompiled = root.join("recompiled");
    let driver = root.join("driver");

    // The oracle: fixture sources compiled by the local javac.
    let fixture_sources: Vec<PathBuf> = FIXTURE_CLASSES
        .iter()
        .map(|class| fixtures_directory().join(format!("{class}.java")))
        .collect();
    compile(&fixture_sources, &original, "fixture sources")?;
    compile(
        &[fixtures_directory().join("CafeDriver.java")],
        &driver,
        "the differential driver",
    )?;

    // Decompile every original class; collect its stubbed-method skips.
    let decompiled_directory = root.join("src").join(PACKAGE);
    fs::create_dir_all(&decompiled_directory)?;
    let mut skips: BTreeMap<&str, Vec<String>> = BTreeMap::new();
    let mut decompiled_sources = Vec::new();
    for &class in FIXTURE_CLASSES {
        let bytes = fs::read(original.join(PACKAGE).join(format!("{class}.class")))?;
        let output = decompile_class_bytes(&bytes)?;
        let source_file = decompiled_directory.join(format!("{class}.java"));
        fs::write(&source_file, &output.source)?;
        decompiled_sources.push(source_file);
        let class_level_errors: Vec<_> = output
            .diagnostics
            .iter()
            .filter(|diagnostic| {
                diagnostic.severity == DiagnosticSeverity::Error && diagnostic.method.is_none()
            })
            .collect();
        assert!(
            class_level_errors.is_empty(),
            "{class} produced class-level errors: {class_level_errors:#?}"
        );
        let mut stubbed: Vec<String> = output
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.severity == DiagnosticSeverity::Error)
            .filter_map(|diagnostic| diagnostic.method.as_ref())
            .map(|method| format!("{}{}", method.name, method.descriptor))
            .collect();
        stubbed.sort();
        stubbed.dedup();
        skips.insert(class, stubbed);
    }

    // Pin stub coverage: growth is a regression, shrinkage is progress.
    for &(class, expected) in EXPECTED_STUBS {
        assert_eq!(
            skips[class], expected,
            "stubbed methods changed for {class}; update EXPECTED_STUBS if intentional"
        );
    }

    compile(&decompiled_sources, &recompiled, "decompiled sources")?;

    // The differential comparison itself.
    for &class in FIXTURE_CLASSES {
        let target = format!("{PACKAGE}.{class}");
        let expected = transcript(&driver, &original, &target, &skips[class])?;
        let actual = transcript(&driver, &recompiled, &target, &skips[class])?;
        assert!(
            expected.lines().count() > 10,
            "the {target} transcript is suspiciously small:\n{expected}"
        );
        assert_eq!(
            expected, actual,
            "behavioral divergence for {target}\n--- original ---\n{expected}\n--- decompiled ---\n{actual}"
        );
    }

    fs::remove_dir_all(&root)?;
    Ok(())
}

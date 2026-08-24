//! Deterministic, non-fail-fast validation for JVM artifact corpora.
//!
//! Reports retain native artifact and class origins, overload-qualified method
//! identities, and the most specific byte offset available for every failure.

use std::cmp::Ordering;
use std::fs;
use std::path::{Path, PathBuf};

use crate::bytecode::{self, Operand};
use crate::classfile::{ClassFile, MethodInfo};
use crate::jar::{ClassVisitControl, JarFile};
use crate::jimage::{JimageFile, JimageVisitControl};
use crate::jmod::JmodFile;
use crate::{Error, Result, descriptor};

/// Kind of physical JVM artifact included in a corpus.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum CorpusArtifactKind {
    /// One standalone `.class` file.
    Class,
    /// One Java archive.
    Jar,
    /// One JDK module archive.
    Jmod,
    /// One JDK module image.
    Jimage,
}

/// Validation phase in which a corpus failure occurred.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum CorpusStage {
    /// Reading or decompressing a container member.
    Read,
    /// Parsing a class file.
    ClassParse,
    /// Resolving or validating class metadata.
    Metadata,
    /// Assembling a parsed class back to bytes.
    ClassAssembly,
    /// Parsing a field or method descriptor.
    Descriptor,
    /// Decoding a JVM method body.
    BytecodeDecode,
    /// Encoding decoded instructions back to bytes.
    BytecodeEncode,
    /// Resolving an instruction's constant-pool reference.
    ConstantReference,
    /// Lowering or verifying shared control flow.
    ControlFlow,
}

/// Overload-qualified method identity retained in a failure.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct CorpusMethod {
    /// JVM method name.
    pub name: String,
    /// Exact JVM method descriptor.
    pub descriptor: String,
}

/// One contextual corpus validation failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CorpusFailure {
    /// Caller-supplied artifact identity, normally a path.
    pub artifact: String,
    /// Physical member or resource name, if the artifact is a container.
    pub entry: Option<String>,
    /// Declared internal class name, or the path-derived name if parsing failed.
    pub class: Option<String>,
    /// Overload-qualified method when the failure belongs to a method.
    pub method: Option<CorpusMethod>,
    /// Byte offset in the most local failing unit. Zero is the format-defined
    /// start when an underlying library cannot provide a narrower location.
    pub byte_offset: usize,
    /// Validation phase.
    pub stage: CorpusStage,
    /// Stable human-readable error detail.
    pub message: String,
}

impl Ord for CorpusFailure {
    fn cmp(&self, other: &Self) -> Ordering {
        (
            &self.artifact,
            &self.entry,
            &self.class,
            &self.method,
            self.byte_offset,
            self.stage,
            &self.message,
        )
            .cmp(&(
                &other.artifact,
                &other.entry,
                &other.class,
                &other.method,
                other.byte_offset,
                other.stage,
                &other.message,
            ))
    }
}

impl PartialOrd for CorpusFailure {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Aggregate deterministic validation results.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CorpusReport {
    /// Number of artifacts visited.
    pub artifacts: usize,
    /// Number of class payloads attempted.
    pub classes: usize,
    /// Number of methods whose metadata was visited.
    pub methods: usize,
    /// Number of methods containing bytecode.
    pub code_methods: usize,
    /// Number of decoded instructions.
    pub instructions: usize,
    /// Number of verified cfglib-backed method graphs.
    pub control_flow_graphs: usize,
    /// Sorted failures; validation continues after independent errors.
    pub failures: Vec<CorpusFailure>,
}

impl CorpusReport {
    /// Returns whether every visited unit passed validation.
    #[must_use]
    pub fn is_success(&self) -> bool {
        self.failures.is_empty()
    }
}

/// Owned input artifact, retaining lazy archive payloads where supported.
#[derive(Debug, Clone)]
pub struct CorpusArtifact {
    name: String,
    value: CorpusArtifactValue,
}

#[derive(Debug, Clone)]
enum CorpusArtifactValue {
    Class(Vec<u8>),
    Jar(JarFile),
    Jmod(JmodFile),
    Jimage(JimageFile),
}

impl CorpusArtifact {
    /// Creates an artifact for a standalone class payload.
    #[must_use]
    pub fn class(name: impl Into<String>, bytes: impl Into<Vec<u8>>) -> Self {
        Self {
            name: name.into(),
            value: CorpusArtifactValue::Class(bytes.into()),
        }
    }

    /// Creates an artifact for an already opened JAR.
    #[must_use]
    pub fn jar(name: impl Into<String>, jar: JarFile) -> Self {
        Self {
            name: name.into(),
            value: CorpusArtifactValue::Jar(jar),
        }
    }

    /// Creates an artifact for an already opened JMOD.
    #[must_use]
    pub fn jmod(name: impl Into<String>, jmod: JmodFile) -> Self {
        Self {
            name: name.into(),
            value: CorpusArtifactValue::Jmod(jmod),
        }
    }

    /// Creates an artifact for an already opened JIMAGE.
    #[must_use]
    pub fn jimage(name: impl Into<String>, jimage: JimageFile) -> Self {
        Self {
            name: name.into(),
            value: CorpusArtifactValue::Jimage(jimage),
        }
    }

    /// Opens an artifact based on its conventional suffix or binary magic.
    ///
    /// # Errors
    ///
    /// Returns an error for unreadable input, an unsupported artifact kind, or
    /// malformed container metadata.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let bytes = fs::read(path)?;
        let name = path.to_string_lossy().into_owned();
        let extension = path
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or_default();
        let value = if extension.eq_ignore_ascii_case("class") {
            CorpusArtifactValue::Class(bytes)
        } else if extension.eq_ignore_ascii_case("jar") {
            CorpusArtifactValue::Jar(JarFile::from_bytes(bytes)?)
        } else if extension.eq_ignore_ascii_case("jmod") {
            CorpusArtifactValue::Jmod(JmodFile::from_bytes(bytes)?)
        } else if looks_like_jimage(&bytes) {
            CorpusArtifactValue::Jimage(JimageFile::from_bytes(bytes)?)
        } else {
            return Err(Error::InvalidJar(format!(
                "unsupported JVM corpus artifact `{}`",
                path.display()
            )));
        };
        Ok(Self { name, value })
    }

    /// Returns the caller-visible artifact identity.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the physical artifact kind.
    #[must_use]
    pub const fn kind(&self) -> CorpusArtifactKind {
        match self.value {
            CorpusArtifactValue::Class(_) => CorpusArtifactKind::Class,
            CorpusArtifactValue::Jar(_) => CorpusArtifactKind::Jar,
            CorpusArtifactValue::Jmod(_) => CorpusArtifactKind::Jmod,
            CorpusArtifactValue::Jimage(_) => CorpusArtifactKind::Jimage,
        }
    }
}

/// Mutable collection of artifacts to validate in a deterministic pass.
#[derive(Debug, Clone, Default)]
pub struct Corpus {
    artifacts: Vec<CorpusArtifact>,
}

impl Corpus {
    /// Creates an empty corpus.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            artifacts: Vec::new(),
        }
    }

    /// Adds an already opened artifact.
    pub fn push(&mut self, artifact: CorpusArtifact) {
        self.artifacts.push(artifact);
    }

    /// Opens and adds an artifact.
    ///
    /// # Errors
    ///
    /// Returns an error if the artifact cannot be opened or classified.
    pub fn open(&mut self, path: impl AsRef<Path>) -> Result<()> {
        self.push(CorpusArtifact::open(path)?);
        Ok(())
    }

    /// Returns input artifacts in insertion order.
    #[must_use]
    pub fn artifacts(&self) -> &[CorpusArtifact] {
        &self.artifacts
    }

    /// Validates every artifact and returns all independent failures sorted by
    /// native origin.
    #[must_use]
    pub fn validate(&self) -> CorpusReport {
        let mut report = CorpusReport::default();
        let mut artifacts = self.artifacts.iter().collect::<Vec<_>>();
        artifacts.sort_by(|left, right| left.name.cmp(&right.name));
        for artifact in artifacts {
            report.artifacts += 1;
            validate_artifact(artifact, &mut report);
        }
        report.failures.sort();
        report
    }
}

fn validate_artifact(artifact: &CorpusArtifact, report: &mut CorpusReport) {
    match &artifact.value {
        CorpusArtifactValue::Class(bytes) => {
            let inferred = inferred_class(&artifact.name);
            validate_class(artifact.name(), None, inferred.as_deref(), bytes, report);
        }
        CorpusArtifactValue::Jar(jar) => {
            let result: std::result::Result<(), std::convert::Infallible> = jar.visit_class_bytes(
                |_| true,
                |entry, bytes| {
                    validate_payload(
                        artifact.name(),
                        Some(entry.name),
                        inferred_class(entry.name),
                        bytes,
                        report,
                    );
                    Ok(ClassVisitControl::Continue)
                },
            );
            match result {
                Ok(()) => {}
                Err(error) => match error {},
            }
        }
        CorpusArtifactValue::Jmod(jmod) => {
            let result: std::result::Result<(), std::convert::Infallible> = jmod.visit_class_bytes(
                |_| true,
                |entry, bytes| {
                    validate_payload(
                        artifact.name(),
                        Some(entry.physical_name),
                        inferred_class(entry.name),
                        bytes,
                        report,
                    );
                    Ok(ClassVisitControl::Continue)
                },
            );
            match result {
                Ok(()) => {}
                Err(error) => match error {},
            }
        }
        CorpusArtifactValue::Jimage(image) => {
            let result: std::result::Result<(), std::convert::Infallible> = image
                .visit_class_bytes(
                    |_| true,
                    |entry, bytes| {
                        validate_payload(
                            artifact.name(),
                            Some(entry.resource_name),
                            Some(entry.class_name.to_owned()),
                            bytes,
                            report,
                        );
                        Ok(JimageVisitControl::Continue)
                    },
                );
            match result {
                Ok(()) => {}
                Err(error) => match error {},
            }
        }
    }
}

fn validate_payload(
    artifact: &str,
    entry: Option<&str>,
    class: Option<String>,
    bytes: std::result::Result<&[u8], Error>,
    report: &mut CorpusReport,
) {
    match bytes {
        Ok(bytes) => validate_class(artifact, entry, class.as_deref(), bytes, report),
        Err(error) => {
            report.classes += 1;
            failure(
                report,
                artifact,
                entry,
                class,
                None,
                CorpusStage::Read,
                error,
            );
        }
    }
}

fn validate_class(
    artifact: &str,
    entry: Option<&str>,
    inferred: Option<&str>,
    bytes: &[u8],
    report: &mut CorpusReport,
) {
    report.classes += 1;
    let class = match ClassFile::parse(bytes) {
        Ok(class) => class,
        Err(error) => {
            failure(
                report,
                artifact,
                entry,
                inferred.map(str::to_owned),
                None,
                CorpusStage::ClassParse,
                error,
            );
            return;
        }
    };
    let class_name = match class.class_name() {
        Ok(name) => Some(name.to_owned()),
        Err(error) => {
            failure(
                report,
                artifact,
                entry,
                inferred.map(str::to_owned),
                None,
                CorpusStage::Metadata,
                error,
            );
            None
        }
    };
    match class.to_bytes() {
        Ok(assembled) if assembled != bytes => report.failures.push(CorpusFailure {
            artifact: artifact.to_owned(),
            entry: entry.map(str::to_owned),
            class: class_name.clone().or_else(|| inferred.map(str::to_owned)),
            method: None,
            byte_offset: first_difference(bytes, &assembled),
            stage: CorpusStage::ClassAssembly,
            message: "parse/assemble round trip changed the class-file bytes".to_owned(),
        }),
        Ok(_) => {}
        Err(error) => failure(
            report,
            artifact,
            entry,
            class_name.clone().or_else(|| inferred.map(str::to_owned)),
            None,
            CorpusStage::ClassAssembly,
            error,
        ),
    }
    validate_fields(artifact, entry, &class, class_name.as_deref(), report);
    for method in &class.methods {
        validate_method(
            artifact,
            entry,
            &class,
            class_name.as_deref(),
            method,
            report,
        );
    }
}

fn validate_fields(
    artifact: &str,
    entry: Option<&str>,
    class: &ClassFile,
    class_name: Option<&str>,
    report: &mut CorpusReport,
) {
    for field in &class.fields {
        match field.descriptor(&class.constant_pool) {
            Ok(value) => {
                if let Err(error) = descriptor::parse_field(value) {
                    failure(
                        report,
                        artifact,
                        entry,
                        class_name.map(str::to_owned),
                        None,
                        CorpusStage::Descriptor,
                        error,
                    );
                }
            }
            Err(error) => failure(
                report,
                artifact,
                entry,
                class_name.map(str::to_owned),
                None,
                CorpusStage::Metadata,
                error,
            ),
        }
    }
}

fn validate_method(
    artifact: &str,
    entry: Option<&str>,
    class: &ClassFile,
    class_name: Option<&str>,
    method: &MethodInfo,
    report: &mut CorpusReport,
) {
    report.methods += 1;
    let name = method.name(&class.constant_pool).map(str::to_owned);
    let descriptor_value = method.descriptor(&class.constant_pool).map(str::to_owned);
    let identity = match (&name, &descriptor_value) {
        (Ok(name), Ok(descriptor)) => Some(CorpusMethod {
            name: name.clone(),
            descriptor: descriptor.clone(),
        }),
        _ => None,
    };
    if let Err(error) = &name {
        failure_ref(
            report,
            artifact,
            entry,
            class_name,
            identity.clone(),
            CorpusStage::Metadata,
            error,
        );
    }
    if let Err(error) = &descriptor_value {
        failure_ref(
            report,
            artifact,
            entry,
            class_name,
            identity.clone(),
            CorpusStage::Metadata,
            error,
        );
    }
    if let Ok(descriptor_value) = &descriptor_value
        && let Err(error) = descriptor::parse_method(descriptor_value)
    {
        failure(
            report,
            artifact,
            entry,
            class_name.map(str::to_owned),
            identity.clone(),
            CorpusStage::Descriptor,
            error,
        );
    }

    validate_method_code(artifact, entry, class, class_name, method, identity, report);
}

#[allow(clippy::too_many_arguments)]
fn validate_method_code(
    artifact: &str,
    entry: Option<&str>,
    class: &ClassFile,
    class_name: Option<&str>,
    method: &MethodInfo,
    identity: Option<CorpusMethod>,
    report: &mut CorpusReport,
) {
    let Some(code) = method.code() else {
        return;
    };
    report.code_methods += 1;
    let instructions = match bytecode::decode_code(code) {
        Ok(instructions) => instructions,
        Err(error) => {
            failure(
                report,
                artifact,
                entry,
                class_name.map(str::to_owned),
                identity,
                CorpusStage::BytecodeDecode,
                error,
            );
            return;
        }
    };
    report.instructions += instructions.len();
    match bytecode::encode(&instructions) {
        Ok(encoded) if encoded != code.code => report.failures.push(CorpusFailure {
            artifact: artifact.to_owned(),
            entry: entry.map(str::to_owned),
            class: class_name.map(str::to_owned),
            method: identity.clone(),
            byte_offset: first_difference(&code.code, &encoded),
            stage: CorpusStage::BytecodeEncode,
            message: "decode/encode round trip changed the method bytecode".to_owned(),
        }),
        Ok(_) => {}
        Err(error) => failure(
            report,
            artifact,
            entry,
            class_name.map(str::to_owned),
            identity.clone(),
            CorpusStage::BytecodeEncode,
            error,
        ),
    }
    for instruction in &instructions {
        if let Some(index) = referenced_constant(&instruction.operand)
            && let Err(error) = class.constant_pool.describe(index)
        {
            report.failures.push(CorpusFailure {
                artifact: artifact.to_owned(),
                entry: entry.map(str::to_owned),
                class: class_name.map(str::to_owned),
                method: identity.clone(),
                byte_offset: instruction.offset,
                stage: CorpusStage::ConstantReference,
                message: error.to_string(),
            });
        }
    }
    if let Some(owner) = class_name {
        match crate::disassembly::lower_method(class, method, owner) {
            Ok(function) => {
                if let Some(body) = function.body {
                    match body.control_flow_graph() {
                        Ok(_) => report.control_flow_graphs += 1,
                        Err(error) => failure(
                            report,
                            artifact,
                            entry,
                            Some(owner.to_owned()),
                            identity,
                            CorpusStage::ControlFlow,
                            Error::from(error),
                        ),
                    }
                }
            }
            Err(error) => failure(
                report,
                artifact,
                entry,
                Some(owner.to_owned()),
                identity,
                CorpusStage::ControlFlow,
                error,
            ),
        }
    }
}

fn failure(
    report: &mut CorpusReport,
    artifact: &str,
    entry: Option<&str>,
    class: Option<String>,
    method: Option<CorpusMethod>,
    stage: CorpusStage,
    error: Error,
) {
    let byte_offset = error_offset(&error);
    let message = error.to_string();
    drop(error);
    report.failures.push(CorpusFailure {
        artifact: artifact.to_owned(),
        entry: entry.map(str::to_owned),
        class,
        method,
        byte_offset,
        stage,
        message,
    });
}

fn failure_ref(
    report: &mut CorpusReport,
    artifact: &str,
    entry: Option<&str>,
    class: Option<&str>,
    method: Option<CorpusMethod>,
    stage: CorpusStage,
    error: &Error,
) {
    report.failures.push(CorpusFailure {
        artifact: artifact.to_owned(),
        entry: entry.map(str::to_owned),
        class: class.map(str::to_owned),
        method,
        byte_offset: error_offset(error),
        stage,
        message: error.to_string(),
    });
}

fn error_offset(error: &Error) -> usize {
    match error {
        Error::InvalidClass { offset, .. }
        | Error::InvalidBytecode { offset, .. }
        | Error::InvalidDescriptor { offset, .. }
        | Error::InvalidJmod { offset, .. }
        | Error::InvalidJimage { offset, .. } => *offset,
        Error::ClassMethod { source, .. } | Error::JarEntry { source, .. } => error_offset(source),
        _ => 0,
    }
}

fn first_difference(left: &[u8], right: &[u8]) -> usize {
    left.iter()
        .zip(right)
        .position(|(left, right)| left != right)
        .unwrap_or_else(|| left.len().min(right.len()))
}

const fn referenced_constant(operand: &Operand) -> Option<u16> {
    match operand {
        Operand::Constant(index)
        | Operand::InvokeDynamic(index)
        | Operand::InvokeInterface { index, .. }
        | Operand::MultiArray { index, .. } => Some(*index),
        Operand::None
        | Operand::Byte(_)
        | Operand::Short(_)
        | Operand::Local(_)
        | Operand::Increment { .. }
        | Operand::Branch(_)
        | Operand::TableSwitch { .. }
        | Operand::LookupSwitch { .. }
        | Operand::ArrayType(_) => None,
    }
}

fn inferred_class(name: &str) -> Option<String> {
    let path = PathBuf::from(name.replace('\\', "/"));
    let value = path.to_string_lossy();
    let value = value
        .strip_prefix("classes/")
        .unwrap_or(&value)
        .strip_suffix(crate::jar::CLASS_ENTRY_SUFFIX)?;
    Some(value.trim_start_matches('/').to_owned())
}

fn looks_like_jimage(bytes: &[u8]) -> bool {
    let Some(raw) = bytes.get(..size_of::<u32>()) else {
        return false;
    };
    let raw: [u8; 4] = raw.try_into().expect("slice width checked");
    u32::from_le_bytes(raw) == crate::jimage::JIMAGE_MAGIC
        || u32::from_be_bytes(raw) == crate::jimage::JIMAGE_MAGIC
}

#[cfg(test)]
mod tests {
    use crate::classfile::{ClassAccessFlags, JAVA_8_MAJOR_VERSION};

    use super::*;

    #[test]
    fn aggregates_and_sorts_contextual_failures() -> Result<()> {
        let valid = ClassFile::new(
            JAVA_8_MAJOR_VERSION,
            "sample/Valid",
            Some("java/lang/Object"),
            ClassAccessFlags::PUBLIC,
        )?;
        let mut jar = JarFile::new();
        jar.add_file("sample/Z.class", b"broken-z".to_vec())?;
        jar.add_class(&valid)?;
        jar.add_file("sample/A.class", b"broken-a".to_vec())?;

        let mut corpus = Corpus::new();
        corpus.push(CorpusArtifact::jar("fixture.jar", jar));
        let report = corpus.validate();

        assert_eq!(report.classes, 3);
        assert_eq!(report.failures.len(), 2);
        assert_eq!(report.failures[0].entry.as_deref(), Some("sample/A.class"));
        assert_eq!(report.failures[0].class.as_deref(), Some("sample/A"));
        assert_eq!(report.failures[0].byte_offset, 0);
        assert_eq!(report.failures[1].entry.as_deref(), Some("sample/Z.class"));
        Ok(())
    }
}

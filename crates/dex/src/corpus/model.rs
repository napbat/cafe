//! Corpus inputs and deterministic report values.

use std::cmp::Ordering;
use std::fs;
use std::path::Path;

use crate::{Error, Result};

/// Kind of physical Android artifact included in a corpus.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum CorpusArtifactKind {
    /// Standalone standard DEX or a DEX 041 multi-header container.
    Dex,
    /// Android application package.
    Apk,
    /// Android App Bundle.
    Aab,
}

/// Validation phase in which a corpus failure occurred.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum CorpusStage {
    /// Parsing container metadata or validating archive layout.
    Container,
    /// Reading or decompressing a container member.
    Read,
    /// Parsing and validating a standard DEX file or container.
    DexParse,
    /// Reassembling DEX or APK bytes.
    Assembly,
    /// Resolving a DEX identifier or method identity.
    Resolution,
    /// Encoding a decoded Dalvik instruction stream.
    InstructionEncode,
    /// Lowering and verifying shared control flow.
    ControlFlow,
    /// Running fixed-point register analysis.
    RegisterAnalysis,
    /// Lowering declarations or bodies into Program.
    Program,
}

/// Overload-qualified DEX method identity retained in a failure.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct CorpusMethod {
    /// Method name.
    pub name: String,
    /// Exact JVM-style method descriptor used by DEX.
    pub descriptor: String,
}

/// One contextual corpus validation failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CorpusFailure {
    /// Caller-supplied artifact identity, normally a path.
    pub artifact: String,
    /// Exact APK or App Bundle entry name.
    pub entry: Option<String>,
    /// Zero-based DEX 041 member index.
    pub dex_member: Option<u32>,
    /// Declaring DEX type descriptor.
    pub class: Option<String>,
    /// Overload-qualified method identity.
    pub method: Option<CorpusMethod>,
    /// Byte offset in the most local failing unit.
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
            &self.dex_member,
            &self.class,
            &self.method,
            self.byte_offset,
            self.stage,
            &self.message,
        )
            .cmp(&(
                &other.artifact,
                &other.entry,
                &other.dex_member,
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
    /// Number of physical artifacts visited.
    pub artifacts: usize,
    /// Number of DEX payloads attempted.
    pub dex_files: usize,
    /// Number of DEX 041 logical members visited.
    pub container_members: usize,
    /// Number of class definitions visited.
    pub classes: usize,
    /// Number of methods visited.
    pub methods: usize,
    /// Number of methods with executable code.
    pub code_methods: usize,
    /// Number of decoded instructions and payloads.
    pub instructions: usize,
    /// Number of verified cfglib-backed method graphs.
    pub control_flow_graphs: usize,
    /// Number of completed register analyses.
    pub register_analyses: usize,
    /// Number of Program modules built under each body policy.
    pub program_modules: usize,
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

/// Owned corpus artifact retaining exact physical bytes.
#[derive(Debug, Clone)]
pub struct CorpusArtifact {
    pub(super) name: String,
    pub(super) kind: CorpusArtifactKind,
    pub(super) bytes: Vec<u8>,
}

impl CorpusArtifact {
    /// Creates a standalone DEX artifact.
    #[must_use]
    pub fn dex(name: impl Into<String>, bytes: impl Into<Vec<u8>>) -> Self {
        Self::new(name, CorpusArtifactKind::Dex, bytes)
    }

    /// Creates an APK artifact.
    #[must_use]
    pub fn apk(name: impl Into<String>, bytes: impl Into<Vec<u8>>) -> Self {
        Self::new(name, CorpusArtifactKind::Apk, bytes)
    }

    /// Creates an Android App Bundle artifact.
    #[must_use]
    pub fn aab(name: impl Into<String>, bytes: impl Into<Vec<u8>>) -> Self {
        Self::new(name, CorpusArtifactKind::Aab, bytes)
    }

    fn new(name: impl Into<String>, kind: CorpusArtifactKind, bytes: impl Into<Vec<u8>>) -> Self {
        Self {
            name: name.into(),
            kind,
            bytes: bytes.into(),
        }
    }

    /// Opens an artifact based on its conventional suffix.
    ///
    /// # Errors
    ///
    /// Returns an error for unreadable input or an unsupported suffix.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let bytes = fs::read(path)?;
        let name = path.to_string_lossy().into_owned();
        let extension = path
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or_default();
        if extension.eq_ignore_ascii_case("dex") {
            Ok(Self::dex(name, bytes))
        } else if extension.eq_ignore_ascii_case("apk") {
            Ok(Self::apk(name, bytes))
        } else if extension.eq_ignore_ascii_case("aab") {
            Ok(Self::aab(name, bytes))
        } else {
            Err(Error::InvalidApk(format!(
                "unsupported Android corpus artifact `{}`",
                path.display()
            )))
        }
    }

    /// Returns the caller-visible artifact identity.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the physical artifact kind.
    #[must_use]
    pub const fn kind(&self) -> CorpusArtifactKind {
        self.kind
    }

    /// Returns exact physical input bytes.
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}

/// Mutable collection of artifacts validated in deterministic name order.
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

    /// Adds an already loaded artifact.
    pub fn push(&mut self, artifact: CorpusArtifact) {
        self.artifacts.push(artifact);
    }

    /// Opens and adds an artifact.
    ///
    /// # Errors
    ///
    /// Returns an error if the artifact cannot be read or classified.
    pub fn open(&mut self, path: impl AsRef<Path>) -> Result<()> {
        self.push(CorpusArtifact::open(path)?);
        Ok(())
    }

    /// Returns artifacts in insertion order.
    #[must_use]
    pub fn artifacts(&self) -> &[CorpusArtifact] {
        &self.artifacts
    }

    /// Validates every artifact without stopping at independent member errors.
    #[must_use]
    pub fn validate(&self) -> CorpusReport {
        super::validate::corpus(self)
    }
}

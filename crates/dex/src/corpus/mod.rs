//! Deterministic, non-fail-fast validation for Android bytecode corpora.
//!
//! The validator accepts standalone DEX (including version 041 containers),
//! APK, and Android App Bundle artifacts. Reports retain artifact, archive
//! entry, container member, class, method, and local byte-offset provenance.

mod model;
mod validate;

pub use self::model::{
    Corpus, CorpusArtifact, CorpusArtifactKind, CorpusFailure, CorpusMethod, CorpusReport,
    CorpusStage,
};

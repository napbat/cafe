//! MLIL construction and verification errors.

use std::fmt;

/// One independently actionable MLIL verification failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerificationIssue {
    /// Human-readable description of the violated invariant.
    pub message: String,
}

impl VerificationIssue {
    pub(crate) fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for VerificationIssue {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

/// Complete deterministic result of MLIL verification.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct VerificationReport {
    /// All structural, semantic, typing, and provenance failures.
    pub issues: Vec<VerificationIssue>,
}

impl VerificationReport {
    /// Returns whether the function satisfies every MLIL invariant.
    #[must_use]
    pub fn is_ok(&self) -> bool {
        self.issues.is_empty()
    }

    /// Returns the number of independent failures.
    #[must_use]
    pub fn issue_count(&self) -> usize {
        self.issues.len()
    }
}

impl fmt::Display for VerificationReport {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "MLIL verification failed")?;
        for issue in &self.issues {
            write!(formatter, "; {issue}")?;
        }
        Ok(())
    }
}

impl std::error::Error for VerificationReport {}

/// Error returned while constructing or validating MLIL.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Builder input names an invalid block, variable, or instruction.
    #[error("invalid MLIL construction: {0}")]
    InvalidConstruction(String),
    /// A provenance range is empty, reversed, or outside the source model.
    #[error("invalid MLIL provenance: {0}")]
    InvalidProvenance(String),
    /// The completed function violates one or more MLIL invariants.
    #[error(transparent)]
    Verification(#[from] VerificationReport),
}

/// Result type returned by MLIL APIs.
pub type Result<T> = std::result::Result<T, Error>;

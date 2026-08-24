//! Structured diagnostics for Java-ecosystem bytecode processing.

use std::fmt;

use crate::{AddressRange, FunctionCoordinate};

/// Severity of a processing diagnostic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum DiagnosticLevel {
    /// Processing cannot produce a valid result.
    Error,
    /// Processing can continue, but the result deserves attention.
    Warning,
    /// Additional explanatory information.
    Note,
}

impl fmt::Display for DiagnosticLevel {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Error => "error",
            Self::Warning => "warning",
            Self::Note => "note",
        })
    }
}

/// Format-qualified bytecode region attached to a diagnostic.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DiagnosticLocation {
    /// Function and address coordinate system.
    pub function: FunctionCoordinate,
    /// Half-open native-address range.
    pub range: AddressRange,
}

impl DiagnosticLocation {
    /// Creates a diagnostic location.
    #[must_use]
    pub const fn new(function: FunctionCoordinate, range: AddressRange) -> Self {
        Self { function, range }
    }
}

impl fmt::Display for DiagnosticLocation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{} at {}..{} {:?}",
            self.function, self.range.start, self.range.end, self.function.address_unit
        )
    }
}

/// Secondary explanation optionally anchored to another bytecode region.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DiagnosticNote {
    /// Human-readable explanation.
    pub message: String,
    /// Related region, when one is known.
    pub location: Option<DiagnosticLocation>,
}

/// One structured processing diagnostic.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Diagnostic {
    /// Diagnostic severity.
    pub level: DiagnosticLevel,
    /// Stable consumer-defined identifier, when available.
    pub code: Option<String>,
    /// Primary human-readable explanation.
    pub message: String,
    /// Primary bytecode region, when available.
    pub location: Option<DiagnosticLocation>,
    /// Related explanations in insertion order.
    pub notes: Vec<DiagnosticNote>,
}

impl Diagnostic {
    /// Creates a diagnostic without a code or source location.
    #[must_use]
    pub fn new(level: DiagnosticLevel, message: impl Into<String>) -> Self {
        Self {
            level,
            code: None,
            message: message.into(),
            location: None,
            notes: Vec::new(),
        }
    }

    /// Attaches a stable consumer-defined diagnostic code.
    #[must_use]
    pub fn with_code(mut self, code: impl Into<String>) -> Self {
        self.code = Some(code.into());
        self
    }

    /// Attaches the primary bytecode location.
    #[must_use]
    pub fn at(mut self, location: DiagnosticLocation) -> Self {
        self.location = Some(location);
        self
    }

    /// Appends an unlocated explanatory note.
    #[must_use]
    pub fn with_note(mut self, message: impl Into<String>) -> Self {
        self.notes.push(DiagnosticNote {
            message: message.into(),
            location: None,
        });
        self
    }

    /// Appends an explanatory note anchored to another region.
    #[must_use]
    pub fn with_related(
        mut self,
        message: impl Into<String>,
        location: DiagnosticLocation,
    ) -> Self {
        self.notes.push(DiagnosticNote {
            message: message.into(),
            location: Some(location),
        });
        self
    }
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.level.fmt(formatter)?;
        if let Some(code) = &self.code {
            write!(formatter, "[{code}]")?;
        }
        write!(formatter, ": {}", self.message)?;
        if let Some(location) = &self.location {
            write!(formatter, " ({location})")?;
        }
        for note in &self.notes {
            write!(formatter, "\n  note: {}", note.message)?;
            if let Some(location) = &note.location {
                write!(formatter, " ({location})")?;
            }
        }
        Ok(())
    }
}

/// Ordered collection of diagnostics with severity queries.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Diagnostics {
    values: Vec<Diagnostic>,
}

impl Diagnostics {
    /// Creates an empty diagnostic collection.
    #[must_use]
    pub const fn new() -> Self {
        Self { values: Vec::new() }
    }

    /// Appends one diagnostic.
    pub fn push(&mut self, diagnostic: Diagnostic) {
        self.values.push(diagnostic);
    }

    /// Appends diagnostics while preserving their order.
    pub fn extend(&mut self, diagnostics: impl IntoIterator<Item = Diagnostic>) {
        self.values.extend(diagnostics);
    }

    /// Returns diagnostics in insertion order.
    #[must_use]
    pub fn as_slice(&self) -> &[Diagnostic] {
        &self.values
    }

    /// Iterates over diagnostics in insertion order.
    pub fn iter(&self) -> impl Iterator<Item = &Diagnostic> {
        self.values.iter()
    }

    /// Returns whether at least one error has been recorded.
    #[must_use]
    pub fn has_errors(&self) -> bool {
        self.values
            .iter()
            .any(|diagnostic| diagnostic.level == DiagnosticLevel::Error)
    }

    /// Returns whether the collection is empty.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    /// Returns the number of diagnostics.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.values.len()
    }

    /// Consumes the collection and returns its diagnostics.
    #[must_use]
    pub fn into_vec(self) -> Vec<Diagnostic> {
        self.values
    }
}

impl IntoIterator for Diagnostics {
    type Item = Diagnostic;
    type IntoIter = std::vec::IntoIter<Diagnostic>;

    fn into_iter(self) -> Self::IntoIter {
        self.values.into_iter()
    }
}

//! Structured non-fatal decompilation diagnostics.

/// Severity of a recoverable decompilation problem.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum DiagnosticSeverity {
    /// Output remains meaningful but includes a documented approximation.
    Warning,
    /// One declaration or body was replaced with a conservative source stub.
    Error,
}

/// Stable category of a decompilation diagnostic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum DiagnosticCode {
    /// A JVM method could not be lifted to verified MLIL.
    MlilLiftFailed,
    /// MLIL contains semantics the Java renderer cannot express exactly.
    UnsupportedSemantics,
    /// Source structure required a state-machine representation.
    StateMachineFallback,
    /// A JVM name is not a legal Java source identifier and was escaped.
    EscapedIdentifier,
    /// Class-file declaration metadata has no exact Java syntax.
    DeclarationApproximation,
}

/// Overload-qualified method identity attached to a diagnostic.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct MethodIdentity {
    /// Exact JVM method name.
    pub name: String,
    /// Exact JVM method descriptor.
    pub descriptor: String,
}

impl MethodIdentity {
    /// Creates an overload-qualified method identity.
    #[must_use]
    pub fn new(name: impl Into<String>, descriptor: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            descriptor: descriptor.into(),
        }
    }
}

/// One deterministic, structured source-recovery diagnostic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    /// Stable category.
    pub code: DiagnosticCode,
    /// Impact on the generated source.
    pub severity: DiagnosticSeverity,
    /// Internal JVM class name.
    pub class_name: String,
    /// Method identity when the problem is body-specific.
    pub method: Option<MethodIdentity>,
    /// Human-readable explanation.
    pub message: String,
}

impl Diagnostic {
    pub(crate) fn method_error(
        code: DiagnosticCode,
        class_name: &str,
        method: MethodIdentity,
        message: impl Into<String>,
    ) -> Self {
        Self {
            code,
            severity: DiagnosticSeverity::Error,
            class_name: class_name.to_owned(),
            method: Some(method),
            message: message.into(),
        }
    }

    pub(crate) fn method_warning(
        code: DiagnosticCode,
        class_name: &str,
        method: MethodIdentity,
        message: impl Into<String>,
    ) -> Self {
        Self {
            code,
            severity: DiagnosticSeverity::Warning,
            class_name: class_name.to_owned(),
            method: Some(method),
            message: message.into(),
        }
    }

    pub(crate) fn class_warning(
        code: DiagnosticCode,
        class_name: &str,
        message: impl Into<String>,
    ) -> Self {
        Self {
            code,
            severity: DiagnosticSeverity::Warning,
            class_name: class_name.to_owned(),
            method: None,
            message: message.into(),
        }
    }
}

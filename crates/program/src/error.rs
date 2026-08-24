//! Program construction and mutation errors.

use std::fmt;

use disassembler::BinaryFormat;

/// Required symbol component that was empty.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SymbolComponent {
    /// Module or source artifact name.
    Module,
    /// Owning type name.
    Type,
    /// Field or method name.
    Name,
    /// Field type, method descriptor, or other native signature.
    Signature,
}

impl fmt::Display for SymbolComponent {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Module => "module name",
            Self::Type => "type name",
            Self::Name => "definition name",
            Self::Signature => "definition signature",
        })
    }
}

/// Kind of definition involved in a model invariant violation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DefinitionKind {
    /// Module definition.
    Module,
    /// Type definition.
    Type,
    /// Field definition.
    Field,
    /// Method definition.
    Method,
}

impl fmt::Display for DefinitionKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Module => "module",
            Self::Type => "type",
            Self::Field => "field",
            Self::Method => "method",
        })
    }
}

/// Failure while constructing or editing a program model.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    /// A required native identity component was empty.
    #[error("{kind} {component} cannot be empty")]
    EmptySymbol {
        /// Invalid definition kind.
        kind: DefinitionKind,
        /// Missing component.
        component: SymbolComponent,
    },

    /// A containing definition already owns the same native identity.
    #[error("duplicate {kind} `{name}{signature}` in {format} container `{container}`")]
    DuplicateDefinition {
        /// Source bytecode format.
        format: BinaryFormat,
        /// Native name of the containing module or type.
        container: String,
        /// Duplicate definition kind.
        kind: DefinitionKind,
        /// Native definition name.
        name: String,
        /// Native signature, empty for a type.
        signature: String,
    },

    /// A type from another native format was inserted into a module.
    #[error(
        "cannot insert {type_format} type `{type_name}` into {module_format} module `{module}`"
    )]
    FormatMismatch {
        /// Destination module name.
        module: String,
        /// Destination module format.
        module_format: BinaryFormat,
        /// Type being inserted.
        type_name: String,
        /// Type's native format.
        type_format: BinaryFormat,
    },
}

/// Result returned by program-model operations.
pub type Result<T> = std::result::Result<T, Error>;

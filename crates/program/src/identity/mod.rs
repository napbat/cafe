//! Stable, format-qualified identities for program definitions.

use disassembler::BinaryFormat;

/// Identity of one source module or artifact.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ModuleId {
    /// Native bytecode format.
    pub format: BinaryFormat,
    /// Format-native module or artifact name.
    pub name: String,
}

impl ModuleId {
    /// Creates a module identity.
    #[must_use]
    pub fn new(format: BinaryFormat, name: impl Into<String>) -> Self {
        Self {
            format,
            name: name.into(),
        }
    }
}

/// Format-qualified identity of a type definition or reference.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TypeId {
    /// Native bytecode format.
    pub format: BinaryFormat,
    /// Format-native type name.
    pub name: String,
}

impl TypeId {
    /// Creates a type identity.
    #[must_use]
    pub fn new(format: BinaryFormat, name: impl Into<String>) -> Self {
        Self {
            format,
            name: name.into(),
        }
    }
}

/// Identity of one field within its declaring type.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct FieldId {
    /// Format-native field name.
    pub name: String,
    /// Format-native field type or signature.
    pub signature: String,
}

impl FieldId {
    /// Creates a field identity.
    #[must_use]
    pub fn new(name: impl Into<String>, signature: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            signature: signature.into(),
        }
    }
}

/// Identity of one method overload within its declaring type.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct MethodId {
    /// Format-native method name.
    pub name: String,
    /// Format-native method signature or descriptor.
    pub signature: String,
}

impl MethodId {
    /// Creates a method identity.
    #[must_use]
    pub fn new(name: impl Into<String>, signature: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            signature: signature.into(),
        }
    }
}

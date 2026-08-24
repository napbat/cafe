//! Format-neutral instruction operands and symbolic references.

use std::fmt;

use super::CodeAddress;

/// Integer immediate whose signedness is known.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Immediate {
    /// Signed integer literal.
    Signed(i64),
    /// Unsigned integer literal.
    Unsigned(u64),
}

/// Kind of symbol-table or constant-table reference.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ReferenceKind {
    /// Literal or otherwise unclassified constant.
    Constant,
    /// String constant.
    String,
    /// Type or class reference.
    Type,
    /// Field reference.
    Field,
    /// Direct or virtual method reference.
    Method,
    /// Interface method reference.
    InterfaceMethod,
    /// Dynamically resolved call site.
    DynamicCallSite,
}

/// Indexed native symbol with an optional resolved display value.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Reference {
    /// Semantic category of the reference.
    pub kind: ReferenceKind,
    /// Native table index in the source format.
    pub index: u32,
    /// Human-readable resolution retained alongside the native index.
    pub display: Option<String>,
}

impl Reference {
    /// Creates an unresolved indexed reference.
    #[must_use]
    pub const fn unresolved(kind: ReferenceKind, index: u32) -> Self {
        Self {
            kind,
            index,
            display: None,
        }
    }

    /// Creates an indexed reference with a resolved display value.
    #[must_use]
    pub fn resolved(kind: ReferenceKind, index: u32, display: impl Into<String>) -> Self {
        Self {
            kind,
            index,
            display: Some(display.into()),
        }
    }
}

/// One keyed target in a switch instruction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SwitchCase {
    /// Integer case key.
    pub key: i64,
    /// Absolute target address.
    pub target: CodeAddress,
}

/// Complete switch dispatch table.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SwitchTable {
    /// Target selected when no key matches.
    pub default: CodeAddress,
    /// Explicit keyed targets.
    pub cases: Vec<SwitchCase>,
}

/// One format-neutral instruction operand.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Operand {
    /// Signed or unsigned immediate value.
    Immediate(Immediate),
    /// JVM local-variable slot.
    Local(u32),
    /// DEX virtual register or another register-based format operand.
    Register(u32),
    /// Indexed constant, type, field, method, or call-site reference.
    Reference(Reference),
    /// Direct code target.
    BranchTarget(CodeAddress),
    /// Dense or sparse switch table normalized to explicit cases.
    Switch(SwitchTable),
    /// Native type name not represented by an indexed reference.
    TypeName(String),
    /// Format-owned textual operand that has no shared semantic category.
    Text(String),
}

impl fmt::Display for Operand {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Immediate(Immediate::Signed(value)) => value.fmt(formatter),
            Self::Immediate(Immediate::Unsigned(value)) => value.fmt(formatter),
            Self::Local(index) => write!(formatter, "local[{index}]"),
            Self::Register(index) => write!(formatter, "v{index}"),
            Self::Reference(reference) => {
                write!(formatter, "#{}", reference.index)?;
                if let Some(display) = &reference.display {
                    write!(formatter, " ({display})")?;
                }
                Ok(())
            }
            Self::BranchTarget(target) => target.fmt(formatter),
            Self::Switch(table) => {
                write!(formatter, "default:{} [", table.default)?;
                for (position, case) in table.cases.iter().enumerate() {
                    if position != 0 {
                        formatter.write_str(", ")?;
                    }
                    write!(formatter, "{}:{}", case.key, case.target)?;
                }
                formatter.write_str("]")
            }
            Self::TypeName(name) | Self::Text(name) => name.fmt(formatter),
        }
    }
}

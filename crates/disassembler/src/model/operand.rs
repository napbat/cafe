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
    /// Standalone method-prototype reference.
    MethodPrototype,
    /// Method-handle reference.
    MethodHandle,
    /// Dynamically resolved call site.
    DynamicCallSite,
}

/// Exact Java text retained as UTF-16 plus a convenient lossy Rust view.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ExactText {
    /// Lossy Unicode view; unpaired surrogates appear as U+FFFD.
    pub text: String,
    /// Exact Java UTF-16 code units.
    pub utf16_units: Vec<u16>,
}

impl ExactText {
    /// Creates exact text from valid Unicode.
    #[must_use]
    pub fn new(value: impl Into<String>) -> Self {
        let text = value.into();
        let utf16_units = text.encode_utf16().collect();
        Self { text, utf16_units }
    }

    /// Creates exact text from arbitrary Java UTF-16 code units.
    #[must_use]
    pub fn from_utf16(utf16_units: Vec<u16>) -> Self {
        Self {
            text: String::from_utf16_lossy(&utf16_units),
            utf16_units,
        }
    }
}

/// Reconstructable symbolic value selected by an indexed native operand.
///
/// This is optional because some VM tables, notably bootstrap call sites and
/// method handles, require recursive artifact metadata that does not fit one
/// instruction operand. Emitters reject such references unless the caller
/// supplies a richer format-specific resolver.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ReferenceSymbol {
    /// Signed JVM integer constant.
    Integer(i32),
    /// IEEE-754 single-precision constant bits.
    Float(u32),
    /// Signed JVM long constant.
    Long(i64),
    /// IEEE-754 double-precision constant bits.
    Double(u64),
    /// Exact Java string contents.
    String(ExactText),
    /// JVM internal name, array descriptor, or DEX type descriptor.
    Type(String),
    /// Overload-qualified field identity.
    Field {
        /// Format-native declaring type name.
        owner: String,
        /// Exact field name.
        name: ExactText,
        /// JVM-compatible field descriptor.
        descriptor: String,
    },
    /// Overload-qualified method identity.
    Method {
        /// Format-native declaring type name.
        owner: String,
        /// Exact method name.
        name: ExactText,
        /// JVM-compatible method descriptor.
        descriptor: String,
    },
    /// Standalone JVM-compatible method descriptor.
    MethodPrototype(String),
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
    /// Structured symbolic value when one instruction operand is sufficient.
    pub symbol: Option<ReferenceSymbol>,
}

impl Reference {
    /// Creates an unresolved indexed reference.
    #[must_use]
    pub const fn unresolved(kind: ReferenceKind, index: u32) -> Self {
        Self {
            kind,
            index,
            display: None,
            symbol: None,
        }
    }

    /// Creates an indexed reference with a resolved display value.
    #[must_use]
    pub fn resolved(kind: ReferenceKind, index: u32, display: impl Into<String>) -> Self {
        Self {
            kind,
            index,
            display: Some(display.into()),
            symbol: None,
        }
    }

    /// Attaches a reconstructable symbolic value while retaining the native index.
    #[must_use]
    pub fn with_symbol(mut self, symbol: ReferenceSymbol) -> Self {
        self.symbol = Some(symbol);
        self
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
    /// Contiguous range of registers in a register-based format.
    RegisterRange {
        /// First register in the range.
        start: u32,
        /// Number of registers in the range.
        count: u32,
    },
    /// Indexed constant, type, field, method, or call-site reference.
    Reference(Reference),
    /// Direct code target.
    BranchTarget(CodeAddress),
    /// Dense or sparse switch table normalized to explicit cases.
    Switch(SwitchTable),
    /// Native type name not represented by an indexed reference.
    TypeName(String),
    /// Opaque inline data owned by an instruction-stream payload.
    Data(Vec<u8>),
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
            Self::RegisterRange { start, count } => {
                write!(formatter, "v{start}..v{}", start.saturating_add(*count))
            }
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
            Self::Data(bytes) => {
                formatter.write_str("[")?;
                for (position, byte) in bytes.iter().enumerate() {
                    if position != 0 {
                        formatter.write_str(" ")?;
                    }
                    write!(formatter, "{byte:02x}")?;
                }
                formatter.write_str("]")
            }
        }
    }
}

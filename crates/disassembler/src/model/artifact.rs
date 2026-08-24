//! Disassembled artifacts, functions, bodies, and exception metadata.

use std::fmt;

use super::{AddressRange, AddressUnit, CodeAddress, Instruction};

/// Native bytecode format from which a disassembly was lowered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum BinaryFormat {
    /// JVM `ClassFile` bytecode.
    JavaClass,
    /// Android Dalvik Executable bytecode.
    Dex,
}

impl fmt::Display for BinaryFormat {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::JavaClass => "java-class",
            Self::Dex => "dex",
        })
    }
}

/// Native access-flag bits retained without assigning cross-format semantics.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RawAccessFlags(u32);

impl RawAccessFlags {
    /// Creates a raw access-flag value.
    #[must_use]
    pub const fn new(bits: u32) -> Self {
        Self(bits)
    }

    /// Returns the source format's unmodified flag bits.
    #[must_use]
    pub const fn bits(self) -> u32 {
        self.0
    }
}

/// Fully qualified identity of a disassembled function or method.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct FunctionSymbol {
    /// Format-native owning type or namespace.
    pub owner: String,
    /// Native function or method name.
    pub name: String,
    /// Native signature or descriptor.
    pub signature: String,
}

/// Catch target classification retained outside cfglib's structural graph.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum CatchType {
    /// Handler catches every thrown value.
    Any,
    /// Handler catches one named type.
    Type(String),
}

/// Exception handler and the exact half-open range it protects.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ExceptionHandler {
    /// Protected instruction range.
    pub protected: AddressRange,
    /// Entry address of the handler.
    pub handler: CodeAddress,
    /// Catch-all or resolved catch type.
    pub catch: CatchType,
}

/// Decoded instructions and metadata for one function body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FunctionBody {
    /// Unit used by every address and size in this body.
    pub address_unit: AddressUnit,
    /// Instructions in native address order.
    pub instructions: Vec<Instruction>,
    /// Exception handlers attached to this body.
    pub exception_handlers: Vec<ExceptionHandler>,
}

impl FunctionBody {
    /// Builds a function body from ordered instructions and exception metadata.
    #[must_use]
    pub const fn new(
        address_unit: AddressUnit,
        instructions: Vec<Instruction>,
        exception_handlers: Vec<ExceptionHandler>,
    ) -> Self {
        Self {
            address_unit,
            instructions,
            exception_handlers,
        }
    }
}

/// One disassembled function, including declarations without executable code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Function {
    /// Stable native identity.
    pub symbol: FunctionSymbol,
    /// Unmodified source-format access flags.
    pub access_flags: RawAccessFlags,
    /// Executable body, absent for abstract or native declarations.
    pub body: Option<FunctionBody>,
}

/// Format-neutral result produced from one source artifact.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Disassembly {
    /// Source bytecode format.
    pub format: BinaryFormat,
    /// Format-native artifact name.
    pub name: String,
    /// Functions in native declaration order.
    pub functions: Vec<Function>,
}

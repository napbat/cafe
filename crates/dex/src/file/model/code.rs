//! DEX code items, class data, and exception handlers.

use crate::instruction::Instruction;

use super::{DebugInfo, FieldIndex, MethodIndex, TypeIndex};

/// One DEX `code_item` with decoded instructions and metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodeItem {
    /// Number of registers in the frame.
    pub registers_size: u16,
    /// Incoming argument width in register words.
    pub ins_size: u16,
    /// Outgoing argument width in register words.
    pub outs_size: u16,
    /// Decoded instructions and payload pseudo-instructions.
    pub instructions: Vec<Instruction>,
    /// Protected regions and typed handlers.
    pub tries: Vec<TryBlock>,
    /// Optional debugging state-machine data.
    pub debug_info: Option<DebugInfo>,
    /// Original absolute `code_item` offset.
    pub data_offset: u32,
}

/// One protected DEX code-unit range.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TryBlock {
    /// Inclusive start in 16-bit code units.
    pub start_address: u32,
    /// Protected length in 16-bit code units.
    pub instruction_count: u16,
    /// Ordered typed handlers and optional catch-all.
    pub handlers: Vec<CatchHandler>,
}

/// One typed or catch-all exception target.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CatchHandler {
    /// Caught type, or `None` for the catch-all handler.
    pub exception_type: Option<TypeIndex>,
    /// Handler entry in 16-bit code units.
    pub address: u32,
}

/// One field declaration encoded in class data.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct EncodedField {
    /// Absolute field identifier after decoding the delta encoding.
    pub field: FieldIndex,
    /// Unmodified DEX access-flag bits.
    pub access_flags: AccessFlags,
}

/// One method declaration encoded in class data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EncodedMethod {
    /// Absolute method identifier after decoding the delta encoding.
    pub method: MethodIndex,
    /// Unmodified DEX access-flag bits.
    pub access_flags: AccessFlags,
    /// Executable code, absent for abstract or native declarations.
    pub code: Option<CodeItem>,
}

/// Decoded direct/static and virtual/instance class members.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ClassData {
    /// Static fields in increasing field-index order.
    pub static_fields: Vec<EncodedField>,
    /// Instance fields in increasing field-index order.
    pub instance_fields: Vec<EncodedField>,
    /// Direct methods in increasing method-index order.
    pub direct_methods: Vec<EncodedMethod>,
    /// Virtual methods in increasing method-index order.
    pub virtual_methods: Vec<EncodedMethod>,
    /// Original absolute `class_data_item` offset.
    pub data_offset: u32,
}

/// Raw DEX declaration-access flags.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct AccessFlags(u32);

impl AccessFlags {
    /// Public declaration.
    pub const PUBLIC: Self = Self(0x0000_0001);
    /// Private declaration.
    pub const PRIVATE: Self = Self(0x0000_0002);
    /// Protected declaration.
    pub const PROTECTED: Self = Self(0x0000_0004);
    /// Static member.
    pub const STATIC: Self = Self(0x0000_0008);
    /// Final declaration.
    pub const FINAL: Self = Self(0x0000_0010);
    /// Synchronized method.
    pub const SYNCHRONIZED: Self = Self(0x0000_0020);
    /// Volatile field or bridge method.
    pub const VOLATILE_OR_BRIDGE: Self = Self(0x0000_0040);
    /// Transient field or varargs method.
    pub const TRANSIENT_OR_VARARGS: Self = Self(0x0000_0080);
    /// Native method.
    pub const NATIVE: Self = Self(0x0000_0100);
    /// Interface declaration.
    pub const INTERFACE: Self = Self(0x0000_0200);
    /// Abstract declaration.
    pub const ABSTRACT: Self = Self(0x0000_0400);
    /// Strict floating-point method.
    pub const STRICT: Self = Self(0x0000_0800);
    /// Synthetic declaration.
    pub const SYNTHETIC: Self = Self(0x0000_1000);
    /// Annotation declaration.
    pub const ANNOTATION: Self = Self(0x0000_2000);
    /// Enum declaration or field.
    pub const ENUM: Self = Self(0x0000_4000);
    /// Constructor or class initializer.
    pub const CONSTRUCTOR: Self = Self(0x0001_0000);
    /// Declared synchronized method.
    pub const DECLARED_SYNCHRONIZED: Self = Self(0x0002_0000);

    /// Retains any raw flag combination.
    #[must_use]
    pub const fn from_bits_retain(bits: u32) -> Self {
        Self(bits)
    }

    /// Returns the exact encoded bits.
    #[must_use]
    pub const fn bits(self) -> u32 {
        self.0
    }

    /// Returns whether all supplied flags are set.
    #[must_use]
    pub const fn contains(self, flags: Self) -> bool {
        self.0 & flags.0 == flags.0
    }
}

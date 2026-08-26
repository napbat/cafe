//! MLIL value types and abstract variable identities.

use disassembler::{BinaryFormat, CodeAddress};

/// Native allocation identity retained for uninitialized references.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct AllocationSite {
    /// Source bytecode family.
    pub format: BinaryFormat,
    /// Native allocation instruction address.
    pub address: CodeAddress,
}

/// Type of a value used or defined by one MLIL instruction.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ValueType {
    /// Type has not yet been constrained.
    Unknown,
    /// Incompatible native predecessor types reached this value.
    Conflict,
    /// Boolean predicate produced by semantic comparison.
    Boolean,
    /// Java computational integer value.
    Integer,
    /// Signed 64-bit integer.
    Long,
    /// IEEE-754 single-precision value.
    Float,
    /// IEEE-754 double-precision value.
    Double,
    /// Unclassified 32-bit integer or float bit pattern.
    Bits32,
    /// Dalvik's exact 32-bit zero bit pattern, usable as numeric zero or null.
    Zero,
    /// Ambiguous 64-bit long/double bit pattern.
    Bits64,
    /// Null reference.
    Null,
    /// Initialized object or array reference, optionally with an exact descriptor.
    Reference(Option<String>),
    /// Incoming constructor receiver, named by its object descriptor, before initialization.
    UninitializedThis(String),
    /// Allocation result before its matching constructor completes.
    Uninitialized {
        /// Exact object descriptor selected by the allocation.
        descriptor: String,
        /// Native instruction distinguishing this allocation.
        site: AllocationSite,
    },
    /// Legacy JVM subroutine return address.
    ReturnAddress,
}

impl ValueType {
    /// Returns whether this is an initialized, null, or uninitialized reference.
    #[must_use]
    pub const fn is_reference(&self) -> bool {
        matches!(
            self,
            Self::Null
                | Self::Reference(_)
                | Self::UninitializedThis(_)
                | Self::Uninitialized { .. }
        )
    }

    /// Returns whether `actual` can conservatively satisfy this expected type.
    #[must_use]
    pub fn accepts(&self, actual: &Self) -> bool {
        if self == actual || matches!(self, Self::Unknown) || matches!(actual, Self::Unknown) {
            return true;
        }
        match (self, actual) {
            (
                Self::Bits32,
                Self::Boolean | Self::Integer | Self::Float | Self::Zero | Self::Null,
            )
            | (Self::Boolean | Self::Integer | Self::Float, Self::Bits32 | Self::Zero)
            | (Self::Bits64, Self::Long | Self::Double)
            | (Self::Long | Self::Double, Self::Bits64)
            | (Self::Null | Self::Reference(_), Self::Zero)
            | (Self::Reference(_), Self::Null)
            | (Self::Reference(None), Self::Reference(_)) => true,
            (Self::Reference(Some(expected)), Self::Reference(Some(actual))) => expected == actual,
            _ => false,
        }
    }
}

/// Semantic purpose of one mutable MLIL variable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum VariableRole {
    /// Incoming method parameter in declaration order.
    Parameter(u16),
    /// Mutable method-local state.
    Local,
    /// Compiler-generated intermediate value.
    Temporary,
    /// Boolean condition used by control flow.
    Condition,
    /// Current caught exception.
    Exception,
}

/// Format-qualified native storage from which an MLIL variable was derived.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum SourceStorage {
    /// JVM local-variable slot.
    JvmLocal(u16),
    /// JVM operand-stack position counted from the bottom.
    JvmStack(u16),
    /// Dalvik virtual register.
    DexRegister(u16),
    /// Dalvik implicit invocation or filled-array result slot.
    DexResult,
    /// Dalvik exception object delivered before `move-exception`.
    DexException,
}

/// Native variable provenance retained independently of MLIL semantics.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct NativeVariable {
    /// Source format.
    pub format: BinaryFormat,
    /// Native storage location.
    pub storage: SourceStorage,
}

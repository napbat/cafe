//! Public register-frame lattice and fixed-point result.

use std::collections::BTreeMap;

use super::super::ControlFlow;

/// Reference value tracked by Dalvik register analysis.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ReferenceType {
    /// Initialized reference whose precise descriptor is unavailable.
    Any,
    /// Initialized reference with a class or array descriptor.
    Descriptor(String),
    /// Allocation result awaiting its matching constructor invocation.
    Uninitialized {
        /// Allocated class descriptor.
        descriptor: String,
        /// `new-instance` operation offset distinguishing allocation sites.
        allocation_offset: u32,
    },
    /// Incoming receiver of a constructor before superclass/peer initialization.
    UninitializedThis {
        /// Declaring class descriptor.
        descriptor: String,
    },
}

impl ReferenceType {
    /// Returns whether this value has completed constructor initialization.
    #[must_use]
    pub const fn is_initialized(&self) -> bool {
        matches!(self, Self::Any | Self::Descriptor(_))
    }
}

/// Abstract value occupying one Dalvik register position.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum RegisterType {
    /// Register has no value on this path.
    Unknown,
    /// Incompatible predecessor states merged here.
    Conflict,
    /// Zero bits, usable as numeric zero or a null reference.
    Zero,
    /// Ambiguous nonzero 32-bit integer/float bit pattern.
    Single,
    /// Integer-like value.
    Integer,
    /// IEEE-754 single-precision value.
    Float,
    /// Zero 64-bit pattern, usable as long or double zero.
    WideZero,
    /// Ambiguous 64-bit long/double bit pattern.
    Wide,
    /// Signed 64-bit integer value.
    Long,
    /// IEEE-754 double-precision value.
    Double,
    /// Object, array, null, or uninitialized reference.
    Reference(ReferenceType),
    /// High register word belonging to the preceding wide value.
    WideContinuation,
}

impl RegisterType {
    pub(super) const fn is_wide_base(&self) -> bool {
        matches!(
            self,
            Self::WideZero | Self::Wide | Self::Long | Self::Double
        )
    }
}

/// Complete abstract register state at an operation boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegisterFrame {
    pub(super) registers: Vec<RegisterType>,
}

impl RegisterFrame {
    /// Returns register positions from `v0` upward.
    #[must_use]
    pub fn registers(&self) -> &[RegisterType] {
        &self.registers
    }

    /// Returns the abstract value beginning at one register position.
    #[must_use]
    pub fn register(&self, index: u16) -> Option<&RegisterType> {
        self.registers.get(usize::from(index))
    }
}

/// Fixed-point register states and the typed control flow used to derive them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegisterAnalysis {
    pub(super) flow: ControlFlow,
    pub(super) entries: BTreeMap<u32, RegisterFrame>,
    pub(super) exits: BTreeMap<u32, RegisterFrame>,
}

impl RegisterAnalysis {
    /// Returns the exception-aware operation graph.
    #[must_use]
    pub const fn flow(&self) -> &ControlFlow {
        &self.flow
    }

    /// Returns the merged register state before an operation.
    #[must_use]
    pub fn entry_frame(&self, offset: u32) -> Option<&RegisterFrame> {
        self.entries.get(&offset)
    }

    /// Returns the register state after normal completion of an operation.
    #[must_use]
    pub fn exit_frame(&self, offset: u32) -> Option<&RegisterFrame> {
        self.exits.get(&offset)
    }

    /// Iterates over reachable operation entry states in address order.
    pub fn entry_frames(&self) -> impl Iterator<Item = (u32, &RegisterFrame)> {
        self.entries.iter().map(|(&offset, frame)| (offset, frame))
    }
}

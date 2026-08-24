//! Typed instruction addresses, sizes, ranges, and address units.

use std::fmt;

/// Unit used by instruction addresses and sizes within one function body.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum AddressUnit {
    /// Eight-bit bytes, as used by JVM method bytecode.
    Byte,
    /// Sixteen-bit code units, as used by DEX instructions.
    CodeUnit16,
}

/// Offset of an instruction from the beginning of its function body.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct CodeAddress(u64);

impl CodeAddress {
    /// The address at the beginning of a function body.
    pub const ZERO: Self = Self(0);

    /// Creates an address from its value in the body's [`AddressUnit`].
    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    /// Returns the address value in the body's [`AddressUnit`].
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }

    /// Adds an encoded instruction size, returning `None` on overflow.
    #[must_use]
    pub fn checked_add(self, size: CodeSize) -> Option<Self> {
        self.0.checked_add(u64::from(size.get())).map(Self)
    }
}

impl From<u16> for CodeAddress {
    fn from(value: u16) -> Self {
        Self(u64::from(value))
    }
}

impl From<u32> for CodeAddress {
    fn from(value: u32) -> Self {
        Self(u64::from(value))
    }
}

impl fmt::Display for CodeAddress {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "0x{:x}", self.0)
    }
}

/// Encoded width of one instruction in the body's [`AddressUnit`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct CodeSize(u32);

impl CodeSize {
    /// Creates an instruction size.
    #[must_use]
    pub const fn new(value: u32) -> Self {
        Self(value)
    }

    /// Returns the encoded size in the body's [`AddressUnit`].
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }

    /// Returns whether this size has no encoded units.
    #[must_use]
    pub const fn is_zero(self) -> bool {
        self.0 == 0
    }
}

impl From<u16> for CodeSize {
    fn from(value: u16) -> Self {
        Self(u32::from(value))
    }
}

impl From<u32> for CodeSize {
    fn from(value: u32) -> Self {
        Self(value)
    }
}

impl fmt::Display for CodeSize {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

/// Half-open address range `[start, end)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct AddressRange {
    /// Inclusive first protected address.
    pub start: CodeAddress,
    /// Exclusive end address.
    pub end: CodeAddress,
}

impl AddressRange {
    /// Creates a half-open address range.
    #[must_use]
    pub const fn new(start: CodeAddress, end: CodeAddress) -> Self {
        Self { start, end }
    }

    /// Returns whether the range contains an address.
    #[must_use]
    pub const fn contains(self, address: CodeAddress) -> bool {
        address.get() >= self.start.get() && address.get() < self.end.get()
    }

    /// Returns whether the range is empty or reversed.
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.start.get() >= self.end.get()
    }
}

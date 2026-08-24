//! Typed bit geometry and payload identifiers for Dalvik instructions.

pub(super) const NIBBLE_MASK: u8 = 0x0f;
pub(super) const NIBBLE_BITS: u32 = 4;
pub(super) const BYTE_BITS: u32 = 8;
pub(super) const CODE_UNIT_BITS: u32 = 16;
pub(super) const DOUBLE_CODE_UNIT_BITS: u32 = 32;
pub(super) const TRIPLE_CODE_UNIT_BITS: u32 = 48;
pub(super) const BYTES_PER_CODE_UNIT: usize = 2;
pub(super) const BYTES_PER_CODE_UNIT_U32: u32 = 2;
pub(super) const CODE_UNITS_PER_WORD: u32 = 2;
pub(super) const MAX_REGISTER_LIST_COUNT: u8 = 5;
pub(super) const REGISTER_LIST_SLOTS: usize = 5;
pub(super) const FIRST_OPERAND_WORD: usize = 1;
pub(super) const SECOND_OPERAND_WORD: usize = 2;
pub(super) const THIRD_OPERAND_WORD: usize = 3;
pub(super) const FOURTH_OPERAND_WORD: usize = 4;
pub(super) const LOW_BYTE_INDEX: usize = 0;
pub(super) const HIGH_BYTE_INDEX: usize = 1;
pub(super) const ARRAY_PADDING_VALUE: u8 = 0;
pub(super) const RESERVED_BYTE_VALUE: u8 = 0;
pub(super) const EMPTY_REGISTER_COUNT: u8 = 0;
pub(super) const NON_EMPTY_RANGE_LAST_DELTA: u16 = 1;
pub(super) const SIGNED_NIBBLE_MINIMUM: i64 = -8;
pub(super) const SIGNED_NIBBLE_MAXIMUM: i64 = 7;
pub(super) const FIRST_CODE_UNIT_OFFSET: u32 = 0;
pub(super) const INVALID_ARRAY_ELEMENT_WIDTH: u16 = 0;
pub(super) const CLEARED_LOW_BITS: i64 = 0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RegisterListSlot {
    C,
    D,
    E,
    F,
    G,
}

impl RegisterListSlot {
    pub(super) const fn index(self) -> usize {
        match self {
            Self::C => 0,
            Self::D => 1,
            Self::E => 2,
            Self::F => 3,
            Self::G => 4,
        }
    }
}

pub(super) const PACKED_SWITCH_HEADER_CODE_UNITS: usize = 4;
pub(super) const PACKED_SWITCH_TARGET_CODE_UNITS: usize = 2;
pub(super) const SPARSE_SWITCH_HEADER_CODE_UNITS: usize = 2;
pub(super) const SPARSE_SWITCH_ENTRY_CODE_UNITS: usize = 4;
pub(super) const ARRAY_DATA_HEADER_CODE_UNITS: usize = 4;
pub(super) const PACKED_SWITCH_HEADER_CODE_UNITS_U32: u32 = 4;
pub(super) const PACKED_SWITCH_TARGET_CODE_UNITS_U32: u32 = 2;
pub(super) const SPARSE_SWITCH_HEADER_CODE_UNITS_U32: u32 = 2;
pub(super) const SPARSE_SWITCH_ENTRY_CODE_UNITS_U32: u32 = 4;
pub(super) const ARRAY_DATA_HEADER_CODE_UNITS_U32: u32 = 4;
pub(super) const ALIGNMENT_ROUNDING_BIAS_U32: u32 = BYTES_PER_CODE_UNIT_U32 - 1;

/// Identifying pseudo-opcode stored at the start of a payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u16)]
pub enum PayloadKind {
    /// Packed-switch payload identifier.
    PackedSwitch = 0x0100,
    /// Sparse-switch payload identifier.
    SparseSwitch = 0x0200,
    /// Array-data payload identifier.
    ArrayData = 0x0300,
}

impl PayloadKind {
    /// Parses the complete 16-bit payload identifier.
    #[must_use]
    pub const fn from_identifier(value: u16) -> Option<Self> {
        match value {
            0x0100 => Some(Self::PackedSwitch),
            0x0200 => Some(Self::SparseSwitch),
            0x0300 => Some(Self::ArrayData),
            _ => None,
        }
    }

    /// Returns the exact 16-bit identifier stored in the instruction stream.
    #[must_use]
    pub const fn identifier(self) -> u16 {
        self as u16
    }

    /// Returns the conventional pseudo-instruction mnemonic.
    #[must_use]
    pub const fn mnemonic(self) -> &'static str {
        match self {
            Self::PackedSwitch => "packed-switch-payload",
            Self::SparseSwitch => "sparse-switch-payload",
            Self::ArrayData => "array-data-payload",
        }
    }
}

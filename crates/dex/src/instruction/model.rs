//! Owned instruction and payload values.

use super::Opcode;

/// One decoded item in a DEX code-unit stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Instruction {
    offset: u32,
    data: InstructionData,
}

impl Instruction {
    /// Creates an ordinary instruction at a code-unit offset.
    #[must_use]
    pub const fn operation(offset: u32, opcode: Opcode, operands: Operands) -> Self {
        Self {
            offset,
            data: InstructionData::Operation { opcode, operands },
        }
    }

    /// Creates a packed-switch payload at a code-unit offset.
    #[must_use]
    pub const fn packed_switch(offset: u32, payload: PackedSwitchPayload) -> Self {
        Self {
            offset,
            data: InstructionData::PackedSwitchPayload(payload),
        }
    }

    /// Creates a sparse-switch payload at a code-unit offset.
    #[must_use]
    pub const fn sparse_switch(offset: u32, payload: SparseSwitchPayload) -> Self {
        Self {
            offset,
            data: InstructionData::SparseSwitchPayload(payload),
        }
    }

    /// Creates an array-data payload at a code-unit offset.
    #[must_use]
    pub const fn array_data(offset: u32, payload: ArrayDataPayload) -> Self {
        Self {
            offset,
            data: InstructionData::ArrayDataPayload(payload),
        }
    }

    /// Returns the start address in 16-bit code units.
    #[must_use]
    pub const fn offset(&self) -> u32 {
        self.offset
    }

    /// Replaces the start address in 16-bit code units.
    ///
    /// This is useful while laying out an edited method. Branch operands remain
    /// absolute and must still point at the intended instruction afterward.
    pub const fn set_offset(&mut self, offset: u32) {
        self.offset = offset;
    }

    /// Returns the typed operation or payload.
    #[must_use]
    pub const fn data(&self) -> &InstructionData {
        &self.data
    }

    /// Returns the mutable typed operation or payload.
    #[must_use]
    pub const fn data_mut(&mut self) -> &mut InstructionData {
        &mut self.data
    }

    /// Returns this item's encoded width in 16-bit code units.
    ///
    /// Returns `None` only if a variable-sized payload overflows DEX's address
    /// space. The encoder reports that condition as an assembly error.
    #[must_use]
    pub fn code_units(&self) -> Option<u32> {
        self.data.code_units()
    }
}

/// Typed contents of one instruction-stream item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstructionData {
    /// An executable Dalvik opcode and its operands.
    Operation {
        /// Standard opcode.
        opcode: Opcode,
        /// Operands appropriate for the opcode's format.
        operands: Operands,
    },
    /// A packed-switch payload referenced by `packed-switch`.
    PackedSwitchPayload(PackedSwitchPayload),
    /// A sparse-switch payload referenced by `sparse-switch`.
    SparseSwitchPayload(SparseSwitchPayload),
    /// An array-data payload referenced by `fill-array-data`.
    ArrayDataPayload(ArrayDataPayload),
}

impl InstructionData {
    /// Returns the encoded width in 16-bit code units.
    #[must_use]
    pub fn code_units(&self) -> Option<u32> {
        match self {
            Self::Operation { opcode, .. } => Some(opcode.format().code_units()),
            Self::PackedSwitchPayload(payload) => payload.code_units(),
            Self::SparseSwitchPayload(payload) => payload.code_units(),
            Self::ArrayDataPayload(payload) => payload.code_units(),
        }
    }

    /// Returns the opcode for an executable operation.
    #[must_use]
    pub const fn opcode(&self) -> Option<Opcode> {
        match self {
            Self::Operation { opcode, .. } => Some(*opcode),
            Self::PackedSwitchPayload(_)
            | Self::SparseSwitchPayload(_)
            | Self::ArrayDataPayload(_) => None,
        }
    }

    /// Returns the operands for an executable operation.
    #[must_use]
    pub const fn operands(&self) -> Option<&Operands> {
        match self {
            Self::Operation { operands, .. } => Some(operands),
            Self::PackedSwitchPayload(_)
            | Self::SparseSwitchPayload(_)
            | Self::ArrayDataPayload(_) => None,
        }
    }
}

/// Operands shared by the standard Dalvik binary formats.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Operands {
    /// No operands.
    None,
    /// One register.
    Register(u16),
    /// Two registers.
    Registers {
        /// First register.
        first: u16,
        /// Second register.
        second: u16,
    },
    /// Three registers.
    ThreeRegisters {
        /// First register.
        first: u16,
        /// Second register.
        second: u16,
        /// Third register.
        third: u16,
    },
    /// One register and a sign-extended literal.
    RegisterLiteral {
        /// Register operand.
        register: u16,
        /// Literal at its semantic width.
        literal: i64,
    },
    /// Two registers and a sign-extended literal.
    RegistersLiteral {
        /// First register.
        first: u16,
        /// Second register.
        second: u16,
        /// Literal at its semantic width.
        literal: i64,
    },
    /// Absolute code-unit branch target.
    Branch {
        /// Target instruction address.
        target: u32,
    },
    /// One register and an absolute code-unit branch target.
    RegisterBranch {
        /// Register operand.
        register: u16,
        /// Target instruction or payload address.
        target: u32,
    },
    /// Two registers and an absolute code-unit branch target.
    RegistersBranch {
        /// First register.
        first: u16,
        /// Second register.
        second: u16,
        /// Target instruction address.
        target: u32,
    },
    /// One register and an identifier-table index.
    RegisterIndex {
        /// Register operand.
        register: u16,
        /// Native DEX table index.
        index: u32,
    },
    /// Two registers and an identifier-table index.
    RegistersIndex {
        /// First register.
        first: u16,
        /// Second register.
        second: u16,
        /// Native DEX table index.
        index: u32,
    },
    /// A register list and one or two identifier-table indices.
    RegisterListIndex {
        /// Registers in invocation order.
        registers: Vec<u16>,
        /// Primary type, method, or call-site index.
        index: u32,
        /// Prototype index used by polymorphic invocation.
        secondary_index: Option<u32>,
    },
    /// A contiguous register range and one or two identifier-table indices.
    RegisterRangeIndex {
        /// First register in the range.
        start: u16,
        /// Number of register words.
        count: u8,
        /// Primary type, method, or call-site index.
        index: u32,
        /// Prototype index used by polymorphic invocation.
        secondary_index: Option<u32>,
    },
}

/// Data table selected by a packed-switch instruction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackedSwitchPayload {
    /// Key corresponding to the first target; later keys are consecutive.
    pub first_key: i32,
    /// Absolute code-unit targets in increasing-key order.
    pub targets: Vec<u32>,
}

impl PackedSwitchPayload {
    /// Returns the encoded payload width in code units.
    #[must_use]
    pub fn code_units(&self) -> Option<u32> {
        let count = u32::try_from(self.targets.len()).ok()?;
        count.checked_mul(2)?.checked_add(4)
    }
}

/// Data table selected by a sparse-switch instruction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SparseSwitchPayload {
    /// Strictly increasing switch keys.
    pub keys: Vec<i32>,
    /// Absolute code-unit targets corresponding one-to-one with `keys`.
    pub targets: Vec<u32>,
}

impl SparseSwitchPayload {
    /// Returns the encoded payload width in code units.
    #[must_use]
    pub fn code_units(&self) -> Option<u32> {
        let count = u32::try_from(self.keys.len()).ok()?;
        count.checked_mul(4)?.checked_add(2)
    }
}

/// Raw element bytes selected by a fill-array-data instruction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArrayDataPayload {
    /// Width of each element in bytes.
    pub element_width: u16,
    /// Number of elements represented by `data`.
    pub element_count: u32,
    /// Concatenated little-endian element bytes without alignment padding.
    pub data: Vec<u8>,
}

impl ArrayDataPayload {
    /// Returns the encoded payload width in code units.
    #[must_use]
    pub fn code_units(&self) -> Option<u32> {
        let byte_count = u32::try_from(self.data.len()).ok()?;
        byte_count.checked_add(1)?.checked_div(2)?.checked_add(4)
    }
}

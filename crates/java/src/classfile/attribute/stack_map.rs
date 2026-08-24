//! Typed stack-map frames and verification types.

/// Typed `StackMapTable` attribute.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StackMapTableAttribute {
    /// Constant-pool index of `StackMapTable`.
    pub name_index: u16,
    /// Frames in increasing bytecode-offset order.
    pub frames: Vec<StackMapFrame>,
}

/// One stack-map frame, preserving its compact encoding category.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StackMapFrame {
    /// `same_frame` with an implicit offset delta from 0 through 63.
    Same {
        /// Offset delta relative to the preceding frame.
        offset_delta: u8,
    },
    /// `same_locals_1_stack_item_frame` with an implicit offset delta.
    SameLocalsOneStack {
        /// Offset delta relative to the preceding frame.
        offset_delta: u8,
        /// Sole operand-stack value.
        stack: VerificationType,
    },
    /// `same_locals_1_stack_item_frame_extended`.
    SameLocalsOneStackExtended {
        /// Offset delta relative to the preceding frame.
        offset_delta: u16,
        /// Sole operand-stack value.
        stack: VerificationType,
    },
    /// `chop_frame`, removing one through three trailing locals.
    Chop {
        /// Offset delta relative to the preceding frame.
        offset_delta: u16,
        /// Number of trailing locals removed, from one through three.
        absent_locals: u8,
    },
    /// `same_frame_extended`.
    SameExtended {
        /// Offset delta relative to the preceding frame.
        offset_delta: u16,
    },
    /// `append_frame`, adding one through three locals.
    Append {
        /// Offset delta relative to the preceding frame.
        offset_delta: u16,
        /// Appended locals, from one through three entries.
        locals: Vec<VerificationType>,
    },
    /// `full_frame` with complete locals and operand stack.
    Full {
        /// Offset delta relative to the preceding frame.
        offset_delta: u16,
        /// Complete local-variable state.
        locals: Vec<VerificationType>,
        /// Complete operand-stack state.
        stack: Vec<VerificationType>,
    },
}

impl StackMapFrame {
    /// Returns this frame's encoded offset delta.
    #[must_use]
    pub fn offset_delta(&self) -> u16 {
        match self {
            Self::Same { offset_delta } | Self::SameLocalsOneStack { offset_delta, .. } => {
                u16::from(*offset_delta)
            }
            Self::SameLocalsOneStackExtended { offset_delta, .. }
            | Self::Chop { offset_delta, .. }
            | Self::SameExtended { offset_delta }
            | Self::Append { offset_delta, .. }
            | Self::Full { offset_delta, .. } => *offset_delta,
        }
    }
}

/// Verification type encoded in a stack-map frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum VerificationType {
    /// Unusable local-variable slot.
    Top,
    /// Integer-like value.
    Integer,
    /// IEEE 754 single-precision value.
    Float,
    /// IEEE 754 double-precision value.
    Double,
    /// Signed 64-bit integer value.
    Long,
    /// Null reference.
    Null,
    /// Uninitialized `this` reference in a constructor.
    UninitializedThis,
    /// Object or array type identified by a class constant.
    Object(u16),
    /// Uninitialized value created by `new` at the supplied bytecode offset.
    Uninitialized(u16),
}

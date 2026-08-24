//! Typed stack-map frames and verification types.

const SAME_FRAME_MIN_TAG: u8 = 0;
const SAME_FRAME_MAX_TAG: u8 = 63;
const SAME_LOCALS_ONE_STACK_MIN_TAG: u8 = 64;
const SAME_LOCALS_ONE_STACK_MAX_TAG: u8 = 127;
const SAME_LOCALS_ONE_STACK_EXTENDED_TAG: u8 = 247;
const CHOP_FRAME_MIN_TAG: u8 = 248;
const CHOP_FRAME_MAX_TAG: u8 = 250;
const SAME_FRAME_EXTENDED_TAG: u8 = 251;
const APPEND_FRAME_MIN_TAG: u8 = 252;
const APPEND_FRAME_MAX_TAG: u8 = 254;
const FULL_FRAME_TAG: u8 = 255;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StackMapFrameTag {
    Same(u8),
    SameLocalsOneStack(u8),
    SameLocalsOneStackExtended,
    Chop(u8),
    SameExtended,
    Append(u8),
    Full,
}

impl StackMapFrameTag {
    pub(crate) const fn from_byte(byte: u8) -> Option<Self> {
        match byte {
            SAME_FRAME_MIN_TAG..=SAME_FRAME_MAX_TAG => Some(Self::Same(byte)),
            SAME_LOCALS_ONE_STACK_MIN_TAG..=SAME_LOCALS_ONE_STACK_MAX_TAG => Some(
                Self::SameLocalsOneStack(byte - SAME_LOCALS_ONE_STACK_MIN_TAG),
            ),
            SAME_LOCALS_ONE_STACK_EXTENDED_TAG => Some(Self::SameLocalsOneStackExtended),
            CHOP_FRAME_MIN_TAG..=CHOP_FRAME_MAX_TAG => {
                Some(Self::Chop(SAME_FRAME_EXTENDED_TAG - byte))
            }
            SAME_FRAME_EXTENDED_TAG => Some(Self::SameExtended),
            APPEND_FRAME_MIN_TAG..=APPEND_FRAME_MAX_TAG => {
                Some(Self::Append(byte - SAME_FRAME_EXTENDED_TAG))
            }
            FULL_FRAME_TAG => Some(Self::Full),
            _ => None,
        }
    }

    pub(crate) const fn same(offset_delta: u8) -> Option<Self> {
        if offset_delta <= SAME_FRAME_MAX_TAG {
            Some(Self::Same(offset_delta))
        } else {
            None
        }
    }

    pub(crate) const fn same_locals_one_stack(offset_delta: u8) -> Option<Self> {
        if offset_delta <= SAME_FRAME_MAX_TAG {
            Some(Self::SameLocalsOneStack(offset_delta))
        } else {
            None
        }
    }

    pub(crate) const fn chop(absent_locals: u8) -> Option<Self> {
        if absent_locals >= SAME_FRAME_EXTENDED_TAG - CHOP_FRAME_MAX_TAG
            && absent_locals <= SAME_FRAME_EXTENDED_TAG - CHOP_FRAME_MIN_TAG
        {
            Some(Self::Chop(absent_locals))
        } else {
            None
        }
    }

    pub(crate) fn append(local_count: usize) -> Option<Self> {
        let local_count = u8::try_from(local_count).ok()?;
        if (APPEND_FRAME_MIN_TAG - SAME_FRAME_EXTENDED_TAG
            ..=APPEND_FRAME_MAX_TAG - SAME_FRAME_EXTENDED_TAG)
            .contains(&local_count)
        {
            Some(Self::Append(local_count))
        } else {
            None
        }
    }

    pub(crate) const fn byte(self) -> u8 {
        match self {
            Self::Same(offset_delta) => offset_delta,
            Self::SameLocalsOneStack(offset_delta) => SAME_LOCALS_ONE_STACK_MIN_TAG + offset_delta,
            Self::SameLocalsOneStackExtended => SAME_LOCALS_ONE_STACK_EXTENDED_TAG,
            Self::Chop(absent_locals) => SAME_FRAME_EXTENDED_TAG - absent_locals,
            Self::SameExtended => SAME_FRAME_EXTENDED_TAG,
            Self::Append(local_count) => SAME_FRAME_EXTENDED_TAG + local_count,
            Self::Full => FULL_FRAME_TAG,
        }
    }
}

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

/// Format discriminator for a stack-map verification type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum VerificationTypeKind {
    /// Unusable local-variable slot.
    Top = 0,
    /// Integer-like value.
    Integer = 1,
    /// IEEE 754 single-precision value.
    Float = 2,
    /// IEEE 754 double-precision value.
    Double = 3,
    /// Signed 64-bit integer value.
    Long = 4,
    /// Null reference.
    Null = 5,
    /// Uninitialized `this` reference in a constructor.
    UninitializedThis = 6,
    /// Object or array type identified by a class constant.
    Object = 7,
    /// Uninitialized value created by `new`.
    Uninitialized = 8,
}

impl VerificationTypeKind {
    /// Returns the class-file tag byte.
    #[must_use]
    pub const fn tag(self) -> u8 {
        self as u8
    }

    pub(crate) const fn from_tag(tag: u8) -> Option<Self> {
        match tag {
            tag if tag == Self::Top.tag() => Some(Self::Top),
            tag if tag == Self::Integer.tag() => Some(Self::Integer),
            tag if tag == Self::Float.tag() => Some(Self::Float),
            tag if tag == Self::Double.tag() => Some(Self::Double),
            tag if tag == Self::Long.tag() => Some(Self::Long),
            tag if tag == Self::Null.tag() => Some(Self::Null),
            tag if tag == Self::UninitializedThis.tag() => Some(Self::UninitializedThis),
            tag if tag == Self::Object.tag() => Some(Self::Object),
            tag if tag == Self::Uninitialized.tag() => Some(Self::Uninitialized),
            _ => None,
        }
    }
}

impl VerificationType {
    /// Returns this verification value's format discriminator.
    #[must_use]
    pub const fn kind(self) -> VerificationTypeKind {
        match self {
            Self::Top => VerificationTypeKind::Top,
            Self::Integer => VerificationTypeKind::Integer,
            Self::Float => VerificationTypeKind::Float,
            Self::Double => VerificationTypeKind::Double,
            Self::Long => VerificationTypeKind::Long,
            Self::Null => VerificationTypeKind::Null,
            Self::UninitializedThis => VerificationTypeKind::UninitializedThis,
            Self::Object(_) => VerificationTypeKind::Object,
            Self::Uninitialized(_) => VerificationTypeKind::Uninitialized,
        }
    }
}

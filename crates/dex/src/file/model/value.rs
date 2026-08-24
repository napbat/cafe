//! DEX encoded values, arrays, call sites, and annotations.

use super::{FieldIndex, MethodHandleIndex, MethodIndex, PrototypeIndex, StringIndex, TypeIndex};

/// Maximum supported nesting for recursive encoded values.
pub(crate) const MAX_ENCODED_VALUE_DEPTH: usize = 128;
/// Initial nesting depth for a top-level encoded value or array.
pub(crate) const ROOT_ENCODED_VALUE_DEPTH: usize = 0;
/// Mask selecting the type tag from an encoded-value header.
pub(crate) const ENCODED_VALUE_TAG_MASK: u8 = 0x1f;
/// Bit displacement of the encoded-value width argument.
pub(crate) const ENCODED_VALUE_ARGUMENT_SHIFT: u32 = 5;
/// Bias converting a zero-based value argument to a byte width.
pub(crate) const ENCODED_VALUE_WIDTH_BIAS: u8 = 1;
/// Increase in nesting depth when entering an encoded array or annotation.
pub(crate) const ENCODED_VALUE_NESTING_INCREMENT: usize = 1;

/// Typed value-argument field from an encoded-value header.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct EncodedValueArgument(u8);

impl EncodedValueArgument {
    pub(crate) const ZERO: Self = Self(0);
    const BOOLEAN_TRUE: Self = Self(1);
    const U16_MAXIMUM: Self = Self(1);
    const U32_MAXIMUM: Self = Self(3);
    const U64_MAXIMUM: Self = Self(7);

    pub(crate) const fn from_header(header: u8) -> Self {
        Self(header >> ENCODED_VALUE_ARGUMENT_SHIFT)
    }

    pub(crate) const fn from_boolean(value: bool) -> Self {
        if value {
            Self::BOOLEAN_TRUE
        } else {
            Self::ZERO
        }
    }

    pub(crate) fn from_payload_width(width: usize) -> Option<Self> {
        let width = u8::try_from(width).ok()?;
        width.checked_sub(ENCODED_VALUE_WIDTH_BIAS).map(Self)
    }

    pub(crate) const fn payload_width(self) -> usize {
        (self.0 + ENCODED_VALUE_WIDTH_BIAS) as usize
    }

    pub(crate) const fn header_bits(self) -> u8 {
        self.0 << ENCODED_VALUE_ARGUMENT_SHIFT
    }

    pub(crate) const fn get(self) -> u8 {
        self.0
    }
}

/// Format-defined tag stored in an encoded-value header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub(crate) enum EncodedValueTag {
    Byte = 0x00,
    Short = 0x02,
    Char = 0x03,
    Int = 0x04,
    Long = 0x06,
    Float = 0x10,
    Double = 0x11,
    MethodType = 0x15,
    MethodHandle = 0x16,
    String = 0x17,
    Type = 0x18,
    Field = 0x19,
    Method = 0x1a,
    Enum = 0x1b,
    Array = 0x1c,
    Annotation = 0x1d,
    Null = 0x1e,
    Boolean = 0x1f,
}

impl EncodedValueTag {
    pub(crate) const fn from_byte(value: u8) -> Option<Self> {
        match value {
            0x00 => Some(Self::Byte),
            0x02 => Some(Self::Short),
            0x03 => Some(Self::Char),
            0x04 => Some(Self::Int),
            0x06 => Some(Self::Long),
            0x10 => Some(Self::Float),
            0x11 => Some(Self::Double),
            0x15 => Some(Self::MethodType),
            0x16 => Some(Self::MethodHandle),
            0x17 => Some(Self::String),
            0x18 => Some(Self::Type),
            0x19 => Some(Self::Field),
            0x1a => Some(Self::Method),
            0x1b => Some(Self::Enum),
            0x1c => Some(Self::Array),
            0x1d => Some(Self::Annotation),
            0x1e => Some(Self::Null),
            0x1f => Some(Self::Boolean),
            _ => None,
        }
    }

    pub(crate) const fn byte(self) -> u8 {
        self as u8
    }

    pub(crate) const fn maximum_argument(self) -> EncodedValueArgument {
        match self {
            Self::Byte | Self::Array | Self::Annotation | Self::Null => EncodedValueArgument::ZERO,
            Self::Short | Self::Char => EncodedValueArgument::U16_MAXIMUM,
            Self::Boolean => EncodedValueArgument::BOOLEAN_TRUE,
            Self::Int
            | Self::Float
            | Self::MethodType
            | Self::MethodHandle
            | Self::String
            | Self::Type
            | Self::Field
            | Self::Method
            | Self::Enum => EncodedValueArgument::U32_MAXIMUM,
            Self::Long | Self::Double => EncodedValueArgument::U64_MAXIMUM,
        }
    }
}

/// One recursively encoded DEX value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EncodedValue {
    /// Signed 8-bit integer.
    Byte(i8),
    /// Signed 16-bit integer.
    Short(i16),
    /// Unsigned UTF-16 code unit.
    Char(u16),
    /// Signed 32-bit integer.
    Int(i32),
    /// Signed 64-bit integer.
    Long(i64),
    /// IEEE-754 single-precision bits.
    Float(u32),
    /// IEEE-754 double-precision bits.
    Double(u64),
    /// Method prototype reference.
    MethodType(PrototypeIndex),
    /// Method handle reference.
    MethodHandle(MethodHandleIndex),
    /// String reference.
    String(StringIndex),
    /// Type reference.
    Type(TypeIndex),
    /// Field reference.
    Field(FieldIndex),
    /// Method reference.
    Method(MethodIndex),
    /// Enum constant represented by a field reference.
    Enum(FieldIndex),
    /// Nested array.
    Array(Vec<EncodedValue>),
    /// Nested annotation.
    Annotation(EncodedAnnotation),
    /// Null reference.
    Null,
    /// Boolean value encoded in the value argument.
    Boolean(bool),
}

/// One encoded annotation value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EncodedAnnotation {
    /// Annotation type.
    pub annotation_type: TypeIndex,
    /// Elements sorted by name index.
    pub elements: Vec<AnnotationElement>,
}

/// Named value in an encoded annotation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnnotationElement {
    /// Element-name string index.
    pub name: StringIndex,
    /// Element value.
    pub value: EncodedValue,
}

/// Bootstrap call-site definition encoded as an array.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallSite {
    /// Encoded bootstrap arguments.
    pub values: Vec<EncodedValue>,
    /// Original absolute encoded-array offset.
    pub data_offset: u32,
}

/// Required leading components and optional arguments of a valid DEX call site.
#[derive(Debug, Clone, Copy)]
pub struct CallSiteComponents<'a> {
    /// Bootstrap method-handle index.
    pub bootstrap_method: MethodHandleIndex,
    /// String index naming the dynamically invoked method.
    pub method_name: StringIndex,
    /// Prototype index describing the dynamically invoked method type.
    pub method_type: PrototypeIndex,
    /// Additional bootstrap arguments in encoded order.
    pub arguments: &'a [EncodedValue],
}

impl CallSite {
    /// Resolves the required typed prefix of this encoded call-site array.
    ///
    /// Returns `None` when the array does not begin with a method handle,
    /// method-name string, and method-type prototype in that order.
    #[must_use]
    pub fn components(&self) -> Option<CallSiteComponents<'_>> {
        let [
            EncodedValue::MethodHandle(bootstrap_method),
            EncodedValue::String(method_name),
            EncodedValue::MethodType(method_type),
            arguments @ ..,
        ] = self.values.as_slice()
        else {
            return None;
        };
        Some(CallSiteComponents {
            bootstrap_method: *bootstrap_method,
            method_name: *method_name,
            method_type: *method_type,
            arguments,
        })
    }
}

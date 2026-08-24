//! Typed parsing and JNI ABI mapping for JVM method descriptors.

use std::fmt;
use std::num::NonZeroU8;

use thiserror::Error;

use crate::text::JavaText;

/// Maximum array rank representable by a JVM descriptor.
pub const MAX_ARRAY_DIMENSIONS: u8 = u8::MAX;
/// Width of one descriptor tag measured in UTF-16 code units.
pub const DESCRIPTOR_TAG_WIDTH: usize = 1;

/// Internal name of the JNI-specialized `jclass` reference type.
pub const JAVA_LANG_CLASS: &str = "java/lang/Class";
/// Internal name of the JNI-specialized `jstring` reference type.
pub const JAVA_LANG_STRING: &str = "java/lang/String";
/// Internal name of the JNI-specialized `jthrowable` reference type.
pub const JAVA_LANG_THROWABLE: &str = "java/lang/Throwable";

/// C spelling of a JNI environment pointer.
pub const JNI_ENVIRONMENT_C_TYPE: &str = "JNIEnv *";
/// C spelling of the JNI void return type.
pub const JNI_VOID_C_TYPE: &str = "void";
/// C spelling of a JNI boolean value.
pub const JNI_BOOLEAN_C_TYPE: &str = "jboolean";
/// C spelling of a JNI byte value.
pub const JNI_BYTE_C_TYPE: &str = "jbyte";
/// C spelling of a JNI UTF-16 code unit.
pub const JNI_CHAR_C_TYPE: &str = "jchar";
/// C spelling of a JNI short value.
pub const JNI_SHORT_C_TYPE: &str = "jshort";
/// C spelling of a JNI int value.
pub const JNI_INT_C_TYPE: &str = "jint";
/// C spelling of a JNI long value.
pub const JNI_LONG_C_TYPE: &str = "jlong";
/// C spelling of a JNI float value.
pub const JNI_FLOAT_C_TYPE: &str = "jfloat";
/// C spelling of a JNI double value.
pub const JNI_DOUBLE_C_TYPE: &str = "jdouble";
/// C spelling of a general JNI object reference.
pub const JNI_OBJECT_C_TYPE: &str = "jobject";
/// C spelling of a JNI class reference.
pub const JNI_CLASS_C_TYPE: &str = "jclass";
/// C spelling of a JNI string reference.
pub const JNI_STRING_C_TYPE: &str = "jstring";
/// C spelling of a JNI throwable reference.
pub const JNI_THROWABLE_C_TYPE: &str = "jthrowable";
/// C spelling of a JNI object-array reference.
pub const JNI_OBJECT_ARRAY_C_TYPE: &str = "jobjectArray";
/// C spelling of a JNI boolean-array reference.
pub const JNI_BOOLEAN_ARRAY_C_TYPE: &str = "jbooleanArray";
/// C spelling of a JNI byte-array reference.
pub const JNI_BYTE_ARRAY_C_TYPE: &str = "jbyteArray";
/// C spelling of a JNI char-array reference.
pub const JNI_CHAR_ARRAY_C_TYPE: &str = "jcharArray";
/// C spelling of a JNI short-array reference.
pub const JNI_SHORT_ARRAY_C_TYPE: &str = "jshortArray";
/// C spelling of a JNI int-array reference.
pub const JNI_INT_ARRAY_C_TYPE: &str = "jintArray";
/// C spelling of a JNI long-array reference.
pub const JNI_LONG_ARRAY_C_TYPE: &str = "jlongArray";
/// C spelling of a JNI float-array reference.
pub const JNI_FLOAT_ARRAY_C_TYPE: &str = "jfloatArray";
/// C spelling of a JNI double-array reference.
pub const JNI_DOUBLE_ARRAY_C_TYPE: &str = "jdoubleArray";

/// One format-defined UTF-16 tag in a JVM descriptor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u16)]
pub enum DescriptorTag {
    /// Starts the parameter list of a method descriptor.
    ParameterListStart = '(' as u16,
    /// Ends the parameter list of a method descriptor.
    ParameterListEnd = ')' as u16,
    /// Marks a void return type.
    Void = 'V' as u16,
    /// Marks the boolean primitive type.
    Boolean = 'Z' as u16,
    /// Marks the byte primitive type.
    Byte = 'B' as u16,
    /// Marks the UTF-16 `char` primitive type.
    Char = 'C' as u16,
    /// Marks the short primitive type.
    Short = 'S' as u16,
    /// Marks the int primitive type.
    Int = 'I' as u16,
    /// Marks the long primitive type.
    Long = 'J' as u16,
    /// Marks the float primitive type.
    Float = 'F' as u16,
    /// Marks the double primitive type.
    Double = 'D' as u16,
    /// Starts an object type descriptor.
    Object = 'L' as u16,
    /// Ends an object type descriptor.
    ObjectEnd = ';' as u16,
    /// Adds one array dimension.
    Array = '[' as u16,
}

impl DescriptorTag {
    /// Returns the exact UTF-16 code unit used in a descriptor.
    #[must_use]
    pub const fn unit(self) -> u16 {
        self as u16
    }

    fn from_unit(unit: u16) -> Option<Self> {
        match unit {
            value if value == Self::ParameterListStart.unit() => Some(Self::ParameterListStart),
            value if value == Self::ParameterListEnd.unit() => Some(Self::ParameterListEnd),
            value if value == Self::Void.unit() => Some(Self::Void),
            value if value == Self::Boolean.unit() => Some(Self::Boolean),
            value if value == Self::Byte.unit() => Some(Self::Byte),
            value if value == Self::Char.unit() => Some(Self::Char),
            value if value == Self::Short.unit() => Some(Self::Short),
            value if value == Self::Int.unit() => Some(Self::Int),
            value if value == Self::Long.unit() => Some(Self::Long),
            value if value == Self::Float.unit() => Some(Self::Float),
            value if value == Self::Double.unit() => Some(Self::Double),
            value if value == Self::Object.unit() => Some(Self::Object),
            value if value == Self::ObjectEnd.unit() => Some(Self::ObjectEnd),
            value if value == Self::Array.unit() => Some(Self::Array),
            _ => None,
        }
    }
}

/// Zero-based position in a sequence of Java UTF-16 code units.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Utf16Offset(usize);

impl Utf16Offset {
    /// Start of a UTF-16 sequence.
    pub const START: Self = Self(0);

    /// Creates an offset from its zero-based code-unit position.
    #[must_use]
    pub const fn new(value: usize) -> Self {
        Self(value)
    }

    /// Returns the zero-based code-unit position.
    #[must_use]
    pub const fn get(self) -> usize {
        self.0
    }
}

impl fmt::Display for Utf16Offset {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

/// Specific structural problem found in a JVM method descriptor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DescriptorProblem {
    /// The descriptor does not start with a parameter list.
    MissingParameterList,
    /// The parameter list has no closing delimiter.
    UnterminatedParameterList,
    /// The descriptor has no return type.
    MissingReturnType,
    /// More data follows the return type.
    TrailingData,
    /// A field or array element type is absent.
    MissingType,
    /// A type starts with an unknown or contextually invalid tag.
    UnexpectedTypeTag(u16),
    /// An object type has no terminating semicolon.
    UnterminatedObjectType,
    /// An object type has an empty internal class name.
    EmptyObjectType,
    /// An array exceeds the JVM's dimension limit.
    TooManyArrayDimensions,
}

impl fmt::Display for DescriptorProblem {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingParameterList => formatter.write_str("missing parameter-list start"),
            Self::UnterminatedParameterList => formatter.write_str("unterminated parameter list"),
            Self::MissingReturnType => formatter.write_str("missing return type"),
            Self::TrailingData => formatter.write_str("trailing descriptor data"),
            Self::MissingType => formatter.write_str("missing field type"),
            Self::UnexpectedTypeTag(unit) => {
                write!(formatter, "unexpected type tag U+{unit:04X}")
            }
            Self::UnterminatedObjectType => formatter.write_str("unterminated object type"),
            Self::EmptyObjectType => formatter.write_str("empty object type"),
            Self::TooManyArrayDimensions => formatter.write_str("too many array dimensions"),
        }
    }
}

/// Error produced while parsing a JVM method descriptor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Error)]
#[error("invalid JVM method descriptor at UTF-16 offset {offset}: {problem}")]
pub struct DescriptorError {
    offset: Utf16Offset,
    problem: DescriptorProblem,
}

impl DescriptorError {
    /// Returns the position at which parsing failed.
    #[must_use]
    pub const fn offset(self) -> Utf16Offset {
        self.offset
    }

    /// Returns the typed reason parsing failed.
    #[must_use]
    pub const fn problem(self) -> DescriptorProblem {
        self.problem
    }
}

/// JVM primitive type usable in a field or method descriptor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PrimitiveType {
    /// Java `boolean`.
    Boolean,
    /// Java `byte`.
    Byte,
    /// Java `char`.
    Char,
    /// Java `short`.
    Short,
    /// Java `int`.
    Int,
    /// Java `long`.
    Long,
    /// Java `float`.
    Float,
    /// Java `double`.
    Double,
}

impl PrimitiveType {
    /// Returns this primitive's descriptor tag.
    #[must_use]
    pub const fn descriptor_tag(self) -> DescriptorTag {
        match self {
            Self::Boolean => DescriptorTag::Boolean,
            Self::Byte => DescriptorTag::Byte,
            Self::Char => DescriptorTag::Char,
            Self::Short => DescriptorTag::Short,
            Self::Int => DescriptorTag::Int,
            Self::Long => DescriptorTag::Long,
            Self::Float => DescriptorTag::Float,
            Self::Double => DescriptorTag::Double,
        }
    }

    /// Returns the JNI ABI type corresponding to this primitive.
    #[must_use]
    pub const fn native_type(self) -> NativeType {
        match self {
            Self::Boolean => NativeType::Boolean,
            Self::Byte => NativeType::Byte,
            Self::Char => NativeType::Char,
            Self::Short => NativeType::Short,
            Self::Int => NativeType::Int,
            Self::Long => NativeType::Long,
            Self::Float => NativeType::Float,
            Self::Double => NativeType::Double,
        }
    }
}

/// Valid, nonzero number of dimensions in a JVM array type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ArrayDimensions(NonZeroU8);

impl ArrayDimensions {
    /// Rank of a one-dimensional array.
    pub const SINGLE: Self = Self(NonZeroU8::MIN);

    /// Creates a nonzero array rank.
    #[must_use]
    pub const fn new(value: u8) -> Option<Self> {
        match NonZeroU8::new(value) {
            Some(value) => Some(Self(value)),
            None => None,
        }
    }

    /// Returns the number of array dimensions.
    #[must_use]
    pub const fn get(self) -> u8 {
        self.0.get()
    }
}

/// Non-array element stored by a JVM array type.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ArrayElement {
    /// Primitive array element.
    Primitive(PrimitiveType),
    /// Object array element, represented by an internal JVM class name.
    Object(JavaText),
}

/// JVM array type with an explicit rank and terminal element type.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ArrayType {
    dimensions: ArrayDimensions,
    element: ArrayElement,
}

impl ArrayType {
    /// Creates an array type.
    #[must_use]
    pub const fn new(dimensions: ArrayDimensions, element: ArrayElement) -> Self {
        Self {
            dimensions,
            element,
        }
    }

    /// Returns the array rank.
    #[must_use]
    pub const fn dimensions(&self) -> ArrayDimensions {
        self.dimensions
    }

    /// Returns the non-array element type.
    #[must_use]
    pub const fn element(&self) -> &ArrayElement {
        &self.element
    }

    /// Returns the JNI array-reference type used at the native boundary.
    #[must_use]
    pub const fn native_type(&self) -> NativeType {
        if self.dimensions.get() != ArrayDimensions::SINGLE.get() {
            return NativeType::ObjectArray;
        }
        match self.element {
            ArrayElement::Primitive(PrimitiveType::Boolean) => NativeType::BooleanArray,
            ArrayElement::Primitive(PrimitiveType::Byte) => NativeType::ByteArray,
            ArrayElement::Primitive(PrimitiveType::Char) => NativeType::CharArray,
            ArrayElement::Primitive(PrimitiveType::Short) => NativeType::ShortArray,
            ArrayElement::Primitive(PrimitiveType::Int) => NativeType::IntArray,
            ArrayElement::Primitive(PrimitiveType::Long) => NativeType::LongArray,
            ArrayElement::Primitive(PrimitiveType::Float) => NativeType::FloatArray,
            ArrayElement::Primitive(PrimitiveType::Double) => NativeType::DoubleArray,
            ArrayElement::Object(_) => NativeType::ObjectArray,
        }
    }
}

/// Non-void Java type usable in a method descriptor.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum JavaType {
    /// Primitive scalar.
    Primitive(PrimitiveType),
    /// Object reference, represented by an internal JVM class name.
    Object(JavaText),
    /// Array reference.
    Array(ArrayType),
}

impl JavaType {
    /// Returns the JNI ABI type used for this Java type.
    #[must_use]
    pub fn native_type(&self) -> NativeType {
        match self {
            Self::Primitive(primitive) => primitive.native_type(),
            Self::Object(name) if name.equals(JAVA_LANG_CLASS) => NativeType::Class,
            Self::Object(name) if name.equals(JAVA_LANG_STRING) => NativeType::String,
            Self::Object(name) if name.equals(JAVA_LANG_THROWABLE) => NativeType::Throwable,
            Self::Object(_) => NativeType::Object,
            Self::Array(array) => array.native_type(),
        }
    }
}

/// Return type of a JVM method descriptor.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ReturnType {
    /// Java `void`.
    Void,
    /// A non-void Java type.
    Type(JavaType),
}

impl ReturnType {
    /// Returns the JNI ABI return type.
    #[must_use]
    pub fn native_type(&self) -> NativeType {
        match self {
            Self::Void => NativeType::Void,
            Self::Type(value) => value.native_type(),
        }
    }
}

/// C-level JNI type used by a native method declaration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NativeType {
    /// Per-thread JNI environment pointer.
    Environment,
    /// C `void` return.
    Void,
    /// JNI boolean scalar.
    Boolean,
    /// JNI byte scalar.
    Byte,
    /// JNI UTF-16 code unit.
    Char,
    /// JNI short scalar.
    Short,
    /// JNI int scalar.
    Int,
    /// JNI long scalar.
    Long,
    /// JNI float scalar.
    Float,
    /// JNI double scalar.
    Double,
    /// General object reference.
    Object,
    /// Class object reference.
    Class,
    /// String object reference.
    String,
    /// Throwable object reference.
    Throwable,
    /// Object-array reference.
    ObjectArray,
    /// Boolean-array reference.
    BooleanArray,
    /// Byte-array reference.
    ByteArray,
    /// Char-array reference.
    CharArray,
    /// Short-array reference.
    ShortArray,
    /// Int-array reference.
    IntArray,
    /// Long-array reference.
    LongArray,
    /// Float-array reference.
    FloatArray,
    /// Double-array reference.
    DoubleArray,
}

impl NativeType {
    /// Returns the standard C spelling from `jni.h`.
    #[must_use]
    pub const fn c_name(self) -> &'static str {
        match self {
            Self::Environment => JNI_ENVIRONMENT_C_TYPE,
            Self::Void => JNI_VOID_C_TYPE,
            Self::Boolean => JNI_BOOLEAN_C_TYPE,
            Self::Byte => JNI_BYTE_C_TYPE,
            Self::Char => JNI_CHAR_C_TYPE,
            Self::Short => JNI_SHORT_C_TYPE,
            Self::Int => JNI_INT_C_TYPE,
            Self::Long => JNI_LONG_C_TYPE,
            Self::Float => JNI_FLOAT_C_TYPE,
            Self::Double => JNI_DOUBLE_C_TYPE,
            Self::Object => JNI_OBJECT_C_TYPE,
            Self::Class => JNI_CLASS_C_TYPE,
            Self::String => JNI_STRING_C_TYPE,
            Self::Throwable => JNI_THROWABLE_C_TYPE,
            Self::ObjectArray => JNI_OBJECT_ARRAY_C_TYPE,
            Self::BooleanArray => JNI_BOOLEAN_ARRAY_C_TYPE,
            Self::ByteArray => JNI_BYTE_ARRAY_C_TYPE,
            Self::CharArray => JNI_CHAR_ARRAY_C_TYPE,
            Self::ShortArray => JNI_SHORT_ARRAY_C_TYPE,
            Self::IntArray => JNI_INT_ARRAY_C_TYPE,
            Self::LongArray => JNI_LONG_ARRAY_C_TYPE,
            Self::FloatArray => JNI_FLOAT_ARRAY_C_TYPE,
            Self::DoubleArray => JNI_DOUBLE_ARRAY_C_TYPE,
        }
    }

    /// Returns whether this type is a JNI reference handle.
    #[must_use]
    pub const fn is_reference(self) -> bool {
        matches!(
            self,
            Self::Object
                | Self::Class
                | Self::String
                | Self::Throwable
                | Self::ObjectArray
                | Self::BooleanArray
                | Self::ByteArray
                | Self::CharArray
                | Self::ShortArray
                | Self::IntArray
                | Self::LongArray
                | Self::FloatArray
                | Self::DoubleArray
        )
    }
}

/// Parsed JVM method descriptor with its exact UTF-16 spelling retained.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct MethodDescriptor {
    text: JavaText,
    parameter_end: Utf16Offset,
    parameters: Vec<JavaType>,
    return_type: ReturnType,
}

impl MethodDescriptor {
    /// Parses a JVM method descriptor from valid Unicode text.
    ///
    /// # Errors
    ///
    /// Returns a typed structural error for malformed descriptor syntax.
    pub fn parse(value: &str) -> Result<Self, DescriptorError> {
        Self::from_utf16(value.encode_utf16().collect())
    }

    /// Parses a JVM method descriptor from exact Java UTF-16 code units.
    ///
    /// # Errors
    ///
    /// Returns a typed structural error for malformed descriptor syntax.
    pub fn from_utf16(units: Vec<u16>) -> Result<Self, DescriptorError> {
        Parser::new(&units).parse().map(|parsed| Self {
            text: JavaText::from_utf16(units),
            parameter_end: parsed.parameter_end,
            parameters: parsed.parameters,
            return_type: parsed.return_type,
        })
    }

    /// Returns the exact descriptor text.
    #[must_use]
    pub const fn text(&self) -> &JavaText {
        &self.text
    }

    /// Returns the exact parameter-descriptor code units without parentheses.
    #[must_use]
    pub fn parameter_utf16_units(&self) -> &[u16] {
        const FIRST_PARAMETER_OFFSET: usize = DESCRIPTOR_TAG_WIDTH;

        &self.text.utf16_units()[FIRST_PARAMETER_OFFSET..self.parameter_end.get()]
    }

    /// Returns parameter types in declaration order.
    #[must_use]
    pub fn parameters(&self) -> &[JavaType] {
        &self.parameters
    }

    /// Returns the method's return type.
    #[must_use]
    pub const fn return_type(&self) -> &ReturnType {
        &self.return_type
    }
}

struct ParsedDescriptor {
    parameter_end: Utf16Offset,
    parameters: Vec<JavaType>,
    return_type: ReturnType,
}

struct Parser<'a> {
    units: &'a [u16],
    position: usize,
}

impl<'a> Parser<'a> {
    const fn new(units: &'a [u16]) -> Self {
        Self {
            units,
            position: Utf16Offset::START.get(),
        }
    }

    fn parse(mut self) -> Result<ParsedDescriptor, DescriptorError> {
        if self.take_tag() != Some(DescriptorTag::ParameterListStart) {
            return Err(Self::error_at(
                Utf16Offset::START.get(),
                DescriptorProblem::MissingParameterList,
            ));
        }

        let mut parameters = Vec::new();
        loop {
            match self.peek_tag() {
                Some(DescriptorTag::ParameterListEnd) => break,
                None if self.peek().is_none() => {
                    return Err(self.error(DescriptorProblem::UnterminatedParameterList));
                }
                _ => parameters.push(self.parse_type()?),
            }
        }
        let parameter_end = Utf16Offset::new(self.position);
        self.take();

        let return_type = match self.peek_tag() {
            Some(DescriptorTag::Void) => {
                self.take();
                ReturnType::Void
            }
            None if self.peek().is_none() => {
                return Err(self.error(DescriptorProblem::MissingReturnType));
            }
            _ => ReturnType::Type(self.parse_type()?),
        };

        if self.peek().is_some() {
            return Err(self.error(DescriptorProblem::TrailingData));
        }

        Ok(ParsedDescriptor {
            parameter_end,
            parameters,
            return_type,
        })
    }

    fn parse_type(&mut self) -> Result<JavaType, DescriptorError> {
        let offset = self.position;
        let Some(unit) = self.take() else {
            return Err(Self::error_at(offset, DescriptorProblem::MissingType));
        };
        let Some(tag) = DescriptorTag::from_unit(unit) else {
            return Err(Self::error_at(
                offset,
                DescriptorProblem::UnexpectedTypeTag(unit),
            ));
        };
        match tag {
            tag @ (DescriptorTag::Boolean
            | DescriptorTag::Byte
            | DescriptorTag::Char
            | DescriptorTag::Short
            | DescriptorTag::Int
            | DescriptorTag::Long
            | DescriptorTag::Float
            | DescriptorTag::Double) => Ok(JavaType::Primitive(primitive_from_tag(tag))),
            DescriptorTag::Object => self.parse_object().map(JavaType::Object),
            DescriptorTag::Array => self.parse_array(),
            tag => Err(Self::error_at(
                offset,
                DescriptorProblem::UnexpectedTypeTag(tag.unit()),
            )),
        }
    }

    fn parse_object(&mut self) -> Result<JavaText, DescriptorError> {
        let name_start = self.position;
        while self.peek_tag() != Some(DescriptorTag::ObjectEnd) {
            if self.take().is_none() {
                return Err(self.error(DescriptorProblem::UnterminatedObjectType));
            }
        }
        if self.position == name_start {
            return Err(Self::error_at(
                name_start,
                DescriptorProblem::EmptyObjectType,
            ));
        }
        let name = JavaText::from_utf16(self.units[name_start..self.position].to_vec());
        self.take();
        Ok(name)
    }

    fn parse_array(&mut self) -> Result<JavaType, DescriptorError> {
        let array_start = self.position.saturating_sub(DESCRIPTOR_TAG_WIDTH);
        let mut dimensions = usize::from(ArrayDimensions::SINGLE.get());
        while self.peek_tag() == Some(DescriptorTag::Array) {
            self.take();
            dimensions = dimensions.saturating_add(usize::from(ArrayDimensions::SINGLE.get()));
            if dimensions > usize::from(MAX_ARRAY_DIMENSIONS) {
                return Err(Self::error_at(
                    array_start,
                    DescriptorProblem::TooManyArrayDimensions,
                ));
            }
        }
        let element = match self.parse_type()? {
            JavaType::Primitive(value) => ArrayElement::Primitive(value),
            JavaType::Object(value) => ArrayElement::Object(value),
            JavaType::Array(_) => unreachable!("all array dimensions are consumed together"),
        };
        let dimensions = u8::try_from(dimensions)
            .ok()
            .and_then(ArrayDimensions::new)
            .expect("validated array dimensions fit a nonzero byte");
        Ok(JavaType::Array(ArrayType::new(dimensions, element)))
    }

    fn peek(&self) -> Option<u16> {
        self.units.get(self.position).copied()
    }

    fn peek_tag(&self) -> Option<DescriptorTag> {
        self.peek().and_then(DescriptorTag::from_unit)
    }

    fn take(&mut self) -> Option<u16> {
        let value = self.peek()?;
        self.position += DESCRIPTOR_TAG_WIDTH;
        Some(value)
    }

    fn take_tag(&mut self) -> Option<DescriptorTag> {
        self.take().and_then(DescriptorTag::from_unit)
    }

    const fn error(&self, problem: DescriptorProblem) -> DescriptorError {
        Self::error_at(self.position, problem)
    }

    const fn error_at(position: usize, problem: DescriptorProblem) -> DescriptorError {
        DescriptorError {
            offset: Utf16Offset::new(position),
            problem,
        }
    }
}

fn primitive_from_tag(tag: DescriptorTag) -> PrimitiveType {
    match tag {
        DescriptorTag::Boolean => PrimitiveType::Boolean,
        DescriptorTag::Byte => PrimitiveType::Byte,
        DescriptorTag::Char => PrimitiveType::Char,
        DescriptorTag::Short => PrimitiveType::Short,
        DescriptorTag::Int => PrimitiveType::Int,
        DescriptorTag::Long => PrimitiveType::Long,
        DescriptorTag::Float => PrimitiveType::Float,
        DescriptorTag::Double => PrimitiveType::Double,
        DescriptorTag::ParameterListStart
        | DescriptorTag::ParameterListEnd
        | DescriptorTag::Void
        | DescriptorTag::Object
        | DescriptorTag::ObjectEnd
        | DescriptorTag::Array => unreachable!("caller supplies a primitive descriptor tag"),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ArrayElement, DescriptorProblem, JavaType, MethodDescriptor, NativeType, PrimitiveType,
        ReturnType,
    };

    #[test]
    fn parses_and_maps_a_complete_method_descriptor() {
        let descriptor = MethodDescriptor::parse(
            "(IZLjava/lang/String;[B[[Ljava/lang/Object;)Ljava/lang/Class;",
        )
        .unwrap();

        assert_eq!(
            descriptor.parameter_utf16_units(),
            "IZLjava/lang/String;[B[[Ljava/lang/Object;"
                .encode_utf16()
                .collect::<Vec<_>>()
        );
        assert_eq!(descriptor.parameters().len(), 5);
        assert_eq!(
            descriptor.parameters()[0],
            JavaType::Primitive(PrimitiveType::Int)
        );
        assert_eq!(
            descriptor.parameters()[1].native_type(),
            NativeType::Boolean
        );
        assert_eq!(descriptor.parameters()[2].native_type(), NativeType::String);
        assert_eq!(
            descriptor.parameters()[3].native_type(),
            NativeType::ByteArray
        );
        assert_eq!(
            descriptor.parameters()[4].native_type(),
            NativeType::ObjectArray
        );
        assert_eq!(descriptor.return_type().native_type(), NativeType::Class);
    }

    #[test]
    fn distinguishes_primitive_and_nested_arrays() {
        let descriptor = MethodDescriptor::parse("([I[[I)V").unwrap();
        let JavaType::Array(one_dimension) = &descriptor.parameters()[0] else {
            panic!("expected array")
        };
        let JavaType::Array(two_dimensions) = &descriptor.parameters()[1] else {
            panic!("expected array")
        };

        assert_eq!(one_dimension.native_type(), NativeType::IntArray);
        assert_eq!(two_dimensions.native_type(), NativeType::ObjectArray);
        assert_eq!(
            one_dimension.element(),
            &ArrayElement::Primitive(PrimitiveType::Int)
        );
        assert_eq!(descriptor.return_type(), &ReturnType::Void);
    }

    #[test]
    fn reports_typed_descriptor_failures() {
        let error = MethodDescriptor::parse("(I").unwrap_err();

        assert_eq!(
            error.problem(),
            DescriptorProblem::UnterminatedParameterList
        );
        assert_eq!(error.offset().get(), 2);
    }

    #[test]
    fn reports_a_missing_array_element_at_end_of_input() {
        let error = MethodDescriptor::parse("([").unwrap_err();

        assert_eq!(error.problem(), DescriptorProblem::MissingType);
        assert_eq!(error.offset().get(), 2);
    }
}

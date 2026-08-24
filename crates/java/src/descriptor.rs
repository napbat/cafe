//! Parser and Java-style formatter for JVM descriptors.

use std::fmt;

use crate::{Error, Result};

const METHOD_PARAMETERS_START: u8 = b'(';
const METHOD_PARAMETERS_END: u8 = b')';
const VOID_RETURN_TAG: u8 = b'V';
const OBJECT_NAME_END: u8 = b';';
const START_POSITION: usize = 0;
const DESCRIPTOR_TAG_WIDTH: usize = size_of::<u8>();
const EMPTY_OBJECT_NAME_LENGTH: usize = 0;

macro_rules! define_field_type_tags {
    ($($variant:ident = $byte:expr),+ $(,)?) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        #[repr(u8)]
        enum FieldTypeTag {
            $($variant = $byte),+
        }

        impl FieldTypeTag {
            const fn from_byte(value: u8) -> Option<Self> {
                match value {
                    $($byte => Some(Self::$variant),)+
                    _ => None,
                }
            }
        }
    };
}

define_field_type_tags! {
    Byte = b'B',
    Char = b'C',
    Double = b'D',
    Float = b'F',
    Int = b'I',
    Long = b'J',
    Short = b'S',
    Boolean = b'Z',
    Object = b'L',
    Array = b'[',
}

/// A non-void JVM field type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JavaType {
    /// Java `byte`.
    Byte,
    /// Java `char`.
    Char,
    /// Java `double`.
    Double,
    /// Java `float`.
    Float,
    /// Java `int`.
    Int,
    /// Java `long`.
    Long,
    /// Java `short`.
    Short,
    /// Java `boolean`.
    Boolean,
    /// A class or interface, stored with its internal JVM name.
    Object(String),
    /// An array of another field type.
    Array(Box<JavaType>),
}

impl fmt::Display for JavaType {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Byte => formatter.write_str("byte"),
            Self::Char => formatter.write_str("char"),
            Self::Double => formatter.write_str("double"),
            Self::Float => formatter.write_str("float"),
            Self::Int => formatter.write_str("int"),
            Self::Long => formatter.write_str("long"),
            Self::Short => formatter.write_str("short"),
            Self::Boolean => formatter.write_str("boolean"),
            Self::Object(name) => formatter.write_str(&name.replace('/', ".")),
            Self::Array(element) => write!(formatter, "{element}[]"),
        }
    }
}

/// The return portion of a method descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReturnType {
    /// Java `void`.
    Void,
    /// A value-returning type.
    Type(JavaType),
}

impl fmt::Display for ReturnType {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Void => formatter.write_str("void"),
            Self::Type(value) => value.fmt(formatter),
        }
    }
}

/// A parsed JVM method descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MethodDescriptor {
    /// Parameter types in declaration order.
    pub parameters: Vec<JavaType>,
    /// Method return type.
    pub return_type: ReturnType,
}

/// Parses a complete field descriptor.
///
/// # Errors
///
/// Returns an error if the descriptor is incomplete, contains an unknown type
/// tag, or has trailing data.
pub fn parse_field(descriptor: &str) -> Result<JavaType> {
    let mut parser = DescriptorParser::new(descriptor);
    let value = parser.parse_type()?;
    parser.finish()?;
    Ok(value)
}

/// Parses a complete method descriptor.
///
/// # Errors
///
/// Returns an error if the parameter list or return type is malformed or if the
/// descriptor has trailing data.
pub fn parse_method(descriptor: &str) -> Result<MethodDescriptor> {
    let mut parser = DescriptorParser::new(descriptor);
    parser.expect(METHOD_PARAMETERS_START)?;
    let mut parameters = Vec::new();
    while parser.peek() != Some(METHOD_PARAMETERS_END) {
        if parser.peek().is_none() {
            return Err(Error::invalid_descriptor(
                parser.position,
                "unterminated parameter list",
            ));
        }
        parameters.push(parser.parse_type()?);
    }
    parser.expect(METHOD_PARAMETERS_END)?;
    let return_type = if parser.peek() == Some(VOID_RETURN_TAG) {
        parser.position += DESCRIPTOR_TAG_WIDTH;
        ReturnType::Void
    } else {
        ReturnType::Type(parser.parse_type()?)
    };
    parser.finish()?;
    Ok(MethodDescriptor {
        parameters,
        return_type,
    })
}

struct DescriptorParser<'a> {
    descriptor: &'a str,
    bytes: &'a [u8],
    position: usize,
}

impl<'a> DescriptorParser<'a> {
    fn new(descriptor: &'a str) -> Self {
        Self {
            descriptor,
            bytes: descriptor.as_bytes(),
            position: START_POSITION,
        }
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.position).copied()
    }

    fn parse_type(&mut self) -> Result<JavaType> {
        let offset = self.position;
        let encoded_tag = self
            .bytes
            .get(self.position)
            .copied()
            .ok_or_else(|| Error::invalid_descriptor(self.position, "expected a field type"))?;
        self.position += DESCRIPTOR_TAG_WIDTH;
        let tag = FieldTypeTag::from_byte(encoded_tag).ok_or_else(|| {
            Error::invalid_descriptor(offset, format!("unknown field-type tag {encoded_tag:?}"))
        })?;
        match tag {
            FieldTypeTag::Byte => Ok(JavaType::Byte),
            FieldTypeTag::Char => Ok(JavaType::Char),
            FieldTypeTag::Double => Ok(JavaType::Double),
            FieldTypeTag::Float => Ok(JavaType::Float),
            FieldTypeTag::Int => Ok(JavaType::Int),
            FieldTypeTag::Long => Ok(JavaType::Long),
            FieldTypeTag::Short => Ok(JavaType::Short),
            FieldTypeTag::Boolean => Ok(JavaType::Boolean),
            FieldTypeTag::Object => self.parse_object(offset),
            FieldTypeTag::Array => Ok(JavaType::Array(Box::new(self.parse_type()?))),
        }
    }

    fn parse_object(&mut self, offset: usize) -> Result<JavaType> {
        let name_start = self.position;
        let relative_end = self.bytes[name_start..]
            .iter()
            .position(|&byte| byte == OBJECT_NAME_END)
            .ok_or_else(|| Error::invalid_descriptor(offset, "unterminated object type"))?;
        if relative_end == EMPTY_OBJECT_NAME_LENGTH {
            return Err(Error::invalid_descriptor(offset, "empty object class name"));
        }
        let name_end = name_start + relative_end;
        let name = self.descriptor[name_start..name_end].to_owned();
        self.position = name_end + DESCRIPTOR_TAG_WIDTH;
        Ok(JavaType::Object(name))
    }

    fn expect(&mut self, expected: u8) -> Result<()> {
        let actual = self.peek().ok_or_else(|| {
            Error::invalid_descriptor(
                self.position,
                format!("expected `{}`", char::from(expected)),
            )
        })?;
        if actual != expected {
            return Err(Error::invalid_descriptor(
                self.position,
                format!(
                    "expected `{}`, found `{}`",
                    char::from(expected),
                    char::from(actual)
                ),
            ));
        }
        self.position += DESCRIPTOR_TAG_WIDTH;
        Ok(())
    }

    fn finish(&self) -> Result<()> {
        if self.position == self.bytes.len() {
            Ok(())
        } else {
            Err(Error::invalid_descriptor(
                self.position,
                "unexpected trailing descriptor data",
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{JavaType, ReturnType, parse_field, parse_method};

    #[test]
    fn parses_and_formats_method_descriptor() {
        let method = parse_method("(I[Ljava/lang/String;)Ljava/util/List;").unwrap();
        assert_eq!(method.parameters[0], JavaType::Int);
        assert_eq!(method.parameters[1].to_string(), "java.lang.String[]");
        assert_eq!(method.return_type.to_string(), "java.util.List");
    }

    #[test]
    fn supports_void_and_multidimensional_arrays() {
        assert_eq!(parse_field("[[I").unwrap().to_string(), "int[][]");
        assert_eq!(parse_method("()V").unwrap().return_type, ReturnType::Void);
    }

    #[test]
    fn rejects_incomplete_descriptors() {
        assert!(parse_method("(I").is_err());
        assert!(parse_field("Ljava/lang/String").is_err());
        assert!(parse_field("V").is_err());
    }
}

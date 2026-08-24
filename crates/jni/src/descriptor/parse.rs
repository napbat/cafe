//! UTF-16 state machine for JVM method descriptors.

use crate::text::JavaText;

use super::{
    ArrayDimensions, ArrayElement, DESCRIPTOR_TAG_WIDTH, DescriptorError, DescriptorProblem,
    DescriptorTag, JavaType, MAX_ARRAY_DIMENSIONS, PrimitiveType, ReturnType, Utf16Offset,
};

pub(super) struct ParsedDescriptor {
    pub(super) parameter_end: Utf16Offset,
    pub(super) parameters: Vec<JavaType>,
    pub(super) return_type: ReturnType,
}

pub(super) fn parse_method_descriptor(units: &[u16]) -> Result<ParsedDescriptor, DescriptorError> {
    Parser::new(units).parse()
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
        Ok(JavaType::Array(super::ArrayType::new(dimensions, element)))
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

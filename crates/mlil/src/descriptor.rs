//! Checked JVM-compatible descriptors used by semantic verification.

use crate::ValueType;

const MAX_ARRAY_DIMENSIONS: usize = 255;

pub(crate) struct MethodDescriptor {
    pub(crate) parameters: Vec<ValueType>,
    pub(crate) return_type: Option<ValueType>,
}

pub(crate) fn field_type(descriptor: &str) -> Option<ValueType> {
    let mut parser = Parser::new(descriptor);
    let value = parser.value_type()?;
    parser.finished().then_some(value)
}

pub(crate) fn method_descriptor(descriptor: &str) -> Option<MethodDescriptor> {
    let mut parser = Parser::new(descriptor);
    parser.expect(b'(')?;
    let mut parameters = Vec::new();
    while parser.peek()? != b')' {
        parameters.push(parser.value_type()?);
    }
    parser.expect(b')')?;
    let return_type = if parser.peek()? == b'V' {
        parser.position += 1;
        None
    } else {
        Some(parser.value_type()?)
    };
    parser.finished().then_some(MethodDescriptor {
        parameters,
        return_type,
    })
}

pub(crate) fn accepts(expected: &ValueType, actual: &ValueType) -> bool {
    if expected.is_reference() {
        matches!(actual, ValueType::Unknown | ValueType::Zero) || actual.is_reference()
    } else {
        expected.accepts(actual)
    }
}

pub(crate) fn is_reference(descriptor: &str) -> bool {
    matches!(field_type(descriptor), Some(ValueType::Reference(_)))
}

pub(crate) fn is_object(descriptor: &str) -> bool {
    descriptor.starts_with('L') && is_reference(descriptor)
}

struct Parser<'a> {
    descriptor: &'a str,
    bytes: &'a [u8],
    position: usize,
}

impl<'a> Parser<'a> {
    const fn new(descriptor: &'a str) -> Self {
        Self {
            descriptor,
            bytes: descriptor.as_bytes(),
            position: 0,
        }
    }

    fn value_type(&mut self) -> Option<ValueType> {
        let start = self.position;
        match self.take()? {
            b'Z' | b'B' | b'C' | b'S' | b'I' => Some(ValueType::Integer),
            b'J' => Some(ValueType::Long),
            b'F' => Some(ValueType::Float),
            b'D' => Some(ValueType::Double),
            b'L' => {
                let name_start = self.position;
                while !matches!(self.peek(), Some(b';')) {
                    let byte = self.take()?;
                    if matches!(byte, b'.' | b'[' | b'(' | b')' | 0) {
                        return None;
                    }
                }
                if self.position == name_start {
                    return None;
                }
                self.position += 1;
                Some(ValueType::Reference(Some(
                    self.descriptor[start..self.position].to_owned(),
                )))
            }
            b'[' => {
                let mut dimensions = 1usize;
                while matches!(self.peek(), Some(b'[')) {
                    self.position += 1;
                    dimensions += 1;
                }
                if dimensions > MAX_ARRAY_DIMENSIONS {
                    return None;
                }
                self.value_type()?;
                Some(ValueType::Reference(Some(
                    self.descriptor[start..self.position].to_owned(),
                )))
            }
            _ => None,
        }
    }

    fn expect(&mut self, expected: u8) -> Option<()> {
        (self.take()? == expected).then_some(())
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.position).copied()
    }

    fn take(&mut self) -> Option<u8> {
        let value = self.peek()?;
        self.position += 1;
        Some(value)
    }

    const fn finished(&self) -> bool {
        self.position == self.bytes.len()
    }
}

#[cfg(test)]
mod tests {
    use super::{field_type, method_descriptor};
    use crate::ValueType;

    #[test]
    fn parses_complete_field_and_method_descriptors() {
        assert_eq!(
            field_type("[[Ljava/lang/String;"),
            Some(ValueType::Reference(Some(
                "[[Ljava/lang/String;".to_owned()
            )))
        );
        let method = method_descriptor("(IJ[Ljava/lang/Object;)D").unwrap();
        assert_eq!(
            method.parameters,
            vec![
                ValueType::Integer,
                ValueType::Long,
                ValueType::Reference(Some("[Ljava/lang/Object;".to_owned()))
            ]
        );
        assert_eq!(method.return_type, Some(ValueType::Double));
        assert!(method_descriptor("()V").unwrap().return_type.is_none());
    }

    #[test]
    fn rejects_partial_or_void_value_descriptors() {
        assert!(field_type("V").is_none());
        assert!(field_type("Igarbage").is_none());
        assert!(field_type("L;").is_none());
        assert!(method_descriptor("(I").is_none());
        assert!(method_descriptor("(V)V").is_none());
        assert!(method_descriptor("()Vextra").is_none());
    }
}

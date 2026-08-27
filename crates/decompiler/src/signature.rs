//! JVM generic-signature parsing for Java source declarations.

use java::classfile::{
    ClassFile, ConstantPool, FieldInfo, KnownAttribute, KnownAttributeKind, MethodInfo,
};
use java::descriptor::MethodDescriptor;

use crate::Result as DecompilerResult;
use crate::diagnostic::{Diagnostic, DiagnosticCode, MethodIdentity};
use crate::names::{SourceNames, identifier};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ClassSignature {
    pub(crate) type_parameters: String,
    pub(crate) superclass: String,
    pub(crate) interfaces: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MethodSignature {
    pub(crate) type_parameters: String,
    pub(crate) parameters: Vec<String>,
    pub(crate) return_type: String,
    pub(crate) throws: Vec<String>,
}

pub(crate) fn class_attribute(
    class: &ClassFile,
    class_name: &str,
    diagnostics: &mut Vec<Diagnostic>,
    names: &SourceNames,
) -> DecompilerResult<Option<ClassSignature>> {
    let Some(value) = signature_value(
        &class.constant_pool,
        class.known_attribute(KnownAttributeKind::Signature),
    )?
    else {
        return Ok(None);
    };
    match parse_class(value, names) {
        Ok(signature) if signature.interfaces.len() == class.interfaces.len() => {
            Ok(Some(signature))
        }
        Ok(_) => {
            diagnostics.push(Diagnostic::class_warning(
                DiagnosticCode::DeclarationApproximation,
                class_name,
                "generic class signature has a different interface count and was ignored",
            ));
            Ok(None)
        }
        Err(error) => {
            diagnostics.push(Diagnostic::class_warning(
                DiagnosticCode::DeclarationApproximation,
                class_name,
                format!("generic class signature was ignored: {error}"),
            ));
            Ok(None)
        }
    }
}

pub(crate) fn field_attribute(
    class: &ClassFile,
    field: &FieldInfo,
    class_name: &str,
    field_name: &str,
    diagnostics: &mut Vec<Diagnostic>,
    names: &SourceNames,
) -> DecompilerResult<Option<String>> {
    let Some(value) = signature_value(
        &class.constant_pool,
        field.known_attribute(KnownAttributeKind::Signature),
    )?
    else {
        return Ok(None);
    };
    match parse_field(value, names) {
        Ok(signature) => Ok(Some(signature)),
        Err(error) => {
            diagnostics.push(Diagnostic::class_warning(
                DiagnosticCode::DeclarationApproximation,
                class_name,
                format!("generic signature for field `{field_name}` was ignored: {error}"),
            ));
            Ok(None)
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn method_attribute(
    class: &ClassFile,
    method: &MethodInfo,
    descriptor: &MethodDescriptor,
    class_name: &str,
    identity: &MethodIdentity,
    diagnostics: &mut Vec<Diagnostic>,
    names: &SourceNames,
) -> DecompilerResult<Option<MethodSignature>> {
    let Some(value) = signature_value(
        &class.constant_pool,
        method.known_attribute(KnownAttributeKind::Signature),
    )?
    else {
        return Ok(None);
    };
    match parse_method(value, names) {
        Ok(signature) if signature.parameters.len() == descriptor.parameters.len() => {
            Ok(Some(signature))
        }
        Ok(_) => {
            diagnostics.push(Diagnostic::method_warning(
                DiagnosticCode::DeclarationApproximation,
                class_name,
                identity.clone(),
                "generic method signature has a different parameter count and was ignored",
            ));
            Ok(None)
        }
        Err(error) => {
            diagnostics.push(Diagnostic::method_warning(
                DiagnosticCode::DeclarationApproximation,
                class_name,
                identity.clone(),
                format!("generic method signature was ignored: {error}"),
            ));
            Ok(None)
        }
    }
}

fn signature_value<'a>(
    pool: &'a ConstantPool,
    attribute: Option<&KnownAttribute>,
) -> java::Result<Option<&'a str>> {
    match attribute {
        Some(KnownAttribute::Signature(attribute)) => pool.utf8(attribute.index).map(Some),
        _ => Ok(None),
    }
}

pub(crate) fn parse_class(value: &str, names: &SourceNames) -> Result<ClassSignature, String> {
    let mut parser = Parser::new(value, names);
    let type_parameters = parser.formal_type_parameters()?;
    let superclass = parser.class_type()?;
    let mut interfaces = Vec::new();
    while !parser.finished() {
        interfaces.push(parser.class_type()?);
    }
    Ok(ClassSignature {
        type_parameters,
        superclass,
        interfaces,
    })
}

pub(crate) fn parse_field(value: &str, names: &SourceNames) -> Result<String, String> {
    let mut parser = Parser::new(value, names);
    let rendered = parser.reference_type()?;
    parser.finish()?;
    Ok(rendered)
}

pub(crate) fn parse_method(value: &str, names: &SourceNames) -> Result<MethodSignature, String> {
    let mut parser = Parser::new(value, names);
    let type_parameters = parser.formal_type_parameters()?;
    parser.expect(b'(')?;
    let mut parameters = Vec::new();
    while parser.peek() != Some(b')') {
        parameters.push(parser.java_type()?);
    }
    parser.expect(b')')?;
    let return_type = if parser.peek() == Some(b'V') {
        parser.take();
        "void".to_owned()
    } else {
        parser.java_type()?
    };
    let mut throws = Vec::new();
    while parser.peek() == Some(b'^') {
        parser.take();
        throws.push(match parser.peek() {
            Some(b'L') => parser.class_type()?,
            Some(b'T') => parser.type_variable()?,
            _ => return Err(parser.error("throws signature is not a class or type variable")),
        });
    }
    parser.finish()?;
    Ok(MethodSignature {
        type_parameters,
        parameters,
        return_type,
        throws,
    })
}

struct Parser<'a> {
    value: &'a str,
    offset: usize,
    names: &'a SourceNames,
}

impl<'a> Parser<'a> {
    const fn new(value: &'a str, names: &'a SourceNames) -> Self {
        Self {
            value,
            offset: 0,
            names,
        }
    }

    fn finished(&self) -> bool {
        self.offset == self.value.len()
    }

    fn finish(&self) -> Result<(), String> {
        if self.finished() {
            Ok(())
        } else {
            Err(self.error("trailing generic-signature content"))
        }
    }

    fn peek(&self) -> Option<u8> {
        self.value.as_bytes().get(self.offset).copied()
    }

    fn take(&mut self) -> Option<u8> {
        let value = self.peek()?;
        self.offset += 1;
        Some(value)
    }

    fn expect(&mut self, expected: u8) -> Result<(), String> {
        if self.take() == Some(expected) {
            Ok(())
        } else {
            Err(self.error(&format!("expected `{}`", char::from(expected))))
        }
    }

    fn error(&self, message: &str) -> String {
        format!(
            "invalid JVM generic signature at byte {}: {message}",
            self.offset
        )
    }

    fn formal_type_parameters(&mut self) -> Result<String, String> {
        if self.peek() != Some(b'<') {
            return Ok(String::new());
        }
        self.take();
        let mut parameters = Vec::new();
        while self.peek() != Some(b'>') {
            if self.finished() {
                return Err(self.error("unterminated formal type parameters"));
            }
            let raw_name = self.read_until(b":")?;
            self.expect(b':')?;
            let mut bounds = Vec::new();
            if self.peek() != Some(b':') {
                bounds.push(self.reference_type()?);
            }
            while self.peek() == Some(b':') {
                self.take();
                bounds.push(self.reference_type()?);
            }
            if bounds
                .first()
                .is_some_and(|bound| bound == "java.lang.Object")
            {
                bounds.remove(0);
            }
            let name = identifier(raw_name).0;
            parameters.push(if bounds.is_empty() {
                name
            } else {
                format!("{name} extends {}", bounds.join(" & "))
            });
        }
        self.expect(b'>')?;
        if parameters.is_empty() {
            return Err(self.error("empty formal type-parameter list"));
        }
        Ok(format!("<{}>", parameters.join(", ")))
    }

    fn java_type(&mut self) -> Result<String, String> {
        let primitive = match self.peek() {
            Some(b'B') => Some("byte"),
            Some(b'C') => Some("char"),
            Some(b'D') => Some("double"),
            Some(b'F') => Some("float"),
            Some(b'I') => Some("int"),
            Some(b'J') => Some("long"),
            Some(b'S') => Some("short"),
            Some(b'Z') => Some("boolean"),
            _ => None,
        };
        if let Some(primitive) = primitive {
            self.take();
            Ok(primitive.to_owned())
        } else {
            self.reference_type()
        }
    }

    fn reference_type(&mut self) -> Result<String, String> {
        match self.peek() {
            Some(b'L') => self.class_type(),
            Some(b'T') => self.type_variable(),
            Some(b'[') => {
                self.take();
                Ok(format!("{}[]", self.java_type()?))
            }
            _ => Err(self.error("expected reference type signature")),
        }
    }

    fn type_variable(&mut self) -> Result<String, String> {
        self.expect(b'T')?;
        let name = self.read_until(b";")?;
        self.expect(b';')?;
        Ok(identifier(name).0)
    }

    fn class_type(&mut self) -> Result<String, String> {
        self.expect(b'L')?;
        let raw = self.read_until(b"<.;")?;
        let mut rendered = self.names.class_name(raw);
        if self.peek() == Some(b'<') {
            rendered.push_str(&self.type_arguments()?);
        }
        while self.peek() == Some(b'.') {
            self.take();
            let segment = self.read_until(b"<.;")?;
            rendered.push('.');
            rendered.push_str(&identifier(segment).0);
            if self.peek() == Some(b'<') {
                rendered.push_str(&self.type_arguments()?);
            }
        }
        self.expect(b';')?;
        Ok(rendered)
    }

    fn type_arguments(&mut self) -> Result<String, String> {
        self.expect(b'<')?;
        let mut arguments = Vec::new();
        while self.peek() != Some(b'>') {
            if self.finished() {
                return Err(self.error("unterminated type arguments"));
            }
            arguments.push(match self.peek() {
                Some(b'*') => {
                    self.take();
                    "?".to_owned()
                }
                Some(b'+') => {
                    self.take();
                    format!("? extends {}", self.reference_type()?)
                }
                Some(b'-') => {
                    self.take();
                    format!("? super {}", self.reference_type()?)
                }
                _ => self.reference_type()?,
            });
        }
        self.expect(b'>')?;
        if arguments.is_empty() {
            return Err(self.error("empty type-argument list"));
        }
        Ok(format!("<{}>", arguments.join(", ")))
    }

    fn read_until(&mut self, delimiters: &[u8]) -> Result<&'a str, String> {
        let start = self.offset;
        while let Some(value) = self.peek() {
            if delimiters.contains(&value) {
                break;
            }
            self.offset += 1;
        }
        if self.offset == start {
            return Err(self.error("expected a non-empty identifier"));
        }
        self.value
            .get(start..self.offset)
            .ok_or_else(|| self.error("identifier is not valid UTF-8"))
    }
}

#[cfg(test)]
mod tests {
    use java::classfile::{ClassAccessFlags, ClassFile, JAVA_8_MAJOR_VERSION};

    use super::{parse_class, parse_field, parse_method};
    use crate::names::SourceNames;

    fn names() -> SourceNames {
        SourceNames::from_class(
            &ClassFile::new(
                JAVA_8_MAJOR_VERSION,
                "sample/Owner",
                Some("java/lang/Object"),
                ClassAccessFlags::PUBLIC | ClassAccessFlags::SUPER,
            )
            .expect("create class"),
        )
        .expect("source names")
    }

    #[test]
    fn renders_class_and_field_type_arguments() {
        let names = names();
        let class = parse_class(
            "<T:Ljava/lang/Object;>Ljava/lang/Object;Ljava/util/Comparator<TT;>;",
            &names,
        )
        .expect("class signature");
        assert_eq!(class.type_parameters, "<T>");
        assert_eq!(class.interfaces, ["java.util.Comparator<T>"]);
        assert_eq!(
            parse_field("Ljava/util/List<+Ljava/lang/Number;>;", &names).expect("field signature"),
            "java.util.List<? extends java.lang.Number>"
        );
    }

    #[test]
    fn renders_method_arrays_bounds_and_throws() {
        let names = names();
        let method = parse_method(
            "<T:Ljava/lang/Object;:Ljava/lang/Comparable<TT;>;>([TT;)TT;^Ljava/io/IOException;",
            &names,
        )
        .expect("method signature");
        assert_eq!(
            method.type_parameters,
            "<T extends java.lang.Comparable<T>>"
        );
        assert_eq!(method.parameters, ["T[]"]);
        assert_eq!(method.return_type, "T");
        assert_eq!(method.throws, ["java.io.IOException"]);
    }
}

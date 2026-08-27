//! Deterministic Java names, types, and literal escaping.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use disassembler::ExactText;
use java::classfile::{ClassFile, KnownAttribute, KnownAttributeKind};
use java::descriptor::{JavaType, ReturnType, parse_field};

use crate::Result;

pub(crate) fn package_and_simple(internal_name: &str) -> (Option<String>, String) {
    match internal_name.rsplit_once('/') {
        Some((package, simple)) => (
            Some(
                package
                    .split('/')
                    .map(|segment| identifier(segment).0)
                    .collect::<Vec<_>>()
                    .join("."),
            ),
            simple.to_owned(),
        ),
        None => (None, internal_name.to_owned()),
    }
}

/// Returns the forward-slash relative source path used for a JVM class name.
///
/// Package and class segments use the same deterministic Java-identifier
/// escaping as the source renderer, so the final file name matches the emitted
/// top-level declaration.
#[must_use]
pub fn compilation_unit_path(internal_name: &str) -> String {
    let mut segments = internal_name
        .split('/')
        .map(|segment| identifier(segment).0)
        .collect::<Vec<_>>();
    if let Some(file_name) = segments.last_mut() {
        file_name.push_str(".java");
    }
    segments.join("/")
}

fn source_class_name(internal_name: &str) -> String {
    internal_name
        .split(['/', '.'])
        .map(|segment| identifier(segment).0)
        .collect::<Vec<_>>()
        .join(".")
}

#[derive(Debug, Default)]
pub(crate) struct SourceNames {
    nested: BTreeMap<String, NestedName>,
}

#[derive(Debug)]
struct NestedName {
    outer: String,
    simple: String,
}

impl SourceNames {
    pub(crate) fn from_class(class: &ClassFile) -> Result<Self> {
        let mut names = Self::from_classes(core::iter::once(class))?;
        // A lone member class is emitted as a legal top-level declaration
        // whose binary `$` name is retained. Its own references must use the
        // same name. Archive compilation-unit recovery supplies the enclosing
        // class and deliberately retains this mapping instead.
        names.nested.remove(class.class_name()?);
        Ok(names)
    }

    pub(crate) fn from_classes<'a>(
        classes: impl IntoIterator<Item = &'a ClassFile>,
    ) -> Result<Self> {
        let mut nested = BTreeMap::new();
        for class in classes {
            let Some(KnownAttribute::InnerClasses(attribute)) =
                class.known_attribute(KnownAttributeKind::InnerClasses)
            else {
                continue;
            };
            for entry in &attribute.classes {
                if entry.outer_class_info_index == 0 || entry.inner_name_index == 0 {
                    continue;
                }
                let inner = class
                    .constant_pool
                    .class_name(entry.inner_class_info_index)?
                    .to_owned();
                let outer = class
                    .constant_pool
                    .class_name(entry.outer_class_info_index)?
                    .to_owned();
                let simple = identifier(class.constant_pool.utf8(entry.inner_name_index)?).0;
                nested.insert(inner, NestedName { outer, simple });
            }
        }
        Ok(Self { nested })
    }

    pub(crate) fn class_name(&self, internal_name: &str) -> String {
        let mut current = internal_name;
        let mut nested_parts = Vec::new();
        let mut visited = BTreeSet::new();
        while let Some(entry) = self.nested.get(current) {
            if !visited.insert(current) {
                return source_class_name(internal_name);
            }
            nested_parts.push(entry.simple.as_str());
            current = &entry.outer;
        }
        let mut rendered = source_class_name(current);
        for part in nested_parts.into_iter().rev() {
            rendered.push('.');
            rendered.push_str(part);
        }
        rendered
    }

    pub(crate) fn type_descriptor(&self, descriptor: &str) -> Result<String> {
        Ok(self.value_type(&parse_field(descriptor)?))
    }

    pub(crate) fn value_type(&self, value: &JavaType) -> String {
        match value {
            JavaType::Byte => "byte".to_owned(),
            JavaType::Char => "char".to_owned(),
            JavaType::Double => "double".to_owned(),
            JavaType::Float => "float".to_owned(),
            JavaType::Int => "int".to_owned(),
            JavaType::Long => "long".to_owned(),
            JavaType::Short => "short".to_owned(),
            JavaType::Boolean => "boolean".to_owned(),
            JavaType::Object(name) => self.class_name(name),
            JavaType::Array(element) => format!("{}[]", self.value_type(element)),
        }
    }

    pub(crate) fn return_type(&self, value: &ReturnType) -> String {
        match value {
            ReturnType::Void => "void".to_owned(),
            ReturnType::Type(value) => self.value_type(value),
        }
    }
}

pub(crate) fn default_value(value: &JavaType) -> &'static str {
    match value {
        JavaType::Boolean => "false",
        JavaType::Byte | JavaType::Char | JavaType::Int | JavaType::Short => "0",
        JavaType::Long => "0L",
        JavaType::Float => "0.0f",
        JavaType::Double => "0.0d",
        JavaType::Object(_) | JavaType::Array(_) => "null",
    }
}

pub(crate) fn identifier(value: &str) -> (String, bool) {
    if valid_identifier(value) && !JAVA_KEYWORDS.contains(&value) {
        return (value.to_owned(), false);
    }
    let mut result = String::from("cafe_");
    for (position, character) in value.chars().enumerate() {
        let valid = if position == 0 {
            character == '_' || character == '$' || character.is_alphabetic()
        } else {
            character == '_'
                || character == '$'
                || character.is_alphanumeric()
                || character.is_alphabetic()
        };
        if valid {
            result.push(character);
        } else {
            write!(result, "u{:04x}", u32::from(character))
                .expect("writing to a String cannot fail");
        }
    }
    if JAVA_KEYWORDS.contains(&result.as_str()) {
        result.push('_');
    }
    (result, true)
}

fn valid_identifier(value: &str) -> bool {
    let mut characters = value.chars();
    let Some(first) = characters.next() else {
        return false;
    };
    (first == '_' || first == '$' || first.is_alphabetic())
        && characters.all(|character| {
            character == '_'
                || character == '$'
                || character.is_alphanumeric()
                || character.is_alphabetic()
        })
}

pub(crate) fn string_literal(value: &ExactText) -> String {
    utf16_literal(&value.utf16_units)
}

pub(crate) fn rust_string_literal(value: &str) -> String {
    utf16_literal(&value.encode_utf16().collect::<Vec<_>>())
}

fn utf16_literal(units: &[u16]) -> String {
    let mut output = String::from("\"");
    for &unit in units {
        match unit {
            0x08 => output.push_str("\\b"),
            0x09 => output.push_str("\\t"),
            0x0a => output.push_str("\\n"),
            0x0c => output.push_str("\\f"),
            0x0d => output.push_str("\\r"),
            0x22 => output.push_str("\\\""),
            0x27 => output.push_str("\\'"),
            0x5c => output.push_str("\\\\"),
            0x20..=0x7e => output.push(char::from_u32(u32::from(unit)).unwrap_or('?')),
            _ => write!(output, "\\u{unit:04x}").expect("writing to a String cannot fail"),
        }
    }
    output.push('"');
    output
}

const JAVA_KEYWORDS: &[&str] = &[
    "abstract",
    "assert",
    "boolean",
    "break",
    "byte",
    "case",
    "catch",
    "char",
    "class",
    "const",
    "continue",
    "default",
    "do",
    "double",
    "else",
    "enum",
    "extends",
    "false",
    "final",
    "finally",
    "float",
    "for",
    "goto",
    "if",
    "implements",
    "import",
    "instanceof",
    "int",
    "interface",
    "long",
    "native",
    "new",
    "null",
    "package",
    "private",
    "protected",
    "public",
    "return",
    "short",
    "static",
    "strictfp",
    "super",
    "switch",
    "synchronized",
    "this",
    "throw",
    "throws",
    "transient",
    "true",
    "try",
    "void",
    "volatile",
    "while",
    "_",
    "exports",
    "module",
    "non-sealed",
    "open",
    "opens",
    "permits",
    "provides",
    "record",
    "requires",
    "sealed",
    "to",
    "transitive",
    "uses",
    "var",
    "when",
    "with",
    "yield",
];

#[cfg(test)]
mod tests {
    use super::compilation_unit_path;

    #[test]
    fn compilation_unit_paths_match_escaped_source_names() {
        assert_eq!(
            compilation_unit_path("example/class"),
            "example/cafe_class.java"
        );
        assert_eq!(
            compilation_unit_path("package/Type-Name"),
            "cafe_package/cafe_Typeu002dName.java"
        );
    }
}

//! Deterministic Java names, types, and literal escaping.

use std::fmt::Write as _;

use disassembler::ExactText;
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

pub(crate) fn source_class_name(internal_name: &str) -> String {
    internal_name
        .split(['/', '.'])
        .map(|segment| identifier(segment).0)
        .collect::<Vec<_>>()
        .join(".")
}

pub(crate) fn source_type_descriptor(descriptor: &str) -> Result<String> {
    Ok(parse_field(descriptor)?.to_string())
}

pub(crate) fn source_type(value: &JavaType) -> String {
    value.to_string()
}

pub(crate) fn source_return_type(value: &ReturnType) -> String {
    value.to_string()
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

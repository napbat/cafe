//! Default symbolic resolver for constant-pool displays retained by disassembly.

use disassembler::{ExactText, Reference, ReferenceKind, ReferenceSymbol};

use super::{JavaReferenceResolutionError, JavaReferenceResolver};
use crate::classfile::{Constant, ConstantPool};

/// Re-interns losslessly parseable symbolic displays retained by Cafe's JVM adapter.
///
/// The resolver supports classes, strings (including exact escaped UTF-16),
/// fields, class/interface methods, method types, ordinary numeric constants,
/// and invokedynamic identities. Method handles and dynamic constants are
/// rejected because their display does not retain complete bootstrap metadata.
#[derive(Debug, Clone, Copy, Default)]
pub struct DisplayJavaReferenceResolver;

impl JavaReferenceResolver for DisplayJavaReferenceResolver {
    fn resolve(
        &mut self,
        reference: &Reference,
        pool: &mut ConstantPool,
    ) -> Result<u16, JavaReferenceResolutionError> {
        if let Some(symbol) = &reference.symbol {
            return resolve_symbol(reference.kind, symbol, pool);
        }
        let display = reference
            .display
            .as_deref()
            .ok_or_else(|| failure("reference has no resolved symbolic display"))?;
        match reference.kind {
            ReferenceKind::Type => pool
                .intern_class(strip_prefix(display, "Class ")?)
                .map_err(native),
            ReferenceKind::String => intern_string(pool, display),
            ReferenceKind::Field => {
                let (owner, name, descriptor) = member(display, "Field ")?;
                pool.intern_field_ref(owner, name, descriptor)
                    .map_err(native)
            }
            ReferenceKind::Method => {
                let (owner, name, descriptor) = member(display, "Method ")?;
                pool.intern_method_ref(owner, name, descriptor)
                    .map_err(native)
            }
            ReferenceKind::InterfaceMethod => {
                let (owner, name, descriptor) = member(display, "InterfaceMethod ")?;
                pool.intern_interface_method_ref(owner, name, descriptor)
                    .map_err(native)
            }
            ReferenceKind::DynamicCallSite => dynamic_call_site(pool, display),
            ReferenceKind::Constant => constant(pool, display),
            ReferenceKind::MethodPrototype | ReferenceKind::MethodHandle => Err(failure(
                "reference kind is not a JVM constant-pool instruction operand",
            )),
        }
    }
}

fn resolve_symbol(
    kind: ReferenceKind,
    symbol: &ReferenceSymbol,
    pool: &mut ConstantPool,
) -> Result<u16, JavaReferenceResolutionError> {
    match symbol {
        ReferenceSymbol::Integer(value) => pool.intern(Constant::Integer(*value)).map_err(native),
        ReferenceSymbol::Float(bits) => pool
            .intern(Constant::Float(f32::from_bits(*bits)))
            .map_err(native),
        ReferenceSymbol::Long(value) => pool.intern(Constant::Long(*value)).map_err(native),
        ReferenceSymbol::Double(bits) => pool
            .intern(Constant::Double(f64::from_bits(*bits)))
            .map_err(native),
        ReferenceSymbol::String(value) => intern_exact_string(pool, value),
        ReferenceSymbol::Type(name) => pool.intern_class(name).map_err(native),
        ReferenceSymbol::Field {
            owner,
            name,
            descriptor,
        } => intern_member(pool, owner, name, descriptor, ReferenceKind::Field),
        ReferenceSymbol::Method {
            owner,
            name,
            descriptor,
        } => intern_member(pool, owner, name, descriptor, kind),
        ReferenceSymbol::MethodPrototype(descriptor) => {
            pool.intern_method_type(descriptor).map_err(native)
        }
    }
}

fn intern_exact_string(
    pool: &mut ConstantPool,
    value: &ExactText,
) -> Result<u16, JavaReferenceResolutionError> {
    let string_index = pool
        .intern_utf16(value.utf16_units.clone())
        .map_err(native)?;
    pool.intern(Constant::String { string_index })
        .map_err(native)
}

fn intern_member(
    pool: &mut ConstantPool,
    owner: &str,
    name: &ExactText,
    descriptor: &str,
    kind: ReferenceKind,
) -> Result<u16, JavaReferenceResolutionError> {
    let class_index = pool.intern_class(owner).map_err(native)?;
    let name_index = pool
        .intern_utf16(name.utf16_units.clone())
        .map_err(native)?;
    let descriptor_index = pool.intern_utf8(descriptor).map_err(native)?;
    let name_and_type_index = pool
        .intern(Constant::NameAndType {
            name_index,
            descriptor_index,
        })
        .map_err(native)?;
    let constant = match kind {
        ReferenceKind::Field => Constant::FieldRef {
            class_index,
            name_and_type_index,
        },
        ReferenceKind::Method => Constant::MethodRef {
            class_index,
            name_and_type_index,
        },
        ReferenceKind::InterfaceMethod => Constant::InterfaceMethodRef {
            class_index,
            name_and_type_index,
        },
        _ => return Err(failure("symbolic member kind is incompatible with JVM")),
    };
    pool.intern(constant).map_err(native)
}

fn intern_string(
    pool: &mut ConstantPool,
    display: &str,
) -> Result<u16, JavaReferenceResolutionError> {
    let quoted = strip_prefix(display, "String ")?;
    let units = unquote_utf16(quoted)?;
    let string_index = pool.intern_utf16(units).map_err(native)?;
    pool.intern(Constant::String { string_index })
        .map_err(native)
}

fn constant(pool: &mut ConstantPool, display: &str) -> Result<u16, JavaReferenceResolutionError> {
    if let Some(value) = display.strip_prefix("int ") {
        return value
            .parse::<i32>()
            .map_err(|_| failure("invalid displayed integer constant"))
            .and_then(|value| pool.intern(Constant::Integer(value)).map_err(native));
    }
    if let Some(value) = display.strip_prefix("long ") {
        return value
            .parse::<i64>()
            .map_err(|_| failure("invalid displayed long constant"))
            .and_then(|value| pool.intern(Constant::Long(value)).map_err(native));
    }
    if let Some(value) = display.strip_prefix("float ") {
        return parse_float32(value)
            .map(Constant::Float)
            .and_then(|value| pool.intern(value).map_err(native));
    }
    if let Some(value) = display.strip_prefix("double ") {
        return parse_float64(value)
            .map(Constant::Double)
            .and_then(|value| pool.intern(value).map_err(native));
    }
    if let Some(descriptor) = display.strip_prefix("MethodType ") {
        return pool.intern_method_type(descriptor).map_err(native);
    }
    Err(failure(
        "displayed constant does not retain a safely reconstructable JVM value",
    ))
}

fn dynamic_call_site(
    pool: &mut ConstantPool,
    display: &str,
) -> Result<u16, JavaReferenceResolutionError> {
    let rest = strip_prefix(display, "InvokeDynamic bootstrap#")?;
    let (bootstrap, symbol) = rest
        .split_once(':')
        .ok_or_else(|| failure("invalid invokedynamic display"))?;
    let bootstrap = bootstrap
        .parse::<u16>()
        .map_err(|_| failure("invalid invokedynamic bootstrap index"))?;
    let (name, descriptor) = symbol
        .split_once(':')
        .ok_or_else(|| failure("invokedynamic display lacks a descriptor"))?;
    pool.intern_invoke_dynamic(bootstrap, name, descriptor)
        .map_err(native)
}

fn member<'a>(
    display: &'a str,
    prefix: &str,
) -> Result<(&'a str, &'a str, &'a str), JavaReferenceResolutionError> {
    let rest = strip_prefix(display, prefix)?;
    let (owner, member) = rest
        .rsplit_once('.')
        .ok_or_else(|| failure("member display lacks an owner"))?;
    let (name, descriptor) = member
        .split_once(':')
        .ok_or_else(|| failure("member display lacks a descriptor"))?;
    Ok((owner, name, descriptor))
}

fn strip_prefix<'a>(value: &'a str, prefix: &str) -> Result<&'a str, JavaReferenceResolutionError> {
    value
        .strip_prefix(prefix)
        .ok_or_else(|| failure(format!("expected `{prefix}` symbolic display")))
}

fn parse_float32(value: &str) -> Result<f32, JavaReferenceResolutionError> {
    let parsed = value
        .parse::<f32>()
        .map_err(|_| failure("invalid displayed floating-point constant"))?;
    reject_nan(parsed.is_nan(), parsed)
}

fn parse_float64(value: &str) -> Result<f64, JavaReferenceResolutionError> {
    let parsed = value
        .parse::<f64>()
        .map_err(|_| failure("invalid displayed floating-point constant"))?;
    reject_nan(parsed.is_nan(), parsed)
}

fn reject_nan<T>(is_nan: bool, value: T) -> Result<T, JavaReferenceResolutionError> {
    if is_nan {
        Err(failure(
            "NaN display does not retain its original payload bits",
        ))
    } else {
        Ok(value)
    }
}

fn unquote_utf16(value: &str) -> Result<Vec<u16>, JavaReferenceResolutionError> {
    let inner = value
        .strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'))
        .ok_or_else(|| failure("string display is not quoted"))?;
    let mut output = Vec::new();
    let mut characters = inner.chars().peekable();
    while let Some(character) = characters.next() {
        if character != '\\' {
            let mut units = [0_u16; 2];
            output.extend_from_slice(character.encode_utf16(&mut units));
            continue;
        }
        let escape = characters
            .next()
            .ok_or_else(|| failure("trailing backslash in string display"))?;
        match escape {
            '0' => output.push(0),
            't' => output.push(u16::from(b'\t')),
            'r' => output.push(u16::from(b'\r')),
            'n' => output.push(u16::from(b'\n')),
            '\\' | '\'' | '"' => output.push(escape as u16),
            'u' => output.extend(parse_unicode_escape(&mut characters)?),
            _ => return Err(failure("unsupported escape in string display")),
        }
    }
    Ok(output)
}

fn parse_unicode_escape(
    characters: &mut std::iter::Peekable<std::str::Chars<'_>>,
) -> Result<Vec<u16>, JavaReferenceResolutionError> {
    let braced = characters.next_if_eq(&'{').is_some();
    let mut digits = String::new();
    if braced {
        for character in characters.by_ref() {
            if character == '}' {
                break;
            }
            digits.push(character);
        }
    } else {
        for _ in 0..4 {
            digits.push(
                characters
                    .next()
                    .ok_or_else(|| failure("truncated Unicode escape"))?,
            );
        }
    }
    let value = u32::from_str_radix(&digits, 16)
        .map_err(|_| failure("invalid Unicode escape in string display"))?;
    if let Ok(unit) = u16::try_from(value) {
        return Ok(vec![unit]);
    }
    let character =
        char::from_u32(value).ok_or_else(|| failure("Unicode escape exceeds the scalar range"))?;
    Ok(character.encode_utf16(&mut [0_u16; 2]).to_vec())
}

#[allow(clippy::needless_pass_by_value)]
fn native(error: crate::Error) -> JavaReferenceResolutionError {
    failure(error.to_string())
}

fn failure(message: impl Into<String>) -> JavaReferenceResolutionError {
    JavaReferenceResolutionError::new(message)
}

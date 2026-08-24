//! DEX type, shorty, and member-name grammar.

use crate::file::{DexString, DexVersion};

/// Parsed descriptor category needed by semantic checks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum DescriptorKind {
    Void,
    Primitive { shorty: char, words: u8 },
    Class,
    Array,
}

impl DescriptorKind {
    pub(super) const fn is_void(self) -> bool {
        matches!(self, Self::Void)
    }

    pub(super) const fn is_class(self) -> bool {
        matches!(self, Self::Class)
    }

    pub(super) const fn is_primitive_or_void(self) -> bool {
        matches!(self, Self::Void | Self::Primitive { .. })
    }

    pub(super) const fn shorty(self) -> char {
        match self {
            Self::Void => 'V',
            Self::Primitive { shorty, .. } => shorty,
            Self::Class | Self::Array => 'L',
        }
    }

    pub(super) const fn words(self) -> u8 {
        match self {
            Self::Void => 0,
            Self::Primitive { words, .. } => words,
            Self::Class | Self::Array => 1,
        }
    }
}

pub(super) fn descriptor(
    value: &DexString,
    version: DexVersion,
) -> std::result::Result<DescriptorKind, String> {
    let units = &value.utf16_units;
    if units.len() == 1 {
        return primitive(units[0], true)
            .ok_or_else(|| "invalid one-unit type descriptor".to_owned());
    }
    let dimensions = units
        .iter()
        .take_while(|unit| **unit == u16::from(b'['))
        .count();
    if dimensions != 0 {
        if dimensions > 255 || dimensions >= units.len() {
            return Err("array descriptor has an invalid dimension count".to_owned());
        }
        let component = &units[dimensions..];
        if component.len() == 1 && primitive(component[0], false).is_some() {
            return Ok(DescriptorKind::Array);
        }
        parse_class(component, version)?;
        return Ok(DescriptorKind::Array);
    }
    parse_class(units, version)?;
    Ok(DescriptorKind::Class)
}

pub(super) fn shorty(value: &DexString) -> std::result::Result<(), String> {
    let mut units = value.utf16_units.iter().copied();
    let Some(first) = units.next() else {
        return Err("shorty descriptor is empty".to_owned());
    };
    if !is_shorty(first, true) || units.any(|unit| !is_shorty(unit, false)) {
        return Err("invalid shorty descriptor".to_owned());
    }
    Ok(())
}

pub(super) fn member_name(
    value: &DexString,
    version: DexVersion,
) -> std::result::Result<(), String> {
    let units = &value.utf16_units;
    let simple = if units.len() >= 3
        && units.first() == Some(&u16::from(b'<'))
        && units.last() == Some(&u16::from(b'>'))
    {
        &units[1..units.len() - 1]
    } else {
        units.as_slice()
    };
    validate_simple_name(simple, version)
}

fn parse_class(units: &[u16], version: DexVersion) -> std::result::Result<(), String> {
    if units.len() < 3
        || units.first() != Some(&u16::from(b'L'))
        || units.last() != Some(&u16::from(b';'))
    {
        return Err("invalid class type descriptor".to_owned());
    }
    for segment in units[1..units.len() - 1].split(|unit| *unit == u16::from(b'/')) {
        validate_simple_name(segment, version)?;
    }
    Ok(())
}

fn validate_simple_name(units: &[u16], version: DexVersion) -> std::result::Result<(), String> {
    if units.is_empty() {
        return Err("simple name is empty".to_owned());
    }
    for decoded in char::decode_utf16(units.iter().copied()) {
        let character = decoded.map_err(|_| "simple name contains an unpaired surrogate")?;
        if !simple_character(character, version) {
            return Err(format!(
                "simple name contains disallowed character U+{:04X}",
                u32::from(character)
            ));
        }
    }
    Ok(())
}

fn simple_character(character: char, version: DexVersion) -> bool {
    matches!(character, 'A'..='Z' | 'a'..='z' | '0'..='9' | '$' | '-' | '_')
        || (version >= DexVersion::V040 && character == ' ')
        || (version >= DexVersion::V040 && character == '\u{00a0}')
        || matches!(character, '\u{00a1}'..='\u{1fff}')
        || (version >= DexVersion::V040 && matches!(character, '\u{2000}'..='\u{200a}'))
        || matches!(character, '\u{2010}'..='\u{2027}')
        || (version >= DexVersion::V040 && character == '\u{202f}')
        || matches!(character, '\u{2030}'..='\u{d7ff}' | '\u{e000}'..='\u{ffef}' | '\u{10000}'..='\u{10ffff}')
}

fn primitive(unit: u16, allow_void: bool) -> Option<DescriptorKind> {
    match u8::try_from(unit).ok()? {
        b'V' if allow_void => Some(DescriptorKind::Void),
        b'Z' => Some(DescriptorKind::Primitive {
            shorty: 'Z',
            words: 1,
        }),
        b'B' => Some(DescriptorKind::Primitive {
            shorty: 'B',
            words: 1,
        }),
        b'S' => Some(DescriptorKind::Primitive {
            shorty: 'S',
            words: 1,
        }),
        b'C' => Some(DescriptorKind::Primitive {
            shorty: 'C',
            words: 1,
        }),
        b'I' => Some(DescriptorKind::Primitive {
            shorty: 'I',
            words: 1,
        }),
        b'J' => Some(DescriptorKind::Primitive {
            shorty: 'J',
            words: 2,
        }),
        b'F' => Some(DescriptorKind::Primitive {
            shorty: 'F',
            words: 1,
        }),
        b'D' => Some(DescriptorKind::Primitive {
            shorty: 'D',
            words: 2,
        }),
        _ => None,
    }
}

fn is_shorty(unit: u16, allow_void: bool) -> bool {
    matches!(
        u8::try_from(unit),
        Ok(b'Z' | b'B' | b'S' | b'C' | b'I' | b'J' | b'F' | b'D' | b'L')
    ) || (allow_void && unit == u16::from(b'V'))
}

//! DEX type, shorty, and member-name grammar.

use crate::file::{DexString, DexVersion};

const DESCRIPTOR_TAG_UNIT_COUNT: usize = 1;
const FIRST_DESCRIPTOR_UNIT_INDEX: usize = 0;
const NO_ARRAY_DIMENSIONS: usize = 0;
const MAX_ARRAY_DIMENSIONS: usize = u8::MAX as usize;
const MINIMUM_CLASS_DESCRIPTOR_UNITS: usize = 3;
const CLASS_NAME_START_INDEX: usize = DESCRIPTOR_TAG_UNIT_COUNT;
const CLASS_NAME_TERMINATOR_WIDTH: usize = DESCRIPTOR_TAG_UNIT_COUNT;
const CLASS_NAME_TERMINATOR: u16 = b';' as u16;
const CLASS_PACKAGE_SEPARATOR: u16 = b'/' as u16;
const MEMBER_NAME_OPEN: u16 = b'<' as u16;
const MEMBER_NAME_CLOSE: u16 = b'>' as u16;

const ASCII_UPPERCASE_START: char = 'A';
const ASCII_UPPERCASE_END: char = 'Z';
const ASCII_LOWERCASE_START: char = 'a';
const ASCII_LOWERCASE_END: char = 'z';
const ASCII_DIGIT_START: char = '0';
const ASCII_DIGIT_END: char = '9';
const SIMPLE_NAME_DOLLAR: char = '$';
const SIMPLE_NAME_HYPHEN: char = '-';
const SIMPLE_NAME_UNDERSCORE: char = '_';
const SIMPLE_NAME_SPACE: char = ' ';
const SIMPLE_NAME_NO_BREAK_SPACE: char = '\u{00a0}';
const SIMPLE_NAME_LEGACY_START: char = '\u{00a1}';
const SIMPLE_NAME_LEGACY_END: char = '\u{1fff}';
const SIMPLE_NAME_SPACE_BLOCK_START: char = '\u{2000}';
const SIMPLE_NAME_SPACE_BLOCK_END: char = '\u{200a}';
const SIMPLE_NAME_PUNCTUATION_START: char = '\u{2010}';
const SIMPLE_NAME_PUNCTUATION_END: char = '\u{2027}';
const SIMPLE_NAME_NARROW_NO_BREAK_SPACE: char = '\u{202f}';
const SIMPLE_NAME_BMP_START: char = '\u{2030}';
const SIMPLE_NAME_BEFORE_SURROGATES_END: char = '\u{d7ff}';
const SIMPLE_NAME_AFTER_SURROGATES_START: char = '\u{e000}';
const SIMPLE_NAME_BMP_END: char = '\u{ffef}';
const SIMPLE_NAME_SUPPLEMENTARY_START: char = '\u{10000}';
const SIMPLE_NAME_SUPPLEMENTARY_END: char = '\u{10ffff}';

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub(super) enum DescriptorTag {
    Void = b'V',
    Boolean = b'Z',
    Byte = b'B',
    Short = b'S',
    Char = b'C',
    Int = b'I',
    Long = b'J',
    Float = b'F',
    Double = b'D',
    Class = b'L',
    Array = b'[',
}

impl DescriptorTag {
    const fn from_unit(unit: u16) -> Option<Self> {
        match unit {
            unit if unit == Self::Void as u16 => Some(Self::Void),
            unit if unit == Self::Boolean as u16 => Some(Self::Boolean),
            unit if unit == Self::Byte as u16 => Some(Self::Byte),
            unit if unit == Self::Short as u16 => Some(Self::Short),
            unit if unit == Self::Char as u16 => Some(Self::Char),
            unit if unit == Self::Int as u16 => Some(Self::Int),
            unit if unit == Self::Long as u16 => Some(Self::Long),
            unit if unit == Self::Float as u16 => Some(Self::Float),
            unit if unit == Self::Double as u16 => Some(Self::Double),
            unit if unit == Self::Class as u16 => Some(Self::Class),
            unit if unit == Self::Array as u16 => Some(Self::Array),
            _ => None,
        }
    }

    const fn shorty(self) -> char {
        match self {
            Self::Void => 'V',
            Self::Boolean => 'Z',
            Self::Byte => 'B',
            Self::Short => 'S',
            Self::Char => 'C',
            Self::Int => 'I',
            Self::Long => 'J',
            Self::Float => 'F',
            Self::Double => 'D',
            Self::Class | Self::Array => 'L',
        }
    }

    const fn is_primitive(self) -> bool {
        matches!(
            self,
            Self::Boolean
                | Self::Byte
                | Self::Short
                | Self::Char
                | Self::Int
                | Self::Long
                | Self::Float
                | Self::Double
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RegisterWidth {
    Single,
    Double,
}

impl RegisterWidth {
    pub(super) const fn words(self) -> u8 {
        match self {
            Self::Single => 1,
            Self::Double => 2,
        }
    }
}

/// Parsed descriptor category needed by semantic checks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum DescriptorKind {
    Void,
    Primitive { tag: DescriptorTag },
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
            Self::Void => DescriptorTag::Void.shorty(),
            Self::Primitive { tag } => tag.shorty(),
            Self::Class | Self::Array => DescriptorTag::Class.shorty(),
        }
    }

    pub(super) const fn register_width(self) -> Option<RegisterWidth> {
        match self {
            Self::Void => None,
            Self::Primitive {
                tag: DescriptorTag::Long | DescriptorTag::Double,
            } => Some(RegisterWidth::Double),
            Self::Primitive { .. } | Self::Class | Self::Array => Some(RegisterWidth::Single),
        }
    }
}

pub(super) fn descriptor(
    value: &DexString,
    version: DexVersion,
) -> std::result::Result<DescriptorKind, String> {
    let units = &value.utf16_units;
    if units.len() == DESCRIPTOR_TAG_UNIT_COUNT {
        return primitive(units[FIRST_DESCRIPTOR_UNIT_INDEX], true)
            .ok_or_else(|| "invalid one-unit type descriptor".to_owned());
    }
    let dimensions = units
        .iter()
        .take_while(|unit| DescriptorTag::from_unit(**unit) == Some(DescriptorTag::Array))
        .count();
    if dimensions != NO_ARRAY_DIMENSIONS {
        if dimensions > MAX_ARRAY_DIMENSIONS || dimensions >= units.len() {
            return Err("array descriptor has an invalid dimension count".to_owned());
        }
        let component = &units[dimensions..];
        if component.len() == DESCRIPTOR_TAG_UNIT_COUNT
            && primitive(component[FIRST_DESCRIPTOR_UNIT_INDEX], false).is_some()
        {
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
    let simple = if units.len() >= MINIMUM_CLASS_DESCRIPTOR_UNITS
        && units.first() == Some(&MEMBER_NAME_OPEN)
        && units.last() == Some(&MEMBER_NAME_CLOSE)
    {
        &units[CLASS_NAME_START_INDEX..units.len() - CLASS_NAME_TERMINATOR_WIDTH]
    } else {
        units.as_slice()
    };
    validate_simple_name(simple, version)
}

fn parse_class(units: &[u16], version: DexVersion) -> std::result::Result<(), String> {
    if units.len() < MINIMUM_CLASS_DESCRIPTOR_UNITS
        || units.first().copied().and_then(DescriptorTag::from_unit) != Some(DescriptorTag::Class)
        || units.last() != Some(&CLASS_NAME_TERMINATOR)
    {
        return Err("invalid class type descriptor".to_owned());
    }
    for segment in units[CLASS_NAME_START_INDEX..units.len() - CLASS_NAME_TERMINATOR_WIDTH]
        .split(|unit| *unit == CLASS_PACKAGE_SEPARATOR)
    {
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
    (ASCII_UPPERCASE_START..=ASCII_UPPERCASE_END).contains(&character)
        || (ASCII_LOWERCASE_START..=ASCII_LOWERCASE_END).contains(&character)
        || (ASCII_DIGIT_START..=ASCII_DIGIT_END).contains(&character)
        || matches!(
            character,
            SIMPLE_NAME_DOLLAR | SIMPLE_NAME_HYPHEN | SIMPLE_NAME_UNDERSCORE
        )
        || (version >= DexVersion::V040 && character == SIMPLE_NAME_SPACE)
        || (version >= DexVersion::V040 && character == SIMPLE_NAME_NO_BREAK_SPACE)
        || (SIMPLE_NAME_LEGACY_START..=SIMPLE_NAME_LEGACY_END).contains(&character)
        || (version >= DexVersion::V040
            && (SIMPLE_NAME_SPACE_BLOCK_START..=SIMPLE_NAME_SPACE_BLOCK_END).contains(&character))
        || (SIMPLE_NAME_PUNCTUATION_START..=SIMPLE_NAME_PUNCTUATION_END).contains(&character)
        || (version >= DexVersion::V040 && character == SIMPLE_NAME_NARROW_NO_BREAK_SPACE)
        || (SIMPLE_NAME_BMP_START..=SIMPLE_NAME_BEFORE_SURROGATES_END).contains(&character)
        || (SIMPLE_NAME_AFTER_SURROGATES_START..=SIMPLE_NAME_BMP_END).contains(&character)
        || (SIMPLE_NAME_SUPPLEMENTARY_START..=SIMPLE_NAME_SUPPLEMENTARY_END).contains(&character)
}

fn primitive(unit: u16, allow_void: bool) -> Option<DescriptorKind> {
    let tag = DescriptorTag::from_unit(unit)?;
    match tag {
        DescriptorTag::Void if allow_void => Some(DescriptorKind::Void),
        tag if tag.is_primitive() => Some(DescriptorKind::Primitive { tag }),
        _ => None,
    }
}

fn is_shorty(unit: u16, allow_void: bool) -> bool {
    DescriptorTag::from_unit(unit).is_some_and(|tag| {
        tag.is_primitive()
            || tag == DescriptorTag::Class
            || (allow_void && tag == DescriptorTag::Void)
    })
}

//! ART quickening tables and canonical Dalvik restoration.

use dex::instruction::{Instruction, Opcode, PayloadKind};

use crate::{Error, Result};

const RETURN_VOID_NO_BARRIER: u8 = 0x73;
const QUICKENING_SENTINEL: u16 = u16::MAX;
const CHECK_CAST_OPCODE: u8 = 0x1f;
const ELEMENTS_PER_ULEB_GROUP: u32 = 7;
const ULEB_PAYLOAD_MASK: u8 = 0x7f;
const ULEB_CONTINUATION: u8 = 0x80;

/// ART Android 9 quickened opcode with its canonical replacement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum QuickOpcode {
    /// Quick instance integer field load.
    Iget = 0xe3,
    /// Quick instance wide field load.
    IgetWide = 0xe4,
    /// Quick instance reference field load.
    IgetObject = 0xe5,
    /// Quick instance integer field store.
    Iput = 0xe6,
    /// Quick instance wide field store.
    IputWide = 0xe7,
    /// Quick instance reference field store.
    IputObject = 0xe8,
    /// Quick virtual invocation.
    InvokeVirtual = 0xe9,
    /// Quick range virtual invocation.
    InvokeVirtualRange = 0xea,
    /// Quick boolean field store.
    IputBoolean = 0xeb,
    /// Quick byte field store.
    IputByte = 0xec,
    /// Quick character field store.
    IputChar = 0xed,
    /// Quick short field store.
    IputShort = 0xee,
    /// Quick boolean field load.
    IgetBoolean = 0xef,
    /// Quick byte field load.
    IgetByte = 0xf0,
    /// Quick character field load.
    IgetChar = 0xf1,
    /// Quick short field load.
    IgetShort = 0xf2,
}

impl QuickOpcode {
    /// Parses one Android 9 quick opcode byte.
    #[must_use]
    pub const fn from_byte(byte: u8) -> Option<Self> {
        Some(match byte {
            0xe3 => Self::Iget,
            0xe4 => Self::IgetWide,
            0xe5 => Self::IgetObject,
            0xe6 => Self::Iput,
            0xe7 => Self::IputWide,
            0xe8 => Self::IputObject,
            0xe9 => Self::InvokeVirtual,
            0xea => Self::InvokeVirtualRange,
            0xeb => Self::IputBoolean,
            0xec => Self::IputByte,
            0xed => Self::IputChar,
            0xee => Self::IputShort,
            0xef => Self::IgetBoolean,
            0xf0 => Self::IgetByte,
            0xf1 => Self::IgetChar,
            0xf2 => Self::IgetShort,
            _ => return None,
        })
    }

    /// Returns the standard opcode restored by ART.
    #[must_use]
    pub const fn canonical(self) -> Opcode {
        match self {
            Self::Iget => Opcode::Iget,
            Self::IgetWide => Opcode::IgetWide,
            Self::IgetObject => Opcode::IgetObject,
            Self::Iput => Opcode::Iput,
            Self::IputWide => Opcode::IputWide,
            Self::IputObject => Opcode::IputObject,
            Self::InvokeVirtual => Opcode::InvokeVirtual,
            Self::InvokeVirtualRange => Opcode::InvokeVirtualRange,
            Self::IputBoolean => Opcode::IputBoolean,
            Self::IputByte => Opcode::IputByte,
            Self::IputChar => Opcode::IputChar,
            Self::IputShort => Opcode::IputShort,
            Self::IgetBoolean => Opcode::IgetBoolean,
            Self::IgetByte => Opcode::IgetByte,
            Self::IgetChar => Opcode::IgetChar,
            Self::IgetShort => Opcode::IgetShort,
        }
    }

    /// Returns the fixed encoded width in code units.
    #[must_use]
    pub const fn code_units(self) -> usize {
        match self {
            Self::Iget
            | Self::IgetWide
            | Self::IgetObject
            | Self::Iput
            | Self::IputWide
            | Self::IputObject
            | Self::IputBoolean
            | Self::IputByte
            | Self::IputChar
            | Self::IputShort
            | Self::IgetBoolean
            | Self::IgetByte
            | Self::IgetChar
            | Self::IgetShort => 2,
            Self::InvokeVirtual | Self::InvokeVirtualRange => 3,
        }
    }
}

/// One method's VDEX quickening indices.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct QuickeningInfo {
    indices: Vec<u16>,
}

impl QuickeningInfo {
    /// Builds an exact index stream.
    #[must_use]
    pub const fn new(indices: Vec<u16>) -> Self {
        Self { indices }
    }

    /// Parses an exact quickening record.
    ///
    /// # Errors
    ///
    /// Returns an error for malformed LEB counts, truncated indices, or
    /// trailing bytes.
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        let (record, consumed) = Self::parse_prefix(bytes)?;
        if consumed == bytes.len() {
            Ok(record)
        } else {
            Err(Error::quickening(
                consumed,
                "quickening record has trailing bytes",
            ))
        }
    }

    /// Parses one record from the beginning of a larger quickening section.
    ///
    /// # Errors
    ///
    /// Returns an error for malformed counts or truncated 16-bit indices.
    pub fn parse_prefix(bytes: &[u8]) -> Result<(Self, usize)> {
        let mut cursor = 0;
        let count = read_uleb(bytes, &mut cursor)?;
        let count = usize::try_from(count)
            .map_err(|_| Error::quickening(cursor, "quickening count is too large"))?;
        let byte_count = count
            .checked_mul(2)
            .ok_or_else(|| Error::quickening(cursor, "quickening index size overflowed"))?;
        let end = cursor
            .checked_add(byte_count)
            .ok_or_else(|| Error::quickening(cursor, "quickening record range overflowed"))?;
        let raw = bytes
            .get(cursor..end)
            .ok_or_else(|| Error::quickening(cursor, "truncated quickening index array"))?;
        let indices = raw
            .as_chunks::<2>()
            .0
            .iter()
            .map(|word| u16::from_le_bytes(*word))
            .collect();
        Ok((Self { indices }, end))
    }

    /// Returns the exact index sequence.
    #[must_use]
    pub fn indices(&self) -> &[u16] {
        &self.indices
    }

    /// Encodes this method record.
    ///
    /// # Errors
    ///
    /// Returns an error when the number of indices exceeds 32 bits.
    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        let count = u32::try_from(self.indices.len())
            .map_err(|_| Error::quickening(0, "quickening index count exceeds 32 bits"))?;
        let mut output = Vec::new();
        write_uleb(&mut output, count);
        for index in &self.indices {
            output.extend_from_slice(&index.to_le_bytes());
        }
        Ok(output)
    }

    pub(crate) fn parse_legacy_for_code(bytes: &[u8], code_units: &[u16]) -> Result<Self> {
        let mut indices = Vec::new();
        let mut byte_cursor = 0usize;
        let mut code_cursor = 0usize;
        while code_cursor < code_units.len() {
            if let Some(width) = payload_width(code_units, code_cursor)? {
                code_cursor = code_cursor
                    .checked_add(width)
                    .ok_or_else(|| Error::quickening(code_cursor, "payload cursor overflowed"))?;
                continue;
            }
            let opcode = code_units[code_cursor].to_le_bytes()[0];
            if let Some(quick) = QuickOpcode::from_byte(opcode) {
                require_words(code_units, code_cursor, quick.code_units())?;
                indices.push(read_legacy_index(bytes, &mut byte_cursor)?);
                code_cursor += quick.code_units();
                continue;
            }
            if opcode == Opcode::Nop.byte() {
                let register = read_legacy_index(bytes, &mut byte_cursor)?;
                indices.push(register);
                if register != QUICKENING_SENTINEL {
                    require_words(code_units, code_cursor, 2)?;
                    indices.push(read_legacy_index(bytes, &mut byte_cursor)?);
                    code_cursor += 2;
                    continue;
                }
            }
            if opcode == RETURN_VOID_NO_BARRIER {
                code_cursor += 1;
                continue;
            }
            let standard = Opcode::from_byte(opcode).ok_or_else(|| {
                Error::quickening(
                    code_cursor,
                    format!("unknown optimized opcode 0x{opcode:02x}"),
                )
            })?;
            let width = usize::try_from(standard.format().code_units()).map_err(|_| {
                Error::quickening(code_cursor, "instruction width does not fit platform")
            })?;
            require_words(code_units, code_cursor, width)?;
            code_cursor += width;
        }
        if byte_cursor != bytes.len() {
            return Err(Error::quickening(
                byte_cursor,
                format!(
                    "legacy method used {byte_cursor} of {} quickening bytes",
                    bytes.len()
                ),
            ));
        }
        Ok(Self { indices })
    }
}

/// Code units proven to contain only standard Dalvik instructions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CanonicalCodeUnits {
    words: Vec<u16>,
}

impl CanonicalCodeUnits {
    /// Returns canonical code units.
    #[must_use]
    pub fn words(&self) -> &[u16] {
        &self.words
    }

    /// Decodes the canonical words through the DEX frontend.
    ///
    /// # Errors
    ///
    /// Returns an error if branch or payload relationships are malformed.
    pub fn decode(&self) -> std::result::Result<Vec<Instruction>, dex::Error> {
        dex::instruction::decode(&self.words)
    }
}

/// One caller-supplied same-width restoration for optimized formats whose
/// original indices require external class-resolution metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CanonicalPatch {
    offset: u32,
    expected_opcode: u8,
    replacement: Vec<u16>,
}

impl CanonicalPatch {
    /// Creates a checked patch description.
    ///
    /// `replacement` must occupy exactly the original instruction width when
    /// applied.
    #[must_use]
    pub const fn new(offset: u32, expected_opcode: u8, replacement: Vec<u16>) -> Self {
        Self {
            offset,
            expected_opcode,
            replacement,
        }
    }

    /// Returns the native code-unit offset.
    #[must_use]
    pub const fn offset(&self) -> u32 {
        self.offset
    }
}

/// Restores Android 9 ART quick opcodes using one method's VDEX index stream.
///
/// This implements ART's quick field, virtual-call, quick check-cast, and
/// return-void-no-barrier restoration. The result is decoded by the standard
/// DEX codec before it can cross the ART boundary.
///
/// # Errors
///
/// Returns an error for truncated instructions, missing or unused indices,
/// malformed payloads, or a noncanonical result.
pub fn dequicken_code_units(
    code_units: &[u16],
    quickening: &QuickeningInfo,
) -> Result<CanonicalCodeUnits> {
    let mut words = code_units.to_vec();
    let mut cursor = 0usize;
    let mut quick_cursor = 0usize;
    while cursor < words.len() {
        if let Some(width) = payload_width(&words, cursor)? {
            cursor = cursor
                .checked_add(width)
                .ok_or_else(|| Error::quickening(cursor, "payload cursor overflowed"))?;
            continue;
        }
        let opcode = words[cursor].to_le_bytes()[0];
        if opcode == RETURN_VOID_NO_BARRIER {
            words[cursor] = (words[cursor] & 0xff00) | u16::from(Opcode::ReturnVoid.byte());
            cursor += 1;
            continue;
        }
        if let Some(quick) = QuickOpcode::from_byte(opcode) {
            let width = quick.code_units();
            require_words(&words, cursor, width)?;
            let index = next_index(quickening, &mut quick_cursor, cursor)?;
            words[cursor] = (words[cursor] & 0xff00) | u16::from(quick.canonical().byte());
            words[cursor + 1] = index;
            cursor += width;
            continue;
        }
        if opcode == Opcode::Nop.byte() && quick_cursor < quickening.indices.len() {
            let register = next_index(quickening, &mut quick_cursor, cursor)?;
            if register != QUICKENING_SENTINEL {
                require_words(&words, cursor, 2)?;
                let register = u8::try_from(register).map_err(|_| {
                    Error::quickening(cursor, "quick check-cast register exceeds eight bits")
                })?;
                let type_index = next_index(quickening, &mut quick_cursor, cursor)?;
                words[cursor] = u16::from(CHECK_CAST_OPCODE) | (u16::from(register) << u8::BITS);
                words[cursor + 1] = type_index;
                cursor += 2;
                continue;
            }
        }
        let standard = Opcode::from_byte(opcode).ok_or_else(|| {
            Error::quickening(cursor, format!("unknown optimized opcode 0x{opcode:02x}"))
        })?;
        let width = standard.format().code_units() as usize;
        require_words(&words, cursor, width)?;
        cursor += width;
    }
    if quick_cursor != quickening.indices.len() {
        return Err(Error::quickening(
            cursor,
            format!(
                "used {quick_cursor} of {} quickening indices",
                quickening.indices.len()
            ),
        ));
    }
    dex::instruction::decode(&words)?;
    Ok(CanonicalCodeUnits { words })
}

/// Applies explicit same-width patches and proves the result with the standard
/// DEX decoder.
///
/// This is the safe boundary for legacy ODEX revisions whose optimized operand
/// values require external dependency/class-resolution metadata.
///
/// # Errors
///
/// Returns an error for unsorted, overlapping, out-of-range, wrong-opcode, or
/// width-changing patches, or if optimized opcodes remain afterward.
pub fn canonicalize_with_patches(
    code_units: &[u16],
    patches: &[CanonicalPatch],
) -> Result<CanonicalCodeUnits> {
    let mut words = code_units.to_vec();
    let mut previous_end = 0usize;
    for patch in patches {
        let start = usize::try_from(patch.offset)
            .map_err(|_| Error::quickening(0, "patch offset is too large"))?;
        if start < previous_end {
            return Err(Error::quickening(
                start,
                "patches overlap or are not ordered",
            ));
        }
        if patch.replacement.is_empty() {
            return Err(Error::quickening(start, "canonical patch is empty"));
        }
        let end = start
            .checked_add(patch.replacement.len())
            .ok_or_else(|| Error::quickening(start, "patch range overflowed"))?;
        let target = words
            .get_mut(start..end)
            .ok_or_else(|| Error::quickening(start, "patch lies outside the method"))?;
        let actual = target[0].to_le_bytes()[0];
        if actual != patch.expected_opcode {
            return Err(Error::quickening(
                start,
                format!(
                    "patch expected opcode 0x{:02x}, found 0x{actual:02x}",
                    patch.expected_opcode
                ),
            ));
        }
        target.copy_from_slice(&patch.replacement);
        previous_end = end;
    }
    dex::instruction::decode(&words)?;
    Ok(CanonicalCodeUnits { words })
}

fn next_index(info: &QuickeningInfo, cursor: &mut usize, code_offset: usize) -> Result<u16> {
    let value = info.indices.get(*cursor).copied().ok_or_else(|| {
        Error::quickening(code_offset, "quickened instruction has no saved index")
    })?;
    *cursor += 1;
    Ok(value)
}

fn read_legacy_index(bytes: &[u8], cursor: &mut usize) -> Result<u16> {
    let end = cursor
        .checked_add(2)
        .ok_or_else(|| Error::quickening(*cursor, "legacy quickening cursor overflowed"))?;
    let raw: [u8; 2] = bytes
        .get(*cursor..end)
        .ok_or_else(|| Error::quickening(*cursor, "truncated legacy quickening index"))?
        .try_into()
        .map_err(|_| Error::quickening(*cursor, "truncated legacy quickening index"))?;
    *cursor = end;
    Ok(u16::from_le_bytes(raw))
}

fn payload_width(words: &[u16], cursor: usize) -> Result<Option<usize>> {
    let Some(kind) = PayloadKind::from_identifier(words[cursor]) else {
        return Ok(None);
    };
    require_words(words, cursor, 2)?;
    let count = usize::from(words[cursor + 1]);
    let width = match kind {
        PayloadKind::PackedSwitch => 4usize
            .checked_add(count.checked_mul(2).ok_or_else(|| {
                Error::quickening(cursor, "packed-switch payload size overflowed")
            })?)
            .ok_or_else(|| Error::quickening(cursor, "packed-switch range overflowed"))?,
        PayloadKind::SparseSwitch => 2usize
            .checked_add(count.checked_mul(4).ok_or_else(|| {
                Error::quickening(cursor, "sparse-switch payload size overflowed")
            })?)
            .ok_or_else(|| Error::quickening(cursor, "sparse-switch range overflowed"))?,
        PayloadKind::ArrayData => {
            require_words(words, cursor, 4)?;
            let element_width = usize::from(words[cursor + 1]);
            let element_count = u32::from(words[cursor + 2]) | (u32::from(words[cursor + 3]) << 16);
            let bytes = usize::try_from(element_count)
                .ok()
                .and_then(|count| count.checked_mul(element_width))
                .ok_or_else(|| Error::quickening(cursor, "array payload size overflowed"))?;
            4usize
                .checked_add(bytes.div_ceil(2))
                .ok_or_else(|| Error::quickening(cursor, "array payload range overflowed"))?
        }
    };
    require_words(words, cursor, width)?;
    Ok(Some(width))
}

fn require_words(words: &[u16], offset: usize, width: usize) -> Result<()> {
    if offset
        .checked_add(width)
        .is_some_and(|end| end <= words.len())
    {
        Ok(())
    } else {
        Err(Error::quickening(
            offset,
            "truncated instruction or payload",
        ))
    }
}

fn read_uleb(bytes: &[u8], cursor: &mut usize) -> Result<u32> {
    let mut value = 0u32;
    for group in 0..5u32 {
        let byte = bytes
            .get(*cursor)
            .copied()
            .ok_or_else(|| Error::quickening(*cursor, "truncated quickening ULEB128"))?;
        *cursor += 1;
        let payload = u32::from(byte & ULEB_PAYLOAD_MASK);
        if group == 4 && payload > 0x0f {
            return Err(Error::quickening(
                *cursor - 1,
                "quickening ULEB128 exceeds 32 bits",
            ));
        }
        value |= payload << (group * ELEMENTS_PER_ULEB_GROUP);
        if byte & ULEB_CONTINUATION == 0 {
            return Ok(value);
        }
    }
    Err(Error::quickening(*cursor, "quickening ULEB128 is too long"))
}

fn write_uleb(output: &mut Vec<u8>, mut value: u32) {
    loop {
        let mut byte = value.to_le_bytes()[0] & ULEB_PAYLOAD_MASK;
        value >>= ELEMENTS_PER_ULEB_GROUP;
        if value != 0 {
            byte |= ULEB_CONTINUATION;
        }
        output.push(byte);
        if value == 0 {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{QuickeningInfo, dequicken_code_units};
    use dex::instruction::{Opcode, Operands};

    #[test]
    fn restores_quick_fields_calls_casts_and_returns() {
        let words = [
            0x10e3, 99, // iget-quick v0, v1, field-offset@99
            0x10e9, 44, 0, // invoke-virtual-quick
            0x0000, 0,      // quick check-cast represented by nop + spare word
            0x0073, // return-void-no-barrier
        ];
        let info = QuickeningInfo::new(vec![7, 8, 1, 9]);
        let canonical = dequicken_code_units(&words, &info).unwrap();
        let decoded = canonical.decode().unwrap();
        assert_eq!(decoded[0].data().opcode(), Some(Opcode::Iget));
        assert_eq!(decoded[1].data().opcode(), Some(Opcode::InvokeVirtual));
        assert_eq!(decoded[2].data().opcode(), Some(Opcode::CheckCast));
        assert_eq!(decoded[3].data().opcode(), Some(Opcode::ReturnVoid));
        assert!(matches!(
            decoded[2].data().operands(),
            Some(Operands::RegisterIndex {
                register: 1,
                index: 9
            })
        ));
    }

    #[test]
    fn quickening_records_round_trip_exactly() {
        let info = QuickeningInfo::new(vec![0, u16::MAX, 42]);
        let bytes = info.to_bytes().unwrap();
        assert_eq!(QuickeningInfo::parse(&bytes).unwrap(), info);
    }
}

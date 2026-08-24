use crate::{Error, Result};

const NUL_BYTE: u8 = 0;
const NUL_CODE_UNIT: u16 = 0;
const SINGLE_BYTE_MIN: u8 = 0x01;
const SINGLE_BYTE_MAX: u8 = 0x7f;
const SINGLE_CODE_UNIT_MIN: u16 = 0x0001;
const SINGLE_CODE_UNIT_MAX: u16 = 0x007f;
const TWO_BYTE_LEAD_PREFIX: u8 = 0xc0;
const TWO_BYTE_LEAD_MAX: u8 = 0xdf;
const THREE_BYTE_LEAD_PREFIX: u8 = 0xe0;
const THREE_BYTE_LEAD_MAX: u8 = 0xef;
const CONTINUATION_PREFIX: u8 = 0x80;
const BYTE_KIND_MASK: u8 = 0xc0;
const CONTINUATION_VALUE_MASK: u8 = 0x3f;
const TWO_BYTE_LEAD_VALUE_MASK: u8 = 0x1f;
const THREE_BYTE_LEAD_VALUE_MASK: u8 = 0x0f;
const CONTINUATION_BITS: u32 = 6;
const THREE_BYTE_LEAD_SHIFT: u32 = CONTINUATION_BITS * 2;
const MIN_TWO_BYTE_CODE_UNIT: u16 = 0x80;
const MAX_TWO_BYTE_CODE_UNIT: u16 = 0x07ff;
const MIN_THREE_BYTE_CODE_UNIT: u16 = 0x0800;
const SINGLE_BYTE_SEQUENCE_LENGTH: usize = 1;
const TWO_BYTE_SEQUENCE_LENGTH: usize = 2;
const THREE_BYTE_SEQUENCE_LENGTH: usize = 3;
const MODIFIED_NUL: [u8; TWO_BYTE_SEQUENCE_LENGTH] = [TWO_BYTE_LEAD_PREFIX, CONTINUATION_PREFIX];

#[derive(Debug)]
pub(crate) struct Decoded {
    pub(crate) text: String,
    pub(crate) units: Vec<u16>,
}

pub(crate) fn decode(bytes: &[u8], source_offset: usize) -> Result<Decoded> {
    let mut units = Vec::with_capacity(bytes.len());
    let mut position = 0;

    while position < bytes.len() {
        let first = bytes[position];
        match first {
            NUL_BYTE => {
                return Err(Error::invalid_class(
                    source_offset + position,
                    "NUL must use the two-byte modified UTF-8 encoding",
                ));
            }
            SINGLE_BYTE_MIN..=SINGLE_BYTE_MAX => {
                units.push(u16::from(first));
                position += SINGLE_BYTE_SEQUENCE_LENGTH;
            }
            TWO_BYTE_LEAD_PREFIX..=TWO_BYTE_LEAD_MAX => {
                let second =
                    continuation(bytes, position + SINGLE_BYTE_SEQUENCE_LENGTH, source_offset)?;
                let value = (u16::from(first & TWO_BYTE_LEAD_VALUE_MASK) << CONTINUATION_BITS)
                    | u16::from(second & CONTINUATION_VALUE_MASK);
                if value != NUL_CODE_UNIT && value < MIN_TWO_BYTE_CODE_UNIT {
                    return Err(Error::invalid_class(
                        source_offset + position,
                        "overlong modified UTF-8 sequence",
                    ));
                }
                units.push(value);
                position += TWO_BYTE_SEQUENCE_LENGTH;
            }
            THREE_BYTE_LEAD_PREFIX..=THREE_BYTE_LEAD_MAX => {
                let second =
                    continuation(bytes, position + SINGLE_BYTE_SEQUENCE_LENGTH, source_offset)?;
                let third =
                    continuation(bytes, position + TWO_BYTE_SEQUENCE_LENGTH, source_offset)?;
                let value = (u16::from(first & THREE_BYTE_LEAD_VALUE_MASK)
                    << THREE_BYTE_LEAD_SHIFT)
                    | (u16::from(second & CONTINUATION_VALUE_MASK) << CONTINUATION_BITS)
                    | u16::from(third & CONTINUATION_VALUE_MASK);
                if value < MIN_THREE_BYTE_CODE_UNIT {
                    return Err(Error::invalid_class(
                        source_offset + position,
                        "overlong modified UTF-8 sequence",
                    ));
                }
                units.push(value);
                position += THREE_BYTE_SEQUENCE_LENGTH;
            }
            _ => {
                return Err(Error::invalid_class(
                    source_offset + position,
                    format!("invalid modified UTF-8 lead byte 0x{first:02x}"),
                ));
            }
        }
    }

    Ok(Decoded {
        text: String::from_utf16_lossy(&units),
        units,
    })
}

fn continuation(bytes: &[u8], position: usize, source_offset: usize) -> Result<u8> {
    let byte = *bytes.get(position).ok_or_else(|| {
        Error::invalid_class(
            source_offset + position,
            "truncated modified UTF-8 sequence",
        )
    })?;
    if byte & BYTE_KIND_MASK == CONTINUATION_PREFIX {
        Ok(byte)
    } else {
        Err(Error::invalid_class(
            source_offset + position,
            format!("invalid modified UTF-8 continuation byte 0x{byte:02x}"),
        ))
    }
}

pub(crate) fn encode(units: &[u16]) -> Vec<u8> {
    let mut encoded = Vec::with_capacity(units.len());
    for &unit in units {
        match unit {
            NUL_CODE_UNIT => encoded.extend_from_slice(&MODIFIED_NUL),
            SINGLE_CODE_UNIT_MIN..=SINGLE_CODE_UNIT_MAX => {
                encoded.push(u8::try_from(unit).expect("single-byte value fits"));
            }
            MIN_TWO_BYTE_CODE_UNIT..=MAX_TWO_BYTE_CODE_UNIT => {
                let lead = u8::try_from(unit >> CONTINUATION_BITS).expect("two-byte lead fits");
                let tail = u8::try_from(unit & u16::from(CONTINUATION_VALUE_MASK))
                    .expect("continuation fits");
                encoded.push(TWO_BYTE_LEAD_PREFIX | lead);
                encoded.push(CONTINUATION_PREFIX | tail);
            }
            _ => {
                let lead =
                    u8::try_from(unit >> THREE_BYTE_LEAD_SHIFT).expect("three-byte lead fits");
                let middle =
                    u8::try_from((unit >> CONTINUATION_BITS) & u16::from(CONTINUATION_VALUE_MASK))
                        .expect("continuation fits");
                let tail = u8::try_from(unit & u16::from(CONTINUATION_VALUE_MASK))
                    .expect("continuation fits");
                encoded.push(THREE_BYTE_LEAD_PREFIX | lead);
                encoded.push(CONTINUATION_PREFIX | middle);
                encoded.push(CONTINUATION_PREFIX | tail);
            }
        }
    }
    encoded
}

#[cfg(test)]
mod tests {
    use super::{decode, encode};

    #[test]
    fn decodes_nul_and_supplementary_characters() {
        let bytes = [b'a', 0xc0, 0x80, 0xed, 0xa0, 0xbd, 0xed, 0xb8, 0x80];
        let decoded = decode(&bytes, 0).unwrap();
        assert_eq!(decoded.text, "a\0😀");
        assert_eq!(decoded.units.len(), 4);
    }

    #[test]
    fn rejects_four_byte_utf8() {
        let error = decode("😀".as_bytes(), 12).unwrap_err();
        assert!(error.to_string().contains("byte 0xf0"));
    }

    #[test]
    fn preserves_unpaired_surrogates() {
        let decoded = decode(&[0xed, 0xa0, 0x80], 0).unwrap();
        assert_eq!(decoded.units, [0xd800]);
        assert_eq!(decoded.text, "�");
    }

    #[test]
    fn encodes_nul_supplementary_and_unpaired_surrogates() {
        let units = [b'a'.into(), 0, 0xd83d, 0xde00, 0xd800];
        assert_eq!(
            encode(&units),
            [
                b'a', 0xc0, 0x80, 0xed, 0xa0, 0xbd, 0xed, 0xb8, 0x80, 0xed, 0xa0, 0x80
            ]
        );
    }
}

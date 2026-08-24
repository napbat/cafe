//! DEX modified UTF-8 parsing and canonical encoding.

use crate::{Error, Result};

use super::io::Cursor;

pub(crate) const TERMINATOR: u8 = 0;
pub(crate) const ONE_BYTE_MINIMUM: u8 = 0x01;
pub(crate) const ONE_BYTE_MAXIMUM: u8 = 0x7f;
pub(crate) const TWO_BYTE_PREFIX: u8 = 0xc0;
const TWO_BYTE_LEAD_MAXIMUM: u8 = 0xdf;
pub(crate) const THREE_BYTE_PREFIX: u8 = 0xe0;
const THREE_BYTE_LEAD_MAXIMUM: u8 = 0xef;
pub(crate) const CONTINUATION_PREFIX: u8 = 0x80;
const CONTINUATION_TAG_MASK: u8 = 0xc0;
pub(crate) const TWO_BYTE_VALUE_LIMIT: u16 = 0x07ff;
const TWO_BYTE_VALUE_MINIMUM: u16 = 0x0080;
const THREE_BYTE_VALUE_MINIMUM: u16 = 0x0800;
pub(crate) const SIX_BIT_MASK: u8 = 0x3f;
pub(crate) const FIVE_BIT_MASK: u8 = 0x1f;
pub(crate) const FOUR_BIT_MASK: u8 = 0x0f;
pub(crate) const SIX_BIT_SHIFT: u32 = 6;
pub(crate) const TWELVE_BIT_SHIFT: u32 = 12;

pub(super) fn decode(cursor: &mut Cursor<'_>, expected_units: u32) -> Result<Vec<u16>> {
    let start = cursor.position();
    let expected = usize::try_from(expected_units)
        .map_err(|_| Error::invalid_dex(start, "string length does not fit this platform"))?;
    let mut units = Vec::with_capacity(expected);
    loop {
        let byte = cursor.u8()?;
        match byte {
            TERMINATOR => break,
            ONE_BYTE_MINIMUM..=ONE_BYTE_MAXIMUM => units.push(u16::from(byte)),
            TWO_BYTE_PREFIX..=TWO_BYTE_LEAD_MAXIMUM => {
                let second = continuation(cursor, start)?;
                let value = (u16::from(byte & FIVE_BIT_MASK) << SIX_BIT_SHIFT) | u16::from(second);
                if value != u16::from(TERMINATOR) && value < TWO_BYTE_VALUE_MINIMUM {
                    return Err(Error::invalid_dex(start, "overlong two-byte MUTF-8 unit"));
                }
                units.push(value);
            }
            THREE_BYTE_PREFIX..=THREE_BYTE_LEAD_MAXIMUM => {
                let second = continuation(cursor, start)?;
                let third = continuation(cursor, start)?;
                let value = (u16::from(byte & FOUR_BIT_MASK) << TWELVE_BIT_SHIFT)
                    | (u16::from(second) << SIX_BIT_SHIFT)
                    | u16::from(third);
                if value < THREE_BYTE_VALUE_MINIMUM {
                    return Err(Error::invalid_dex(start, "overlong three-byte MUTF-8 unit"));
                }
                units.push(value);
            }
            _ => return Err(Error::invalid_dex(start, "invalid MUTF-8 leading byte")),
        }
        if units.len() > expected {
            return Err(Error::invalid_dex(
                start,
                "MUTF-8 data exceeds its declared UTF-16 length",
            ));
        }
    }
    if units.len() != expected {
        return Err(Error::invalid_dex(
            start,
            format!(
                "MUTF-8 data has {} UTF-16 units but declares {expected}",
                units.len()
            ),
        ));
    }
    Ok(units)
}

fn continuation(cursor: &mut Cursor<'_>, start: usize) -> Result<u8> {
    let byte = cursor.u8()?;
    if byte & CONTINUATION_TAG_MASK == CONTINUATION_PREFIX {
        Ok(byte & SIX_BIT_MASK)
    } else {
        Err(Error::invalid_dex(
            start,
            "invalid MUTF-8 continuation byte",
        ))
    }
}

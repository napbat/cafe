//! Canonical 32-bit LEB128 decoding.

use crate::{Error, Result};

use super::Cursor;

pub(super) const PAYLOAD_MASK: u8 = 0x7f;
pub(super) const CONTINUATION_BIT: u8 = 0x80;
pub(super) const SIGN_BIT: u8 = 0x40;
pub(super) const GROUP_BITS: usize = 7;
pub(super) const P1_BIAS: u32 = 1;
pub(super) const ULEB128P1_NONE: u32 = 0;
pub(super) const GROUP_COUNT_BIAS: usize = 1;
pub(super) const UNSIGNED_TERMINATOR: u32 = 0;
pub(super) const POSITIVE_SIGNED_TERMINATOR: i32 = 0;
pub(super) const NEGATIVE_SIGNED_TERMINATOR: i32 = -1;
const MAX_ENCODED_BYTES: usize = 5;
const FINAL_BYTE_INDEX: usize = MAX_ENCODED_BYTES - 1;
const MAX_UNSIGNED_FINAL_PAYLOAD: u32 = 0x0f;
const MAX_POSITIVE_SIGNED_FINAL_PAYLOAD: u32 = 0x07;
const MIN_NEGATIVE_SIGNED_FINAL_PAYLOAD: u32 = 0x78;
const MAX_PAYLOAD: u32 = 0x7f;
const TARGET_WIDTH_BITS: usize = 32;

pub(super) fn read_unsigned(cursor: &mut Cursor<'_>) -> Result<u32> {
    let start = cursor.position();
    let mut result = 0u32;
    for index in 0..MAX_ENCODED_BYTES {
        let byte = cursor.u8()?;
        let payload = u32::from(byte & PAYLOAD_MASK);
        if index == FINAL_BYTE_INDEX && payload > MAX_UNSIGNED_FINAL_PAYLOAD {
            return Err(Error::invalid_dex(start, "uleb128 exceeds 32 bits"));
        }
        result |= payload << (index * GROUP_BITS);
        if byte & CONTINUATION_BIT == 0 {
            return Ok(result);
        }
    }
    Err(Error::invalid_dex(start, "uleb128 exceeds five bytes"))
}

pub(super) fn read_signed(cursor: &mut Cursor<'_>) -> Result<i32> {
    let start = cursor.position();
    let mut result = 0u32;
    for index in 0..MAX_ENCODED_BYTES {
        let byte = cursor.u8()?;
        let payload = u32::from(byte & PAYLOAD_MASK);
        if index == FINAL_BYTE_INDEX
            && !matches!(
                payload,
                0..=MAX_POSITIVE_SIGNED_FINAL_PAYLOAD | MIN_NEGATIVE_SIGNED_FINAL_PAYLOAD..=MAX_PAYLOAD
            )
        {
            return Err(Error::invalid_dex(start, "sleb128 exceeds 32 bits"));
        }
        result |= payload << (index * GROUP_BITS);
        if byte & CONTINUATION_BIT == 0 {
            let used_bits = (index + GROUP_COUNT_BIAS) * GROUP_BITS;
            if used_bits < TARGET_WIDTH_BITS && byte & SIGN_BIT != 0 {
                result |= u32::MAX << used_bits;
            }
            return Ok(i32::from_ne_bytes(result.to_ne_bytes()));
        }
    }
    Err(Error::invalid_dex(start, "sleb128 exceeds five bytes"))
}

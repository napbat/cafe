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
const INITIAL_VALUE_BITS: u32 = 0;
const CLEARED_CONTROL_BITS: u8 = 0;
const MAX_ENCODED_BYTES: usize = 5;
const FINAL_BYTE_INDEX: usize = MAX_ENCODED_BYTES - 1;
const MAX_UNSIGNED_FINAL_PAYLOAD: u32 = 0x0f;
const MIN_POSITIVE_SIGNED_FINAL_PAYLOAD: u32 = 0;
const MAX_POSITIVE_SIGNED_FINAL_PAYLOAD: u32 = 0x07;
const MIN_NEGATIVE_SIGNED_FINAL_PAYLOAD: u32 = 0x78;
const MAX_PAYLOAD: u32 = 0x7f;
const TARGET_WIDTH_BITS: usize = 32;

pub(super) fn read_unsigned(cursor: &mut Cursor<'_>) -> Result<u32> {
    let start = cursor.position();
    let mut result = INITIAL_VALUE_BITS;
    for index in 0..MAX_ENCODED_BYTES {
        let byte = cursor.u8()?;
        let payload = u32::from(byte & PAYLOAD_MASK);
        if index == FINAL_BYTE_INDEX && payload > MAX_UNSIGNED_FINAL_PAYLOAD {
            return Err(Error::invalid_dex(
                start,
                format!("uleb128 exceeds {TARGET_WIDTH_BITS} bits"),
            ));
        }
        result |= payload << (index * GROUP_BITS);
        if byte & CONTINUATION_BIT == CLEARED_CONTROL_BITS {
            return Ok(result);
        }
    }
    Err(Error::invalid_dex(
        start,
        format!("uleb128 exceeds {MAX_ENCODED_BYTES} bytes"),
    ))
}

pub(super) fn read_signed(cursor: &mut Cursor<'_>) -> Result<i32> {
    let start = cursor.position();
    let mut result = INITIAL_VALUE_BITS;
    for index in 0..MAX_ENCODED_BYTES {
        let byte = cursor.u8()?;
        let payload = u32::from(byte & PAYLOAD_MASK);
        if index == FINAL_BYTE_INDEX
            && !matches!(
                payload,
                MIN_POSITIVE_SIGNED_FINAL_PAYLOAD..=MAX_POSITIVE_SIGNED_FINAL_PAYLOAD
                    | MIN_NEGATIVE_SIGNED_FINAL_PAYLOAD..=MAX_PAYLOAD
            )
        {
            return Err(Error::invalid_dex(
                start,
                format!("sleb128 exceeds {TARGET_WIDTH_BITS} bits"),
            ));
        }
        result |= payload << (index * GROUP_BITS);
        if byte & CONTINUATION_BIT == CLEARED_CONTROL_BITS {
            let used_bits = (index + GROUP_COUNT_BIAS) * GROUP_BITS;
            if used_bits < TARGET_WIDTH_BITS && byte & SIGN_BIT != CLEARED_CONTROL_BITS {
                result |= u32::MAX << used_bits;
            }
            return Ok(i32::from_ne_bytes(result.to_ne_bytes()));
        }
    }
    Err(Error::invalid_dex(
        start,
        format!("sleb128 exceeds {MAX_ENCODED_BYTES} bytes"),
    ))
}

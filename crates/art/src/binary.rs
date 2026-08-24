//! Shared bounds-checked binary helpers for ART containers.

use sha1::{Digest, Sha1};

use crate::{Error, Result};

const ADLER_MODULUS: u32 = 65_521;
const ADLER_CHUNK_SIZE: usize = 5_552;

pub(crate) fn u32_at(bytes: &[u8], offset: usize, format: &'static str) -> Result<u32> {
    let raw: [u8; 4] = range(bytes, offset, 4, format, "32-bit value")?
        .try_into()
        .map_err(|_| Error::invalid(format, offset, "truncated 32-bit value"))?;
    Ok(u32::from_le_bytes(raw))
}

pub(crate) fn put_u32(
    bytes: &mut [u8],
    offset: usize,
    value: u32,
    format: &'static str,
) -> Result<()> {
    bytes
        .get_mut(offset..offset + 4)
        .ok_or_else(|| Error::invalid(format, offset, "32-bit output field is out of bounds"))?
        .copy_from_slice(&value.to_le_bytes());
    Ok(())
}

pub(crate) fn range<'a>(
    bytes: &'a [u8],
    offset: usize,
    length: usize,
    format: &'static str,
    what: &str,
) -> Result<&'a [u8]> {
    let end = offset
        .checked_add(length)
        .ok_or_else(|| Error::invalid(format, offset, format!("{what} range overflowed")))?;
    bytes
        .get(offset..end)
        .ok_or_else(|| Error::invalid(format, offset, format!("truncated {what}")))
}

pub(crate) fn checked_range(
    offset: u32,
    length: u32,
    total: usize,
    format: &'static str,
    what: &str,
) -> Result<std::ops::Range<usize>> {
    let start = usize::try_from(offset)
        .map_err(|_| Error::invalid(format, 0, format!("{what} offset is too large")))?;
    let length = usize::try_from(length)
        .map_err(|_| Error::invalid(format, start, format!("{what} length is too large")))?;
    let end = start
        .checked_add(length)
        .ok_or_else(|| Error::invalid(format, start, format!("{what} range overflowed")))?;
    if end <= total {
        Ok(start..end)
    } else {
        Err(Error::invalid(format, start, format!("truncated {what}")))
    }
}

pub(crate) fn align_up(value: usize, alignment: usize, format: &'static str) -> Result<usize> {
    let remainder = value % alignment;
    if remainder == 0 {
        Ok(value)
    } else {
        value
            .checked_add(alignment - remainder)
            .ok_or_else(|| Error::invalid(format, value, "alignment overflowed"))
    }
}

pub(crate) fn adler32(bytes: &[u8]) -> u32 {
    let mut first = 1u32;
    let mut second = 0u32;
    for chunk in bytes.chunks(ADLER_CHUNK_SIZE) {
        for byte in chunk {
            first += u32::from(*byte);
            second += first;
        }
        first %= ADLER_MODULUS;
        second %= ADLER_MODULUS;
    }
    (second << u16::BITS) | first
}

pub(crate) fn update_standard_dex_integrity(bytes: &mut [u8]) -> Result<()> {
    const SIGNATURE_OFFSET: usize = 12;
    const FILE_SIZE_OFFSET: usize = 32;
    const CHECKSUM_OFFSET: usize = 8;
    const SIGNATURE_SIZE: usize = 20;
    if bytes.len() < 0x70 {
        return Err(Error::invalid("DEX", 0, "embedded DEX header is truncated"));
    }
    let signature: [u8; SIGNATURE_SIZE] = Sha1::digest(&bytes[FILE_SIZE_OFFSET..]).into();
    bytes[SIGNATURE_OFFSET..SIGNATURE_OFFSET + SIGNATURE_SIZE].copy_from_slice(&signature);
    let checksum = adler32(&bytes[SIGNATURE_OFFSET..]);
    bytes[CHECKSUM_OFFSET..SIGNATURE_OFFSET].copy_from_slice(&checksum.to_le_bytes());
    Ok(())
}

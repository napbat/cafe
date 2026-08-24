//! Typed ZIP and APK-signing-block layout vocabulary.

use crate::{Error, Result};

pub(super) const ZIP_END_OF_CENTRAL_DIRECTORY_SIGNATURE: [u8; 4] = [0x50, 0x4b, 0x05, 0x06];
pub(super) const ZIP_END_OF_CENTRAL_DIRECTORY_SIZE: usize = 22;
pub(super) const ZIP_U16_MAXIMUM: usize = 65_535;
pub(super) const ZIP_MAXIMUM_COMMENT_SIZE: usize = ZIP_U16_MAXIMUM;
pub(super) const ZIP_CENTRAL_DIRECTORY_OFFSET_FIELD: usize = 16;
pub(super) const ZIP_COMMENT_LENGTH_FIELD: usize = 20;
pub(super) const ZIP_U16_FIELD_WIDTH: usize = size_of::<u16>();
pub(super) const ZIP_U32_FIELD_WIDTH: usize = size_of::<u32>();
pub(super) const ZIP64_U32_SENTINEL: u32 = u32::MAX;

pub(super) const SIGNING_BLOCK_MAGIC: &[u8; 16] = b"APK Sig Block 42";
pub(super) const SIGNING_BLOCK_SIZE_FIELD_WIDTH: usize = size_of::<u64>();
pub(super) const SIGNING_BLOCK_ID_FIELD_WIDTH: usize = size_of::<u32>();
pub(super) const SIGNING_BLOCK_TRAILER_SIZE: usize =
    SIGNING_BLOCK_SIZE_FIELD_WIDTH + SIGNING_BLOCK_MAGIC.len();
pub(super) const SIGNING_BLOCK_MINIMUM_REPORTED_SIZE: u64 = SIGNING_BLOCK_TRAILER_SIZE as u64;

pub(super) const ZIP_EXTRA_FIELD_ID_OFFSET: usize = 0;
pub(super) const ZIP_EXTRA_FIELD_LENGTH_OFFSET: usize = size_of::<u16>();
pub(super) const ZIP_EXTRA_FIELD_HEADER_SIZE: usize = size_of::<u16>() * 2;

pub(super) const PORTABLE_FILE_MODE: u32 = 0o644;
pub(super) const PORTABLE_DIRECTORY_MODE: u32 = 0o755;
pub(super) const PORTABLE_SYMLINK_MODE: u32 = 0o777;

pub(super) const INITIAL_ENTRY_ID: u64 = 0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct ZipSections {
    pub(super) end_of_central_directory: usize,
    pub(super) central_directory: usize,
}

pub(super) fn zip_sections(bytes: &[u8]) -> Result<ZipSections> {
    if bytes.len() < ZIP_END_OF_CENTRAL_DIRECTORY_SIZE {
        return Err(Error::invalid_apk(
            "ZIP end-of-central-directory record is missing",
        ));
    }
    let search_start = bytes.len().saturating_sub(
        ZIP_END_OF_CENTRAL_DIRECTORY_SIZE
            .checked_add(ZIP_MAXIMUM_COMMENT_SIZE)
            .expect("ZIP EOCD search bound is representable"),
    );
    let latest_start = bytes.len() - ZIP_END_OF_CENTRAL_DIRECTORY_SIZE;
    let end_of_central_directory = (search_start..=latest_start)
        .rev()
        .find(|&offset| {
            bytes.get(offset..offset + ZIP_END_OF_CENTRAL_DIRECTORY_SIGNATURE.len())
                == Some(ZIP_END_OF_CENTRAL_DIRECTORY_SIGNATURE.as_slice())
                && record_ends_at_input_end(bytes, offset)
        })
        .ok_or_else(|| Error::invalid_apk("ZIP end-of-central-directory record is missing"))?;
    let central_directory = read_u32(
        bytes,
        end_of_central_directory + ZIP_CENTRAL_DIRECTORY_OFFSET_FIELD,
    )?;
    if central_directory == ZIP64_U32_SENTINEL {
        return Err(Error::invalid_apk(
            "ZIP64 APK archives are not supported for signing-block preservation",
        ));
    }
    let central_directory = usize::try_from(central_directory)
        .map_err(|_| Error::invalid_apk("central-directory offset does not fit this platform"))?;
    if central_directory > end_of_central_directory {
        return Err(Error::invalid_apk(
            "central-directory offset follows the end record",
        ));
    }
    Ok(ZipSections {
        end_of_central_directory,
        central_directory,
    })
}

pub(super) fn write_central_directory_offset(
    bytes: &mut [u8],
    end_of_central_directory: usize,
    value: u32,
) -> Result<()> {
    let start = end_of_central_directory
        .checked_add(ZIP_CENTRAL_DIRECTORY_OFFSET_FIELD)
        .ok_or_else(|| Error::invalid_apk("central-directory field offset overflowed"))?;
    let end = start
        .checked_add(ZIP_U32_FIELD_WIDTH)
        .ok_or_else(|| Error::invalid_apk("central-directory field end overflowed"))?;
    bytes
        .get_mut(start..end)
        .ok_or_else(|| Error::invalid_apk("central-directory field is truncated"))?
        .copy_from_slice(&value.to_le_bytes());
    Ok(())
}

pub(super) fn read_u32(bytes: &[u8], offset: usize) -> Result<u32> {
    let end = offset
        .checked_add(ZIP_U32_FIELD_WIDTH)
        .ok_or_else(|| Error::invalid_apk("32-bit field offset overflowed"))?;
    let field: [u8; ZIP_U32_FIELD_WIDTH] = bytes
        .get(offset..end)
        .ok_or_else(|| Error::invalid_apk("32-bit field is truncated"))?
        .try_into()
        .expect("slice length was checked");
    Ok(u32::from_le_bytes(field))
}

pub(super) fn read_u64(bytes: &[u8], offset: usize) -> Result<u64> {
    let end = offset
        .checked_add(SIGNING_BLOCK_SIZE_FIELD_WIDTH)
        .ok_or_else(|| Error::invalid_apk("64-bit field offset overflowed"))?;
    let field: [u8; SIGNING_BLOCK_SIZE_FIELD_WIDTH] = bytes
        .get(offset..end)
        .ok_or_else(|| Error::invalid_apk("64-bit field is truncated"))?
        .try_into()
        .expect("slice length was checked");
    Ok(u64::from_le_bytes(field))
}

fn record_ends_at_input_end(bytes: &[u8], offset: usize) -> bool {
    let Some(comment_offset) = offset.checked_add(ZIP_COMMENT_LENGTH_FIELD) else {
        return false;
    };
    let Some(comment_end) = comment_offset.checked_add(ZIP_U16_FIELD_WIDTH) else {
        return false;
    };
    let Some(field) = bytes.get(comment_offset..comment_end) else {
        return false;
    };
    let comment_length = usize::from(u16::from_le_bytes(
        field.try_into().expect("slice length was checked"),
    ));
    offset
        .checked_add(ZIP_END_OF_CENTRAL_DIRECTORY_SIZE)
        .and_then(|end| end.checked_add(comment_length))
        == Some(bytes.len())
}

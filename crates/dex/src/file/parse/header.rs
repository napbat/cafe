//! DEX header and integrity parsing.

use crate::{Error, Result};

use crate::file::header::{
    CONTAINER_HEADER_SIZE, DexHeader, DexVersion, ENDIAN_CONSTANT, Endian, HeaderField,
    LEGACY_HEADER_OFFSET, LEGACY_HEADER_SIZE, MAGIC_PREFIX, MAGIC_SIZE, MAGIC_TERMINATOR,
    MAGIC_TERMINATOR_INDEX, MAGIC_VERSION_SIZE, REVERSE_ENDIAN_CONSTANT, SECTION_OFFSET_DELTA,
    SIGNATURE_SIZE, Section,
};
use crate::file::integrity;
use crate::file::io::Reader;

pub(super) fn parse(bytes: &[u8], header_offset: usize) -> Result<DexHeader> {
    let little = Reader::new(bytes, Endian::Little);
    let version = parse_version(little, header_offset)?;
    let endian = parse_endian(little, header_offset)?;
    let reader = Reader::new(bytes, endian);
    let header_size = parse_header_size(reader, header_offset, version)?;
    reader.bytes(
        header_offset,
        usize::try_from(header_size)
            .map_err(|_| Error::invalid_dex(header_offset, "header size does not fit platform"))?,
    )?;

    let file_size_offset = field_offset(header_offset, HeaderField::FileSize)?;
    let file_size = reader.u32(file_size_offset)?;
    let (container_size, declared_header_offset) = if version == DexVersion::V041 {
        (
            reader.u32(field_offset(header_offset, HeaderField::ContainerSize)?)?,
            reader.u32(field_offset(header_offset, HeaderField::HeaderOffset)?)?,
        )
    } else {
        (file_size, crate::file::header::ABSENT_OFFSET)
    };
    validate_bounds(
        bytes,
        version,
        header_offset,
        file_size,
        container_size,
        declared_header_offset,
    )?;

    let signature_offset = field_offset(header_offset, HeaderField::Signature)?;
    let signature: [u8; SIGNATURE_SIZE] = reader
        .bytes(signature_offset, SIGNATURE_SIZE)?
        .try_into()
        .map_err(|_| Error::invalid_dex(signature_offset, "truncated SHA-1 signature"))?;
    let checksum_offset = field_offset(header_offset, HeaderField::Checksum)?;
    let checksum = reader.u32(checksum_offset)?;
    validate_integrity(
        bytes,
        version,
        header_offset,
        file_size,
        checksum,
        signature,
    )?;

    Ok(DexHeader {
        version,
        checksum,
        signature,
        file_size,
        header_size,
        endian,
        link_size: reader.u32(field_offset(header_offset, HeaderField::LinkSize)?)?,
        link_off: reader.u32(field_offset(header_offset, HeaderField::LinkOffset)?)?,
        map_off: reader.u32(field_offset(header_offset, HeaderField::MapOffset)?)?,
        string_ids: section(reader, header_offset, HeaderField::StringIds)?,
        type_ids: section(reader, header_offset, HeaderField::TypeIds)?,
        proto_ids: section(reader, header_offset, HeaderField::PrototypeIds)?,
        field_ids: section(reader, header_offset, HeaderField::FieldIds)?,
        method_ids: section(reader, header_offset, HeaderField::MethodIds)?,
        class_defs: section(reader, header_offset, HeaderField::ClassDefinitions)?,
        data: section(reader, header_offset, HeaderField::Data)?,
        container_size,
        header_offset: declared_header_offset,
    })
}

fn parse_version(reader: Reader<'_>, header_offset: usize) -> Result<DexVersion> {
    let magic_offset = field_offset(header_offset, HeaderField::Magic)?;
    let magic = reader.bytes(magic_offset, MAGIC_SIZE)?;
    if magic.get(..MAGIC_PREFIX.len()) != Some(MAGIC_PREFIX)
        || magic.get(MAGIC_TERMINATOR_INDEX) != Some(&MAGIC_TERMINATOR)
    {
        return Err(Error::invalid_dex(header_offset, "invalid DEX magic"));
    }
    let version_start = MAGIC_PREFIX.len();
    let version_end = version_start + MAGIC_VERSION_SIZE;
    let digits: [u8; MAGIC_VERSION_SIZE] = magic[version_start..version_end]
        .try_into()
        .map_err(|_| Error::invalid_dex(header_offset, "truncated DEX version"))?;
    DexVersion::from_digits(digits).ok_or_else(|| {
        Error::invalid_dex(
            header_offset + version_start,
            format!(
                "unsupported DEX version {}",
                String::from_utf8_lossy(&digits)
            ),
        )
    })
}

fn parse_endian(reader: Reader<'_>, header_offset: usize) -> Result<Endian> {
    let endian_offset = field_offset(header_offset, HeaderField::EndianTag)?;
    let raw_endian = reader.u32(endian_offset)?;
    match raw_endian {
        ENDIAN_CONSTANT => Ok(Endian::Little),
        REVERSE_ENDIAN_CONSTANT => Ok(Endian::Reverse),
        _ => Err(Error::invalid_dex(
            endian_offset,
            format!("invalid endian tag 0x{raw_endian:08x}"),
        )),
    }
}

fn parse_header_size(reader: Reader<'_>, header_offset: usize, version: DexVersion) -> Result<u32> {
    let expected = version.header_size();
    let offset = field_offset(header_offset, HeaderField::HeaderSize)?;
    let actual = reader.u32(offset)?;
    if actual == expected {
        Ok(actual)
    } else {
        Err(Error::invalid_dex(
            offset,
            format!(
                "version {} requires header size 0x{expected:x}, found 0x{actual:x}",
                String::from_utf8_lossy(&version.digits())
            ),
        ))
    }
}

fn validate_bounds(
    bytes: &[u8],
    version: DexVersion,
    actual_header_offset: usize,
    file_size: u32,
    container_size: u32,
    declared_header_offset: u32,
) -> Result<()> {
    let file_size_offset = field_offset(actual_header_offset, HeaderField::FileSize)?;
    let actual_header_offset_u32 = u32::try_from(actual_header_offset)
        .map_err(|_| Error::invalid_dex(actual_header_offset, "header offset exceeds 32 bits"))?;
    let physical_size = u32::try_from(bytes.len()).map_err(|_| {
        Error::invalid_dex(
            LEGACY_HEADER_OFFSET,
            "DEX container exceeds 32-bit address space",
        )
    })?;
    if version == DexVersion::V041 {
        if declared_header_offset != actual_header_offset_u32 {
            return Err(Error::invalid_dex(
                field_offset(actual_header_offset, HeaderField::HeaderOffset)?,
                format!(
                    "declared header offset {declared_header_offset} does not match {actual_header_offset_u32}"
                ),
            ));
        }
        if container_size != physical_size {
            return Err(Error::invalid_dex(
                field_offset(actual_header_offset, HeaderField::ContainerSize)?,
                format!("container size {container_size} does not match {physical_size}"),
            ));
        }
        let end = declared_header_offset
            .checked_add(file_size)
            .ok_or_else(|| Error::invalid_dex(file_size_offset, "logical file size overflowed"))?;
        if file_size < CONTAINER_HEADER_SIZE || end > container_size {
            return Err(Error::invalid_dex(
                file_size_offset,
                "logical file size falls outside the container",
            ));
        }
    } else {
        if actual_header_offset != LEGACY_HEADER_OFFSET {
            return Err(Error::invalid_dex(
                actual_header_offset,
                "legacy DEX header must begin at byte zero",
            ));
        }
        if file_size != physical_size || file_size < LEGACY_HEADER_SIZE {
            return Err(Error::invalid_dex(
                file_size_offset,
                format!("file size {file_size} does not match {physical_size}"),
            ));
        }
    }
    Ok(())
}

fn validate_integrity(
    bytes: &[u8],
    version: DexVersion,
    header_offset: usize,
    file_size: u32,
    checksum: u32,
    signature: [u8; SIGNATURE_SIZE],
) -> Result<()> {
    let file_size_offset = field_offset(header_offset, HeaderField::FileSize)?;
    let logical_end = header_offset
        .checked_add(usize::try_from(file_size).map_err(|_| {
            Error::invalid_dex(file_size_offset, "file size does not fit this platform")
        })?)
        .ok_or_else(|| Error::invalid_dex(file_size_offset, "logical file end overflowed"))?;
    let logical = bytes
        .get(header_offset..logical_end)
        .ok_or_else(|| Error::invalid_dex(header_offset, "logical DEX file is truncated"))?;
    if integrity::signature(&logical[HeaderField::FileSize.offset()..]) != signature {
        return Err(Error::invalid_dex(
            field_offset(header_offset, HeaderField::Signature)?,
            "SHA-1 signature does not match the logical file",
        ));
    }
    if integrity::adler32(&logical[HeaderField::Signature.offset()..]) != checksum {
        return Err(Error::invalid_dex(
            field_offset(header_offset, HeaderField::Checksum)?,
            "Adler-32 checksum does not match the logical file",
        ));
    }
    if version == DexVersion::V041 {
        // The experimental container currently retains per-header logical
        // integrity fields. Cross-header bounds are validated by DexContainer.
    }
    Ok(())
}

fn field_offset(header_offset: usize, field: HeaderField) -> Result<usize> {
    header_offset
        .checked_add(field.offset())
        .ok_or_else(|| Error::invalid_dex(header_offset, "DEX header field offset overflowed"))
}

fn section(reader: Reader<'_>, header_offset: usize, field: HeaderField) -> Result<Section> {
    let offset = field_offset(header_offset, field)?;
    Ok(Section {
        size: reader.u32(offset)?,
        offset: reader.u32(offset + SECTION_OFFSET_DELTA)?,
    })
}

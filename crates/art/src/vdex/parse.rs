//! Version-specific VDEX section and member parsing.

use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;

use dex::file::{CompactDexVersion, DexSourceFormat, DexVersion};

use super::{VDEX_MAGIC, VdexDexMember, VdexFile, VdexLayout, VdexSection, VdexVersion};
use crate::binary::{align_up, checked_range, range, u32_at};
use crate::{Error, Result};

const LEGACY_HEADER_SIZE: usize = 24;
const SPLIT_020_HEADER_SIZE: usize = 20;
const SPLIT_021_HEADER_SIZE: usize = 28;
const SECTIONED_HEADER_SIZE: usize = 12;
const SECTION_HEADER_SIZE: usize = 12;
const DEX_SECTION_HEADER_SIZE: usize = 12;
const CHECKSUM_SIZE: usize = 4;
const DEX_FILE_SIZE_OFFSET: usize = 32;
const DEX_ALIGNMENT: usize = 4;
const SPLIT_DEX_SECTION_VERSION: &[u8; 4] = b"002\0";
const EMPTY_DEX_SECTION_VERSION: &[u8; 4] = b"000\0";

pub(super) fn parse(bytes: &[u8]) -> Result<VdexFile> {
    range(bytes, 0, 8, "VDEX", "magic and version")?;
    if bytes.get(..VDEX_MAGIC.len()) != Some(VDEX_MAGIC) {
        return Err(Error::invalid("VDEX", 0, "invalid VDEX magic"));
    }
    let version = parse_version(bytes)?;
    match version {
        VdexVersion::V009 | VdexVersion::V012 => parse_legacy(bytes, version),
        VdexVersion::V020 | VdexVersion::V021 => parse_split(bytes, version),
        VdexVersion::V027 => parse_sectioned(bytes),
    }
}

fn parse_version(bytes: &[u8]) -> Result<VdexVersion> {
    if bytes.get(7) != Some(&0) {
        return Err(Error::invalid("VDEX", 7, "version is not zero terminated"));
    }
    let digits: [u8; 3] = bytes[4..7]
        .try_into()
        .map_err(|_| Error::invalid("VDEX", 4, "truncated version"))?;
    match &digits {
        b"009" => Ok(VdexVersion::V009),
        b"012" => Ok(VdexVersion::V012),
        b"020" => Ok(VdexVersion::V020),
        b"021" => Ok(VdexVersion::V021),
        b"027" => Ok(VdexVersion::V027),
        _ => Err(Error::UnsupportedVersion {
            format: "VDEX",
            version: String::from_utf8_lossy(&digits).into_owned(),
        }),
    }
}

fn parse_legacy(bytes: &[u8], version: VdexVersion) -> Result<VdexFile> {
    range(bytes, 0, LEGACY_HEADER_SIZE, "VDEX", "legacy header")?;
    let dex_count = u32_at(bytes, 8, "VDEX")?;
    let dex_size = u32_at(bytes, 12, "VDEX")?;
    let verifier_size = u32_at(bytes, 16, "VDEX")?;
    let quickening_size = u32_at(bytes, 20, "VDEX")?;
    let checksums = checksum_range(LEGACY_HEADER_SIZE, dex_count, bytes.len())?;
    let dex_files = following_range(checksums.end, dex_size, bytes.len(), "DEX section")?;
    let verifier = following_range(
        dex_files.end,
        verifier_size,
        bytes.len(),
        "verifier dependencies",
    )?;
    let quickening = following_range(
        verifier.end,
        quickening_size,
        bytes.len(),
        "quickening data",
    )?;
    require_end(bytes, quickening.end)?;
    let mut sections = BTreeMap::new();
    sections.insert(VdexSection::Checksums, checksums.clone());
    sections.insert(VdexSection::DexFiles, dex_files.clone());
    sections.insert(VdexSection::VerifierDependencies, verifier);
    sections.insert(VdexSection::Quickening, quickening);
    let has_prefix = version == VdexVersion::V012;
    let members = inventory_members(bytes, dex_count, &checksums, &dex_files, has_prefix)?;
    Ok(VdexFile {
        version,
        layout: VdexLayout::Legacy,
        bytes: bytes.to_vec(),
        sections,
        members,
    })
}

fn parse_split(bytes: &[u8], version: VdexVersion) -> Result<VdexFile> {
    let header_size = match version {
        VdexVersion::V020 => SPLIT_020_HEADER_SIZE,
        VdexVersion::V021 => SPLIT_021_HEADER_SIZE,
        _ => {
            return Err(Error::invalid(
                "VDEX",
                4,
                "version does not use a split header",
            ));
        }
    };
    range(bytes, 0, header_size, "VDEX", "split header")?;
    let dex_section_version = range(bytes, 8, 4, "VDEX", "DEX section version")?;
    if dex_section_version != SPLIT_DEX_SECTION_VERSION
        && dex_section_version != EMPTY_DEX_SECTION_VERSION
    {
        return Err(Error::UnsupportedVersion {
            format: "VDEX DEX section",
            version: String::from_utf8_lossy(dex_section_version)
                .trim_end_matches('\0')
                .to_owned(),
        });
    }
    let dex_count = u32_at(bytes, 12, "VDEX")?;
    let verifier_size = u32_at(bytes, 16, "VDEX")?;
    let (boot_size, context_size) = if version == VdexVersion::V021 {
        (u32_at(bytes, 20, "VDEX")?, u32_at(bytes, 24, "VDEX")?)
    } else {
        (0, 0)
    };
    let checksums = checksum_range(header_size, dex_count, bytes.len())?;
    let mut cursor = checksums.end;
    let mut sections = BTreeMap::new();
    sections.insert(VdexSection::Checksums, checksums.clone());
    let (dex_files, shared_data, quickening_size) = if dex_section_version
        == SPLIT_DEX_SECTION_VERSION
    {
        range(
            bytes,
            cursor,
            DEX_SECTION_HEADER_SIZE,
            "VDEX",
            "DEX section header",
        )?;
        let dex_size = u32_at(bytes, cursor, "VDEX")?;
        let shared_size = u32_at(bytes, cursor + 4, "VDEX")?;
        let quickening_size = u32_at(bytes, cursor + 8, "VDEX")?;
        cursor += DEX_SECTION_HEADER_SIZE;
        let dex_files = following_range(cursor, dex_size, bytes.len(), "DEX main section")?;
        cursor = dex_files.end;
        let shared = following_range(cursor, shared_size, bytes.len(), "CompactDex shared data")?;
        cursor = shared.end;
        (dex_files, shared, quickening_size)
    } else {
        (cursor..cursor, cursor..cursor, 0)
    };
    sections.insert(VdexSection::DexFiles, dex_files.clone());
    sections.insert(VdexSection::SharedData, shared_data.clone());
    let verifier = following_range(cursor, verifier_size, bytes.len(), "verifier dependencies")?;
    cursor = verifier.end;
    let quickening = following_range(cursor, quickening_size, bytes.len(), "quickening data")?;
    cursor = quickening.end;
    let boot = following_range(cursor, boot_size, bytes.len(), "boot-class-path checksums")?;
    cursor = boot.end;
    let context = following_range(cursor, context_size, bytes.len(), "class-loader context")?;
    require_end(bytes, context.end)?;
    sections.insert(VdexSection::VerifierDependencies, verifier);
    sections.insert(VdexSection::Quickening, quickening);
    sections.insert(VdexSection::BootClasspathChecksums, boot);
    sections.insert(VdexSection::ClassLoaderContext, context);
    let members = if dex_section_version == EMPTY_DEX_SECTION_VERSION {
        Vec::new()
    } else {
        inventory_members(bytes, dex_count, &checksums, &dex_files, true)?
    };
    Ok(VdexFile {
        version,
        layout: VdexLayout::Split,
        bytes: bytes.to_vec(),
        sections,
        members,
    })
}

fn parse_sectioned(bytes: &[u8]) -> Result<VdexFile> {
    range(bytes, 0, SECTIONED_HEADER_SIZE, "VDEX", "sectioned header")?;
    let section_count = u32_at(bytes, 8, "VDEX")?;
    let sections = parse_section_directory(bytes, section_count)?;
    let checksums = required(&sections, VdexSection::Checksums)?;
    if !checksums.len().is_multiple_of(CHECKSUM_SIZE) {
        return Err(Error::invalid(
            "VDEX",
            checksums.start,
            "checksum section size is not a multiple of four",
        ));
    }
    let dex_count = u32::try_from(checksums.len() / CHECKSUM_SIZE)
        .map_err(|_| Error::invalid("VDEX", checksums.start, "DEX count exceeds 32 bits"))?;
    let dex_files = required(&sections, VdexSection::DexFiles)?;
    let _ = required(&sections, VdexSection::VerifierDependencies)?;
    let members = inventory_members(bytes, dex_count, checksums, dex_files, false)?;
    Ok(VdexFile {
        version: VdexVersion::V027,
        layout: VdexLayout::Sectioned,
        bytes: bytes.to_vec(),
        sections,
        members,
    })
}

fn parse_section_directory(
    bytes: &[u8],
    section_count: u32,
) -> Result<BTreeMap<VdexSection, Range<usize>>> {
    if !(3..=4).contains(&section_count) {
        return Err(Error::invalid(
            "VDEX",
            8,
            format!("sectioned VDEX needs three or four sections, found {section_count}"),
        ));
    }
    let directory_size = usize::try_from(section_count)
        .ok()
        .and_then(|count| count.checked_mul(SECTION_HEADER_SIZE))
        .ok_or_else(|| Error::invalid("VDEX", 8, "section directory size overflowed"))?;
    range(
        bytes,
        SECTIONED_HEADER_SIZE,
        directory_size,
        "VDEX",
        "section directory",
    )?;
    let mut sections = BTreeMap::new();
    let mut physical = Vec::new();
    let mut seen_kinds = BTreeSet::new();
    let section_count = usize::try_from(section_count)
        .map_err(|_| Error::invalid("VDEX", 8, "section count is too large"))?;
    let directory_end = SECTIONED_HEADER_SIZE
        .checked_add(directory_size)
        .ok_or_else(|| Error::invalid("VDEX", 8, "section directory range overflowed"))?;
    for index in 0..section_count {
        let entry = SECTIONED_HEADER_SIZE + index * SECTION_HEADER_SIZE;
        let raw_kind = u32_at(bytes, entry, "VDEX")?;
        if usize::try_from(raw_kind).ok() != Some(index) {
            return Err(Error::invalid(
                "VDEX",
                entry,
                format!("section {index} declares out-of-order kind {raw_kind}"),
            ));
        }
        if !seen_kinds.insert(raw_kind) {
            return Err(Error::invalid(
                "VDEX",
                entry,
                format!("duplicate section kind {raw_kind}"),
            ));
        }
        let offset = u32_at(bytes, entry + 4, "VDEX")?;
        let size = u32_at(bytes, entry + 8, "VDEX")?;
        let kind = match raw_kind {
            0 => VdexSection::Checksums,
            1 => VdexSection::DexFiles,
            2 => VdexSection::VerifierDependencies,
            3 => VdexSection::TypeLookupTables,
            value => VdexSection::Unknown(value),
        };
        let section = if size == 0 {
            let position = usize::try_from(offset)
                .map_err(|_| Error::invalid("VDEX", entry + 4, "empty offset is too large"))?;
            if position > bytes.len() {
                return Err(Error::invalid(
                    "VDEX",
                    entry + 4,
                    "empty offset is out of bounds",
                ));
            }
            position..position
        } else {
            checked_range(offset, size, bytes.len(), "VDEX", "section")?
        };
        if !section.is_empty() {
            if section.start < directory_end {
                return Err(Error::invalid(
                    "VDEX",
                    entry + 4,
                    "section overlaps the VDEX header or directory",
                ));
            }
            physical.push((section.clone(), entry));
        }
        sections.insert(kind, section);
    }
    physical.sort_by_key(|(range, _)| range.start);
    for pair in physical.windows(2) {
        if pair[1].0.start < pair[0].0.end {
            return Err(Error::invalid(
                "VDEX",
                pair[1].1,
                "section directory ranges overlap",
            ));
        }
    }
    let declared_end = physical
        .iter()
        .map(|(range, _)| range.end)
        .max()
        .unwrap_or(directory_end);
    require_end(bytes, declared_end)?;
    Ok(sections)
}

fn inventory_members(
    bytes: &[u8],
    count: u32,
    checksums: &Range<usize>,
    dex_files: &Range<usize>,
    has_quickening_prefix: bool,
) -> Result<Vec<VdexDexMember>> {
    let mut cursor = dex_files.start;
    let capacity = usize::try_from(count)
        .map_err(|_| Error::invalid("VDEX", dex_files.start, "DEX count is too large"))?;
    let mut output = Vec::with_capacity(capacity);
    for index in 0..count {
        let quickening_table_offset = if has_quickening_prefix {
            let value = u32_at(bytes, cursor, "VDEX")?;
            cursor = cursor
                .checked_add(4)
                .ok_or_else(|| Error::invalid("VDEX", cursor, "member cursor overflowed"))?;
            Some(value)
        } else {
            None
        };
        let source_format = source_format(bytes, cursor)?;
        let file_size = u32_at(bytes, cursor + DEX_FILE_SIZE_OFFSET, "VDEX")?;
        let length = usize::try_from(file_size)
            .map_err(|_| Error::invalid("VDEX", cursor, "DEX member size is too large"))?;
        let end = cursor
            .checked_add(length)
            .ok_or_else(|| Error::invalid("VDEX", cursor, "DEX member range overflowed"))?;
        if end > dex_files.end {
            return Err(Error::invalid("VDEX", cursor, "truncated DEX member"));
        }
        let main_range = cursor..end;
        let checksum = u32_at(
            bytes,
            checksums.start
                + usize::try_from(index).map_err(|_| {
                    Error::invalid("VDEX", checksums.start, "DEX index is too large")
                })? * CHECKSUM_SIZE,
            "VDEX",
        )?;
        output.push(VdexDexMember {
            index,
            checksum,
            main_range: main_range.clone(),
            source_format,
            quickening_table_offset,
        });
        cursor = align_up(main_range.end, DEX_ALIGNMENT, "VDEX")?;
        if cursor > dex_files.end {
            return Err(Error::invalid(
                "VDEX",
                main_range.end,
                "DEX member alignment exceeds its section",
            ));
        }
    }
    if cursor != dex_files.end {
        let padding = bytes
            .get(cursor..dex_files.end)
            .ok_or_else(|| Error::invalid("VDEX", cursor, "DEX section range disappeared"))?;
        if padding.iter().any(|byte| *byte != 0) {
            return Err(Error::invalid(
                "VDEX",
                cursor,
                "DEX section has nonzero trailing bytes",
            ));
        }
    }
    Ok(output)
}

fn source_format(bytes: &[u8], offset: usize) -> Result<DexSourceFormat> {
    let magic = range(bytes, offset, 8, "VDEX", "DEX member magic")?;
    if magic.get(..4) == Some(b"dex\n") && magic.get(7) == Some(&0) {
        let digits: [u8; 3] = magic[4..7]
            .try_into()
            .map_err(|_| Error::invalid("VDEX", offset + 4, "truncated DEX version"))?;
        let version = DexVersion::from_digits(digits).ok_or_else(|| Error::UnsupportedVersion {
            format: "embedded DEX",
            version: String::from_utf8_lossy(&digits).into_owned(),
        })?;
        Ok(DexSourceFormat::Standard(version))
    } else if magic == b"cdex001\0" {
        Ok(DexSourceFormat::Compact(CompactDexVersion::V001))
    } else {
        Err(Error::invalid(
            "VDEX",
            offset,
            "member is neither supported DEX nor CompactDex",
        ))
    }
}

fn checksum_range(start: usize, count: u32, total: usize) -> Result<Range<usize>> {
    let bytes = usize::try_from(count)
        .ok()
        .and_then(|count| count.checked_mul(CHECKSUM_SIZE))
        .ok_or_else(|| Error::invalid("VDEX", start, "checksum section size overflowed"))?;
    let end = start
        .checked_add(bytes)
        .ok_or_else(|| Error::invalid("VDEX", start, "checksum section range overflowed"))?;
    if end <= total {
        Ok(start..end)
    } else {
        Err(Error::invalid("VDEX", start, "truncated checksum section"))
    }
}

fn following_range(start: usize, size: u32, total: usize, what: &str) -> Result<Range<usize>> {
    let length = usize::try_from(size)
        .map_err(|_| Error::invalid("VDEX", start, format!("{what} size is too large")))?;
    let end = start
        .checked_add(length)
        .ok_or_else(|| Error::invalid("VDEX", start, format!("{what} range overflowed")))?;
    if end <= total {
        Ok(start..end)
    } else {
        Err(Error::invalid("VDEX", start, format!("truncated {what}")))
    }
}

fn required(
    sections: &BTreeMap<VdexSection, Range<usize>>,
    kind: VdexSection,
) -> Result<&Range<usize>> {
    sections
        .get(&kind)
        .ok_or_else(|| Error::invalid("VDEX", 0, format!("missing {kind:?} section")))
}

fn require_end(bytes: &[u8], end: usize) -> Result<()> {
    if end == bytes.len() {
        Ok(())
    } else {
        Err(Error::invalid(
            "VDEX",
            end,
            format!(
                "{} trailing bytes after declared sections",
                bytes.len() - end
            ),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::{EMPTY_DEX_SECTION_VERSION, SPLIT_021_HEADER_SIZE, parse};
    use crate::vdex::{VdexLayout, VdexSection, VdexVersion};

    #[test]
    fn distinguishes_version_020_and_021_header_widths() {
        let mut version_020 = split_header(*b"020\0", &[1, 2], &[], &[]);
        version_020.truncate(20);
        version_020[16..20].copy_from_slice(&0u32.to_le_bytes());
        version_020.extend_from_slice(&7u32.to_le_bytes());
        let parsed_020 = parse(&version_020).unwrap();
        assert_eq!(parsed_020.version(), VdexVersion::V020);
        assert_eq!(parsed_020.layout(), VdexLayout::Split);
        assert_eq!(
            parsed_020.section(VdexSection::Checksums).unwrap(),
            7u32.to_le_bytes()
        );

        let version_021 = split_header(*b"021\0", &[1, 2], &[3], &[4, 5]);
        let parsed_021 = parse(&version_021).unwrap();
        assert_eq!(parsed_021.version(), VdexVersion::V021);
        assert_eq!(
            parsed_021
                .section(VdexSection::BootClasspathChecksums)
                .unwrap(),
            [3]
        );
        assert_eq!(
            parsed_021.section(VdexSection::ClassLoaderContext).unwrap(),
            [4, 5]
        );
    }

    fn split_header(version: [u8; 4], verifier: &[u8], boot: &[u8], context: &[u8]) -> Vec<u8> {
        let mut bytes = vec![0; SPLIT_021_HEADER_SIZE];
        bytes[..4].copy_from_slice(b"vdex");
        bytes[4..8].copy_from_slice(&version);
        bytes[8..12].copy_from_slice(EMPTY_DEX_SECTION_VERSION);
        bytes[12..16].copy_from_slice(&1u32.to_le_bytes());
        bytes[16..20].copy_from_slice(&u32::try_from(verifier.len()).unwrap().to_le_bytes());
        bytes[20..24].copy_from_slice(&u32::try_from(boot.len()).unwrap().to_le_bytes());
        bytes[24..28].copy_from_slice(&u32::try_from(context.len()).unwrap().to_le_bytes());
        bytes.extend_from_slice(&7u32.to_le_bytes());
        bytes.extend_from_slice(verifier);
        bytes.extend_from_slice(boot);
        bytes.extend_from_slice(context);
        bytes
    }
}

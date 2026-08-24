//! Deterministic VDEX 027 assembly.

use super::{VDEX_MAGIC, VdexFile};
use crate::binary::{align_up, put_u32, u32_at};
use crate::{Error, Result};

const HEADER_SIZE: usize = 12;
const SECTION_COUNT: usize = 4;
const SECTION_HEADER_SIZE: usize = 12;
const DIRECTORY_END: usize = HEADER_SIZE + SECTION_COUNT * SECTION_HEADER_SIZE;
const ALIGNMENT: usize = 4;

pub(super) fn sectioned(
    files: &[dex::DexFile],
    verifier_dependencies: &[u8],
    type_lookup_tables: &[u8],
) -> Result<VdexFile> {
    let mut dex_section = Vec::new();
    let mut checksums = Vec::with_capacity(files.len() * 4);
    for file in files {
        if file.version() == dex::DexVersion::V041 {
            return Err(Error::invalid(
                "VDEX",
                0,
                "DEX 041 containers must be supplied as a physical DexContainer",
            ));
        }
        let bytes = file.to_bytes()?;
        let checksum = u32_at(&bytes, 8, "DEX")?;
        checksums.extend_from_slice(&checksum.to_le_bytes());
        dex_section.extend_from_slice(&bytes);
        while !dex_section.len().is_multiple_of(ALIGNMENT) {
            dex_section.push(0);
        }
    }

    let checksum_offset = DIRECTORY_END;
    let dex_offset = align_up(
        checksum_offset
            .checked_add(checksums.len())
            .ok_or_else(|| Error::invalid("VDEX", checksum_offset, "checksum range overflowed"))?,
        ALIGNMENT,
        "VDEX",
    )?;
    let verifier_offset = align_up(
        dex_offset
            .checked_add(dex_section.len())
            .ok_or_else(|| Error::invalid("VDEX", dex_offset, "DEX section range overflowed"))?,
        ALIGNMENT,
        "VDEX",
    )?;
    let type_lookup_offset = align_up(
        verifier_offset
            .checked_add(verifier_dependencies.len())
            .ok_or_else(|| {
                Error::invalid("VDEX", verifier_offset, "verifier section range overflowed")
            })?,
        ALIGNMENT,
        "VDEX",
    )?;
    let total = type_lookup_offset
        .checked_add(type_lookup_tables.len())
        .ok_or_else(|| Error::invalid("VDEX", type_lookup_offset, "output size overflowed"))?;
    let mut output = vec![0; total];
    output[..VDEX_MAGIC.len()].copy_from_slice(VDEX_MAGIC);
    output[4..8].copy_from_slice(b"027\0");
    put_u32(
        &mut output,
        8,
        to_u32(SECTION_COUNT, "section count")?,
        "VDEX",
    )?;
    write_section(&mut output, 0, checksum_offset, checksums.len())?;
    write_section(&mut output, 1, dex_offset, dex_section.len())?;
    write_section(&mut output, 2, verifier_offset, verifier_dependencies.len())?;
    write_section(&mut output, 3, type_lookup_offset, type_lookup_tables.len())?;
    output[checksum_offset..checksum_offset + checksums.len()].copy_from_slice(&checksums);
    output[dex_offset..dex_offset + dex_section.len()].copy_from_slice(&dex_section);
    output[verifier_offset..verifier_offset + verifier_dependencies.len()]
        .copy_from_slice(verifier_dependencies);
    output[type_lookup_offset..].copy_from_slice(type_lookup_tables);
    VdexFile::parse(&output)
}

fn write_section(output: &mut [u8], kind: u32, offset: usize, size: usize) -> Result<()> {
    let kind_index = usize::try_from(kind)
        .map_err(|_| Error::invalid("VDEX", HEADER_SIZE, "section kind is too large"))?;
    let entry = HEADER_SIZE + kind_index * SECTION_HEADER_SIZE;
    put_u32(output, entry, kind, "VDEX")?;
    put_u32(output, entry + 4, to_u32(offset, "section offset")?, "VDEX")?;
    put_u32(output, entry + 8, to_u32(size, "section size")?, "VDEX")
}

fn to_u32(value: usize, what: &str) -> Result<u32> {
    u32::try_from(value)
        .map_err(|_| Error::invalid("VDEX", value, format!("{what} exceeds 32 bits")))
}

#[cfg(test)]
mod tests {
    use super::sectioned;

    #[test]
    fn builds_deterministic_sectioned_vdex() {
        let dex = dex::DexFile::new(dex::DexVersion::V040);
        let first = sectioned(std::slice::from_ref(&dex), &[1, 2], &[3, 4]).unwrap();
        let second = sectioned(&[dex], &[1, 2], &[3, 4]).unwrap();
        assert_eq!(first.to_bytes().unwrap(), second.to_bytes().unwrap());
        assert_eq!(first.members().len(), 1);
        assert_eq!(
            first.canonical_standard_dex(0).unwrap().version(),
            dex::DexVersion::V040
        );
    }
}

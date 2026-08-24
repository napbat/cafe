//! Legacy Dalvik optimized DEX (`dey\n036`) containers.

use crate::binary::{adler32, align_up, checked_range, put_u32, range, u32_at};
use crate::{Error, Result};

/// Complete supported ODEX magic and version.
pub const ODEX_MAGIC: &[u8; 8] = b"dey\n036\0";
/// Fixed ODEX 036 header width.
pub const ODEX_HEADER_SIZE: usize = 40;

const DEX_OFFSET_FIELD: usize = 8;
const DEX_LENGTH_FIELD: usize = 12;
const DEPS_OFFSET_FIELD: usize = 16;
const DEPS_LENGTH_FIELD: usize = 20;
const OPT_OFFSET_FIELD: usize = 24;
const OPT_LENGTH_FIELD: usize = 28;
const FLAGS_FIELD: usize = 32;
const CHECKSUM_FIELD: usize = 36;
const SECTION_ALIGNMENT: usize = 8;

/// ODEX optimizer flags, retaining unknown bits.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct OdexFlags(u32);

impl OdexFlags {
    /// File values use reverse byte order.
    pub const BIG_ENDIAN: Self = Self(1 << 1);

    /// Retains all encoded bits.
    #[must_use]
    pub const fn from_bits_retain(bits: u32) -> Self {
        Self(bits)
    }

    /// Returns exact encoded bits.
    #[must_use]
    pub const fn bits(self) -> u32 {
        self.0
    }

    /// Returns whether all supplied flags are present.
    #[must_use]
    pub const fn contains(self, flags: Self) -> bool {
        self.0 & flags.0 == flags.0
    }
}

/// Typed ODEX 036 header.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct OdexHeader {
    /// Embedded DEX offset.
    pub dex_offset: u32,
    /// Embedded DEX length.
    pub dex_length: u32,
    /// Dependency-table offset.
    pub dependencies_offset: u32,
    /// Dependency-table length.
    pub dependencies_length: u32,
    /// Optimizer-data offset.
    pub optimized_offset: u32,
    /// Optimizer-data length.
    pub optimized_length: u32,
    /// Exact optimizer flags.
    pub flags: OdexFlags,
    /// Adler-32 covering the dependencies-through-optimizer range.
    pub checksum: u32,
}

/// Parsed legacy ODEX container with opaque dependency and optimizer state.
#[derive(Debug, Clone)]
pub struct OdexFile {
    header: OdexHeader,
    bytes: Vec<u8>,
}

impl OdexFile {
    /// Parses ODEX version 036 and validates all section ranges and checksum.
    ///
    /// # Errors
    ///
    /// Returns an error for other versions, malformed alignment, overlapping
    /// sections, truncated payloads, or checksum mismatches.
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        range(bytes, 0, ODEX_HEADER_SIZE, "ODEX", "header")?;
        if bytes.get(..ODEX_MAGIC.len()) != Some(ODEX_MAGIC) {
            let version = bytes.get(4..8).map_or_else(
                || "truncated".to_owned(),
                |value| {
                    String::from_utf8_lossy(value)
                        .trim_end_matches('\0')
                        .to_owned()
                },
            );
            return Err(Error::UnsupportedVersion {
                format: "ODEX",
                version,
            });
        }
        let header = OdexHeader {
            dex_offset: u32_at(bytes, DEX_OFFSET_FIELD, "ODEX")?,
            dex_length: u32_at(bytes, DEX_LENGTH_FIELD, "ODEX")?,
            dependencies_offset: u32_at(bytes, DEPS_OFFSET_FIELD, "ODEX")?,
            dependencies_length: u32_at(bytes, DEPS_LENGTH_FIELD, "ODEX")?,
            optimized_offset: u32_at(bytes, OPT_OFFSET_FIELD, "ODEX")?,
            optimized_length: u32_at(bytes, OPT_LENGTH_FIELD, "ODEX")?,
            flags: OdexFlags::from_bits_retain(u32_at(bytes, FLAGS_FIELD, "ODEX")?),
            checksum: u32_at(bytes, CHECKSUM_FIELD, "ODEX")?,
        };
        validate(&header, bytes)?;
        Ok(Self {
            header,
            bytes: bytes.to_vec(),
        })
    }

    /// Constructs ODEX 036 from an embedded DEX and exact opaque metadata.
    ///
    /// Sections are aligned and the ODEX checksum is regenerated.
    ///
    /// # Errors
    ///
    /// Returns an error when output coordinates exceed 32 bits.
    pub fn from_parts(
        dex: &[u8],
        dependencies: &[u8],
        optimized: &[u8],
        flags: OdexFlags,
    ) -> Result<Self> {
        let dex_offset = align_up(ODEX_HEADER_SIZE, SECTION_ALIGNMENT, "ODEX")?;
        let dependencies_offset = align_up(
            dex_offset
                .checked_add(dex.len())
                .ok_or_else(|| Error::invalid("ODEX", dex_offset, "DEX range overflowed"))?,
            SECTION_ALIGNMENT,
            "ODEX",
        )?;
        let optimized_offset = align_up(
            dependencies_offset
                .checked_add(dependencies.len())
                .ok_or_else(|| {
                    Error::invalid("ODEX", dependencies_offset, "dependency range overflowed")
                })?,
            SECTION_ALIGNMENT,
            "ODEX",
        )?;
        let total = optimized_offset
            .checked_add(optimized.len())
            .ok_or_else(|| Error::invalid("ODEX", optimized_offset, "output size overflowed"))?;
        let mut bytes = vec![0; total];
        bytes[..ODEX_MAGIC.len()].copy_from_slice(ODEX_MAGIC);
        bytes[dex_offset..dex_offset + dex.len()].copy_from_slice(dex);
        bytes[dependencies_offset..dependencies_offset + dependencies.len()]
            .copy_from_slice(dependencies);
        bytes[optimized_offset..].copy_from_slice(optimized);
        let header = OdexHeader {
            dex_offset: to_u32(dex_offset, "DEX offset")?,
            dex_length: to_u32(dex.len(), "DEX length")?,
            dependencies_offset: to_u32(dependencies_offset, "dependency offset")?,
            dependencies_length: to_u32(dependencies.len(), "dependency length")?,
            optimized_offset: to_u32(optimized_offset, "optimizer offset")?,
            optimized_length: to_u32(optimized.len(), "optimizer length")?,
            flags,
            checksum: adler32(&bytes[dependencies_offset..]),
        };
        write_header(&mut bytes, header)?;
        validate(&header, &bytes)?;
        Ok(Self { header, bytes })
    }

    /// Returns the typed header.
    #[must_use]
    pub const fn header(&self) -> &OdexHeader {
        &self.header
    }

    /// Returns embedded DEX bytes, which may still contain optimized opcodes.
    #[must_use]
    pub fn dex_bytes(&self) -> &[u8] {
        section(&self.bytes, self.header.dex_offset, self.header.dex_length)
    }

    /// Returns the opaque dependency table exactly.
    #[must_use]
    pub fn dependencies(&self) -> &[u8] {
        section(
            &self.bytes,
            self.header.dependencies_offset,
            self.header.dependencies_length,
        )
    }

    /// Returns opaque optimizer chunks exactly.
    #[must_use]
    pub fn optimized_data(&self) -> &[u8] {
        section(
            &self.bytes,
            self.header.optimized_offset,
            self.header.optimized_length,
        )
    }

    /// Parses the embedded bytes only if they are already canonical standard DEX.
    ///
    /// # Errors
    ///
    /// Returns a DEX error when legacy optimized instructions still need a
    /// caller-supplied resolution plan.
    pub fn canonical_dex(&self) -> Result<dex::DexFile> {
        dex::DexFile::parse(self.dex_bytes()).map_err(Error::from)
    }

    /// Reassembles the exact ODEX bytes.
    ///
    /// # Errors
    ///
    /// This immutable representation is already validated and cannot fail.
    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        Ok(self.bytes.clone())
    }
}

fn validate(header: &OdexHeader, bytes: &[u8]) -> Result<()> {
    let dex = checked_range(
        header.dex_offset,
        header.dex_length,
        bytes.len(),
        "ODEX",
        "embedded DEX",
    )?;
    let dependencies = checked_range(
        header.dependencies_offset,
        header.dependencies_length,
        bytes.len(),
        "ODEX",
        "dependency table",
    )?;
    let optimized = checked_range(
        header.optimized_offset,
        header.optimized_length,
        bytes.len(),
        "ODEX",
        "optimizer data",
    )?;
    for (name, offset) in [
        ("embedded DEX", dex.start),
        ("dependency table", dependencies.start),
        ("optimizer data", optimized.start),
    ] {
        if !offset.is_multiple_of(SECTION_ALIGNMENT) {
            return Err(Error::invalid(
                "ODEX",
                offset,
                format!("{name} is not eight-byte aligned"),
            ));
        }
    }
    if dex.start < ODEX_HEADER_SIZE
        || dependencies.start < dex.end
        || optimized.start < dependencies.end
    {
        return Err(Error::invalid(
            "ODEX",
            0,
            "sections overlap or are not in DEX/dependencies/optimizer order",
        ));
    }
    let covered = bytes
        .get(dependencies.start..optimized.end)
        .ok_or_else(|| Error::invalid("ODEX", dependencies.start, "checksum range is truncated"))?;
    let actual = adler32(covered);
    if actual != header.checksum {
        return Err(Error::invalid(
            "ODEX",
            CHECKSUM_FIELD,
            format!(
                "checksum mismatch: stored 0x{:08x}, calculated 0x{actual:08x}",
                header.checksum
            ),
        ));
    }
    Ok(())
}

fn write_header(bytes: &mut [u8], header: OdexHeader) -> Result<()> {
    bytes[..ODEX_MAGIC.len()].copy_from_slice(ODEX_MAGIC);
    for (offset, value) in [
        (DEX_OFFSET_FIELD, header.dex_offset),
        (DEX_LENGTH_FIELD, header.dex_length),
        (DEPS_OFFSET_FIELD, header.dependencies_offset),
        (DEPS_LENGTH_FIELD, header.dependencies_length),
        (OPT_OFFSET_FIELD, header.optimized_offset),
        (OPT_LENGTH_FIELD, header.optimized_length),
        (FLAGS_FIELD, header.flags.bits()),
        (CHECKSUM_FIELD, header.checksum),
    ] {
        put_u32(bytes, offset, value, "ODEX")?;
    }
    Ok(())
}

fn section(bytes: &[u8], offset: u32, length: u32) -> &[u8] {
    let start = offset as usize;
    &bytes[start..start + length as usize]
}

fn to_u32(value: usize, what: &str) -> Result<u32> {
    u32::try_from(value)
        .map_err(|_| Error::invalid("ODEX", value, format!("{what} exceeds 32 bits")))
}

#[cfg(test)]
mod tests {
    use super::{OdexFile, OdexFlags};

    #[test]
    fn builds_and_preserves_all_opaque_sections() {
        let file =
            OdexFile::from_parts(&[1, 2, 3], &[4, 5], &[6, 7, 8], OdexFlags::default()).unwrap();
        let bytes = file.to_bytes().unwrap();
        let parsed = OdexFile::parse(&bytes).unwrap();
        assert_eq!(parsed.dex_bytes(), [1, 2, 3]);
        assert_eq!(parsed.dependencies(), [4, 5]);
        assert_eq!(parsed.optimized_data(), [6, 7, 8]);
        assert_eq!(parsed.to_bytes().unwrap(), bytes);
    }

    #[test]
    fn rejects_corrupted_optimizer_checksum() {
        let file = OdexFile::from_parts(&[1], &[2], &[3], OdexFlags::default()).unwrap();
        let mut bytes = file.to_bytes().unwrap();
        *bytes.last_mut().unwrap() ^= 1;
        assert!(OdexFile::parse(&bytes).is_err());
    }
}

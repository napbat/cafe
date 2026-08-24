//! OAT metadata discovery without parsing ELF or native instructions.

use crate::binary::{put_u32, range, u32_at};
use crate::{Error, Result};

/// Stable OAT header signature.
pub const OAT_MAGIC: &[u8; 4] = b"oat\n";
/// Stable prefix width through the checksum field.
pub const OAT_STABLE_PREFIX_SIZE: usize = 12;

const FIRST_SUPPORTED_VERSION: u16 = 20;
const VERSION_OFFSET: usize = 4;
const VERSION_DIGITS: usize = 3;
const VERSION_TERMINATOR_OFFSET: usize = 7;
const CHECKSUM_OFFSET: usize = 8;

/// Numeric three-digit OAT version.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct OatVersion([u8; VERSION_DIGITS]);

impl OatVersion {
    /// Parses three ASCII decimal digits in ART's supported stable-prefix range.
    ///
    /// # Errors
    ///
    /// Returns an error for nondecimal digits or versions predating OAT 020.
    pub fn from_digits(digits: [u8; VERSION_DIGITS]) -> Result<Self> {
        if !digits.iter().all(u8::is_ascii_digit) {
            return Err(Error::UnsupportedVersion {
                format: "OAT",
                version: String::from_utf8_lossy(&digits).into_owned(),
            });
        }
        let number = u16::from(digits[0] - b'0') * 100
            + u16::from(digits[1] - b'0') * 10
            + u16::from(digits[2] - b'0');
        if number < FIRST_SUPPORTED_VERSION {
            return Err(Error::UnsupportedVersion {
                format: "OAT",
                version: String::from_utf8_lossy(&digits).into_owned(),
            });
        }
        Ok(Self(digits))
    }

    /// Creates a version from its numeric value.
    ///
    /// # Errors
    ///
    /// Returns an error outside the explicit `020..=999` prefix range.
    pub fn from_number(number: u16) -> Result<Self> {
        if !(FIRST_SUPPORTED_VERSION..=999).contains(&number) {
            return Err(Error::UnsupportedVersion {
                format: "OAT",
                version: number.to_string(),
            });
        }
        let text = format!("{number:03}");
        let digits = text.as_bytes().try_into().map_err(|_| {
            Error::invalid("OAT", VERSION_OFFSET, "numeric version width is not three")
        })?;
        Ok(Self(digits))
    }

    /// Returns the exact three version digits.
    #[must_use]
    pub const fn digits(self) -> [u8; VERSION_DIGITS] {
        self.0
    }

    /// Returns the numeric version.
    #[must_use]
    pub fn number(self) -> u16 {
        u16::from(self.0[0] - b'0') * 100
            + u16::from(self.0[1] - b'0') * 10
            + u16::from(self.0[2] - b'0')
    }
}

impl std::fmt::Display for OatVersion {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&String::from_utf8_lossy(&self.0))
    }
}

/// Stable OAT prefix metadata.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct OatHeader {
    /// Byte offset of the OAT header in the supplied file or ELF container.
    pub offset: usize,
    /// OAT format version.
    pub version: OatVersion,
    /// Stored OAT metadata checksum.
    pub checksum: u32,
}

/// OAT metadata plus exact opaque bytes, including any enclosing ELF file.
#[derive(Debug, Clone)]
pub struct OatFile {
    header: OatHeader,
    bytes: Vec<u8>,
}

impl OatFile {
    /// Returns whether bytes contain exactly one syntactically valid OAT header.
    #[must_use]
    pub fn contains_header(bytes: &[u8]) -> bool {
        find_headers(bytes).len() == 1
    }

    /// Finds the stable OAT prefix in a direct OAT region or opaque ELF file.
    ///
    /// Only magic, version, and checksum are interpreted. Architecture fields,
    /// ELF metadata, compiled code, and relocation state remain opaque.
    ///
    /// # Errors
    ///
    /// Returns an error when there is no unique valid header or its version is
    /// outside the explicit `020..=999` stable-prefix range.
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        let candidates = find_headers(bytes);
        let offset = match candidates.as_slice() {
            [offset] => *offset,
            [] => return Err(Error::invalid("OAT", 0, "no valid OAT header was found")),
            _ => {
                return Err(Error::invalid(
                    "OAT",
                    candidates[1],
                    "more than one valid OAT header was found",
                ));
            }
        };
        range(
            bytes,
            offset,
            OAT_STABLE_PREFIX_SIZE,
            "OAT",
            "stable header prefix",
        )?;
        let digits: [u8; VERSION_DIGITS] = bytes
            [offset + VERSION_OFFSET..offset + VERSION_OFFSET + VERSION_DIGITS]
            .try_into()
            .map_err(|_| Error::invalid("OAT", offset + VERSION_OFFSET, "truncated version"))?;
        let header = OatHeader {
            offset,
            version: OatVersion::from_digits(digits)?,
            checksum: u32_at(bytes, offset + CHECKSUM_OFFSET, "OAT")?,
        };
        Ok(Self {
            header,
            bytes: bytes.to_vec(),
        })
    }

    /// Builds a direct OAT metadata region with opaque bytes after the stable prefix.
    ///
    /// # Errors
    ///
    /// Returns an error only if the fixed output header cannot be written.
    pub fn from_payload(version: OatVersion, checksum: u32, payload: &[u8]) -> Result<Self> {
        let mut bytes = vec![0; OAT_STABLE_PREFIX_SIZE];
        bytes[..OAT_MAGIC.len()].copy_from_slice(OAT_MAGIC);
        bytes[VERSION_OFFSET..VERSION_OFFSET + VERSION_DIGITS].copy_from_slice(&version.digits());
        bytes[VERSION_TERMINATOR_OFFSET] = 0;
        put_u32(&mut bytes, CHECKSUM_OFFSET, checksum, "OAT")?;
        bytes.extend_from_slice(payload);
        Ok(Self {
            header: OatHeader {
                offset: 0,
                version,
                checksum,
            },
            bytes,
        })
    }

    /// Returns stable OAT metadata.
    #[must_use]
    pub const fn header(&self) -> &OatHeader {
        &self.header
    }

    /// Returns all bytes after the stable prefix within the OAT region.
    ///
    /// For an ELF-backed file this includes native sections and remains opaque.
    #[must_use]
    pub fn opaque_payload(&self) -> &[u8] {
        &self.bytes[self.header.offset + OAT_STABLE_PREFIX_SIZE..]
    }

    /// Returns the complete exact input, including an enclosing ELF container.
    ///
    /// # Errors
    ///
    /// This immutable operation currently cannot fail; the result form keeps
    /// serialization APIs uniform if checked rewriting is added later.
    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        Ok(self.bytes.clone())
    }
}

fn find_headers(bytes: &[u8]) -> Vec<usize> {
    if bytes.len() < OAT_STABLE_PREFIX_SIZE {
        return Vec::new();
    }
    bytes
        .windows(OAT_MAGIC.len())
        .enumerate()
        .filter_map(|(offset, magic)| {
            let version_end = offset.checked_add(VERSION_TERMINATOR_OFFSET + 1)?;
            let digits = bytes.get(offset + VERSION_OFFSET..offset + VERSION_TERMINATOR_OFFSET)?;
            (magic == OAT_MAGIC
                && version_end <= bytes.len()
                && digits.iter().all(u8::is_ascii_digit)
                && bytes[offset + VERSION_TERMINATOR_OFFSET] == 0
                && offset + OAT_STABLE_PREFIX_SIZE <= bytes.len())
            .then_some(offset)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{OatFile, OatVersion};

    #[test]
    fn discovers_and_preserves_an_elf_embedded_header() {
        let direct = OatFile::from_payload(OatVersion::from_number(181).unwrap(), 7, &[1, 2])
            .unwrap()
            .to_bytes()
            .unwrap();
        let mut elf_like = vec![0x7f, b'E', b'L', b'F', 0, 0];
        elf_like.extend_from_slice(&direct);
        let parsed = OatFile::parse(&elf_like).unwrap();
        assert_eq!(parsed.header().offset, 6);
        assert_eq!(parsed.header().version.number(), 181);
        assert_eq!(parsed.to_bytes().unwrap(), elf_like);
    }

    #[test]
    fn rejects_ambiguous_headers() {
        let one = OatFile::from_payload(OatVersion::from_number(64).unwrap(), 0, &[])
            .unwrap()
            .to_bytes()
            .unwrap();
        let mut two = one.clone();
        two.extend_from_slice(&one);
        assert!(OatFile::parse(&two).is_err());
    }
}

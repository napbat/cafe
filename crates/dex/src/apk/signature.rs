//! APK v1 and signing-block detection with explicit rewrite policies.

use std::fmt;

use super::layout::{
    SIGNING_BLOCK_ID_FIELD_WIDTH, SIGNING_BLOCK_MAGIC, SIGNING_BLOCK_MINIMUM_REPORTED_SIZE,
    SIGNING_BLOCK_SIZE_FIELD_WIDTH, SIGNING_BLOCK_TRAILER_SIZE, read_u32, read_u64, zip_sections,
};
use super::{ApkFile, EntryId};
use crate::{Error, Result};

/// Conventional JAR-signing manifest entry used by APK Signature Scheme v1.
pub const V1_MANIFEST_ENTRY: &str = "META-INF/MANIFEST.MF";

/// Source-stamp certificate digest entry paired with a signing-block stamp.
pub const SOURCE_STAMP_CERTIFICATE_ENTRY: &str = "stamp-cert-sha256";

const V1_METADATA_PREFIX: &str = "META-INF/";
const V1_SIGNATURE_PREFIX: &str = "SIG-";
const V1_SIGNATURE_SUFFIXES: &[&str] = &[".SF", ".DSA", ".RSA", ".EC"];

/// Strongly typed identifier for an APK signing-block entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SigningBlockId(u32);

impl SigningBlockId {
    /// APK Signature Scheme v2 block.
    pub const V2: Self = Self(0x7109_871a);
    /// APK Signature Scheme v3.0 block.
    pub const V3: Self = Self(0xf053_68c0);
    /// APK Signature Scheme v3.1 block.
    pub const V31: Self = Self(0x1b93_ad61);
    /// APK Signature Scheme v3.2 hybrid block.
    pub const V32: Self = Self(0x70e1_c89f);
    /// APK source-stamp block.
    pub const SOURCE_STAMP: Self = Self(0x6dff_800d);

    /// Retains an arbitrary signing-block entry identifier.
    #[must_use]
    pub const fn new(value: u32) -> Self {
        Self(value)
    }

    /// Returns the exact encoded identifier.
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }

    /// Classifies a known Android signing or source-stamp identifier.
    #[must_use]
    pub const fn kind(self) -> SigningBlockKind {
        match self {
            Self::V2 => SigningBlockKind::V2,
            Self::V3 => SigningBlockKind::V3,
            Self::V31 => SigningBlockKind::V31,
            Self::V32 => SigningBlockKind::V32,
            Self::SOURCE_STAMP => SigningBlockKind::SourceStamp,
            _ => SigningBlockKind::Unknown,
        }
    }
}

impl fmt::Display for SigningBlockId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "0x{:08x}", self.0)
    }
}

/// Semantic kind of a known APK signing-block entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum SigningBlockKind {
    /// APK Signature Scheme v2.
    V2,
    /// APK Signature Scheme v3.0.
    V3,
    /// APK Signature Scheme v3.1.
    V31,
    /// APK Signature Scheme v3.2 hybrid signatures.
    V32,
    /// APK source stamp.
    SourceStamp,
    /// Unknown extension entry retained by numeric ID.
    Unknown,
}

/// One exact ID-value pair in an APK signing block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SigningBlockEntry {
    /// Typed entry identifier.
    pub id: SigningBlockId,
    /// Exact entry value, excluding its length and ID fields.
    pub value: Vec<u8>,
}

/// Parsed APK signing block with exact bytes retained for explicit preservation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SigningBlock {
    entries: Vec<SigningBlockEntry>,
    raw: Vec<u8>,
}

impl SigningBlock {
    /// Returns ID-value pairs in their encoded order.
    #[must_use]
    pub fn entries(&self) -> &[SigningBlockEntry] {
        &self.entries
    }

    /// Returns the exact encoded block, including both size fields and magic.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.raw
    }

    /// Returns whether the block contains an exact identifier.
    #[must_use]
    pub fn contains(&self, id: SigningBlockId) -> bool {
        self.entries.iter().any(|entry| entry.id == id)
    }
}

/// Observable signature state of an editable APK.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SignatureState {
    /// No v1 artifacts or APK signing block are present.
    Unsigned,
    /// Signature material is present and the archive is unchanged.
    Present,
    /// Signature material remains after a mutation and cannot be valid.
    PotentiallyInvalidated,
}

/// Policy applied when serializing a mutated signed APK.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum SignaturePolicy {
    /// Reject rewrites that would silently invalidate signature material.
    #[default]
    Reject,
    /// Retain v1 files and reinsert the exact signing block as stale metadata.
    Preserve,
    /// Remove v1 files, source-stamp metadata, and the APK signing block.
    Strip,
}

/// Returns whether an entry is a top-level APK Signature Scheme v1 artifact.
#[must_use]
pub fn is_v1_signature_entry(name: &str) -> bool {
    let Some(remainder) = strip_prefix_ascii_case(name, V1_METADATA_PREFIX) else {
        return false;
    };
    if remainder.is_empty() || remainder.contains('/') {
        return false;
    }
    let uppercase = remainder.to_ascii_uppercase();
    uppercase.starts_with(V1_SIGNATURE_PREFIX)
        || V1_SIGNATURE_SUFFIXES
            .iter()
            .any(|suffix| uppercase.ends_with(suffix))
}

impl ApkFile {
    /// Returns the parsed APK signing block, if present.
    #[must_use]
    pub const fn signing_block(&self) -> Option<&SigningBlock> {
        self.signing_block.as_ref()
    }

    /// Returns stable IDs of v1 signature files in archive order.
    #[must_use]
    pub fn v1_signature_entry_ids(&self) -> Vec<EntryId> {
        self.entries
            .iter()
            .filter(|entry| is_v1_signature_entry(&entry.name))
            .map(|entry| entry.id)
            .collect()
    }

    /// Returns whether any v1, v2+, source-stamp, or unknown signing block exists.
    #[must_use]
    pub fn has_signature_artifacts(&self) -> bool {
        self.signing_block.is_some() || !self.v1_signature_entry_ids().is_empty()
    }

    /// Returns whether signature material is pristine or potentially stale.
    #[must_use]
    pub fn signature_state(&self) -> SignatureState {
        if !self.has_signature_artifacts() {
            SignatureState::Unsigned
        } else if self.dirty {
            SignatureState::PotentiallyInvalidated
        } else {
            SignatureState::Present
        }
    }

    pub(super) fn strip_signature_artifacts(&mut self) -> usize {
        let before = self.entries.len();
        self.entries.retain(|entry| {
            !is_v1_signature_entry(&entry.name)
                && !entry.name.eq_ignore_ascii_case(V1_MANIFEST_ENTRY)
                && !entry
                    .name
                    .eq_ignore_ascii_case(SOURCE_STAMP_CERTIFICATE_ENTRY)
        });
        let removed_entries = before - self.entries.len();
        let removed_block = usize::from(self.signing_block.take().is_some());
        if removed_entries != 0 || removed_block != 0 {
            self.dirty = true;
        }
        removed_entries + removed_block
    }
}

pub(super) fn parse_signing_block(bytes: &[u8]) -> Result<Option<SigningBlock>> {
    let sections = zip_sections(bytes)?;
    let Some(magic_start) = sections
        .central_directory
        .checked_sub(SIGNING_BLOCK_MAGIC.len())
    else {
        return Ok(None);
    };
    if bytes.get(magic_start..sections.central_directory) != Some(SIGNING_BLOCK_MAGIC.as_slice()) {
        return Ok(None);
    }
    let footer_size_offset = magic_start
        .checked_sub(SIGNING_BLOCK_SIZE_FIELD_WIDTH)
        .ok_or_else(|| Error::invalid_apk("APK signing-block footer is truncated"))?;
    let reported_size = read_u64(bytes, footer_size_offset)?;
    if reported_size < SIGNING_BLOCK_MINIMUM_REPORTED_SIZE {
        return Err(Error::invalid_apk("APK signing-block size is too small"));
    }
    let total_size = reported_size
        .checked_add(
            u64::try_from(SIGNING_BLOCK_SIZE_FIELD_WIDTH)
                .expect("signing-block size field width fits u64"),
        )
        .and_then(|value| usize::try_from(value).ok())
        .ok_or_else(|| Error::invalid_apk("APK signing-block size exceeds this platform"))?;
    let block_start = sections
        .central_directory
        .checked_sub(total_size)
        .ok_or_else(|| Error::invalid_apk("APK signing-block start precedes the archive"))?;
    if read_u64(bytes, block_start)? != reported_size {
        return Err(Error::invalid_apk(
            "APK signing-block size fields do not match",
        ));
    }

    let pairs_start = block_start
        .checked_add(SIGNING_BLOCK_SIZE_FIELD_WIDTH)
        .ok_or_else(|| Error::invalid_apk("APK signing-block pair offset overflowed"))?;
    let pairs_end = sections
        .central_directory
        .checked_sub(SIGNING_BLOCK_TRAILER_SIZE)
        .ok_or_else(|| Error::invalid_apk("APK signing-block trailer is truncated"))?;
    let mut cursor = pairs_start;
    let mut entries = Vec::new();
    while cursor < pairs_end {
        let pair_size = read_u64(bytes, cursor)?;
        if pair_size
            < u64::try_from(SIGNING_BLOCK_ID_FIELD_WIDTH)
                .expect("signing-block ID field width fits u64")
        {
            return Err(Error::invalid_apk(
                "APK signing-block pair is shorter than its ID",
            ));
        }
        cursor = cursor
            .checked_add(SIGNING_BLOCK_SIZE_FIELD_WIDTH)
            .ok_or_else(|| Error::invalid_apk("APK signing-block pair offset overflowed"))?;
        let pair_size = usize::try_from(pair_size)
            .map_err(|_| Error::invalid_apk("APK signing-block pair exceeds this platform"))?;
        let entry_end = cursor
            .checked_add(pair_size)
            .filter(|end| *end <= pairs_end)
            .ok_or_else(|| Error::invalid_apk("APK signing-block pair is truncated"))?;
        let id = SigningBlockId::new(read_u32(bytes, cursor)?);
        let value_start = cursor
            .checked_add(SIGNING_BLOCK_ID_FIELD_WIDTH)
            .ok_or_else(|| Error::invalid_apk("APK signing-block value offset overflowed"))?;
        entries.push(SigningBlockEntry {
            id,
            value: bytes[value_start..entry_end].to_vec(),
        });
        cursor = entry_end;
    }
    if cursor != pairs_end {
        return Err(Error::invalid_apk(
            "APK signing-block pairs do not fill the block",
        ));
    }
    Ok(Some(SigningBlock {
        entries,
        raw: bytes[block_start..sections.central_directory].to_vec(),
    }))
}

fn strip_prefix_ascii_case<'a>(value: &'a str, prefix: &str) -> Option<&'a str> {
    let candidate = value.get(..prefix.len())?;
    candidate
        .eq_ignore_ascii_case(prefix)
        .then(|| &value[prefix.len()..])
}

#[cfg(test)]
mod tests {
    use super::{SIGNING_BLOCK_MAGIC, SigningBlockId, SigningBlockKind, is_v1_signature_entry};
    use crate::apk::edit::insert_signing_block;
    use crate::apk::{ApkFile, SignaturePolicy, SignatureState};

    const TEST_PAIR_VALUE: &[u8] = b"signature";

    #[test]
    fn recognizes_v1_files_and_typed_block_ids() {
        assert!(is_v1_signature_entry("META-INF/APP.SF"));
        assert!(is_v1_signature_entry("meta-inf/APP.RSA"));
        assert!(!is_v1_signature_entry("META-INF/MANIFEST.MF"));
        assert!(!is_v1_signature_entry("META-INF/nested/APP.SF"));
        assert_eq!(SigningBlockId::V2.kind(), SigningBlockKind::V2);
        assert_eq!(SigningBlockId::V32.kind(), SigningBlockKind::V32);
        assert_eq!(SigningBlockId::new(7).kind(), SigningBlockKind::Unknown);
    }

    #[test]
    fn detects_preserves_and_strips_whole_file_signing_blocks() {
        let mut unsigned = ApkFile::new();
        unsigned.add_file("asset.txt", b"before".to_vec()).unwrap();
        let unsigned_bytes = unsigned.to_bytes().unwrap();
        let block = test_signing_block(SigningBlockId::V2, TEST_PAIR_VALUE);
        let signed_bytes = insert_signing_block(unsigned_bytes, &block).unwrap();

        let mut signed = ApkFile::from_bytes(signed_bytes).unwrap();
        assert_eq!(signed.signature_state(), SignatureState::Present);
        assert!(signed.signing_block().unwrap().contains(SigningBlockId::V2));
        signed.put_file("asset.txt", b"after".to_vec()).unwrap();
        assert!(signed.to_bytes().is_err());

        let preserved = signed
            .to_bytes_with_signature_policy(SignaturePolicy::Preserve)
            .unwrap();
        assert!(
            ApkFile::from_bytes(preserved)
                .unwrap()
                .signing_block()
                .unwrap()
                .contains(SigningBlockId::V2)
        );

        let stripped = signed
            .to_bytes_with_signature_policy(SignaturePolicy::Strip)
            .unwrap();
        assert!(
            ApkFile::from_bytes(stripped)
                .unwrap()
                .signing_block()
                .is_none()
        );
    }

    fn test_signing_block(id: SigningBlockId, value: &[u8]) -> Vec<u8> {
        let pair_size = u64::try_from(size_of::<u32>() + value.len()).unwrap();
        let reported_size = u64::try_from(size_of::<u64>()).unwrap()
            + pair_size
            + u64::try_from(size_of::<u64>() + SIGNING_BLOCK_MAGIC.len()).unwrap();
        let mut block = Vec::new();
        block.extend_from_slice(&reported_size.to_le_bytes());
        block.extend_from_slice(&pair_size.to_le_bytes());
        block.extend_from_slice(&id.get().to_le_bytes());
        block.extend_from_slice(value);
        block.extend_from_slice(&reported_size.to_le_bytes());
        block.extend_from_slice(SIGNING_BLOCK_MAGIC);
        block
    }
}

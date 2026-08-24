//! Bounds-checked ART compact offset tables.

use crate::file::Endian;
use crate::{Error, Result};

const ELEMENTS_PER_BLOCK: usize = 16;
const BIT_MASK_BYTES: usize = 2;
const EMBEDDED_HEADER_BYTES: usize = 8;
const WORD_BYTES: usize = 4;
const ULEB_PAYLOAD_MASK: u8 = 0x7f;
const ULEB_CONTINUATION: u8 = 0x80;
const ULEB_GROUP_BITS: u32 = 7;

/// Decoded ART compact offset table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompactOffsetTable {
    offsets: Vec<u32>,
}

/// Binary components for a compact offset table whose base values live elsewhere.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EncodedCompactOffsetTable {
    /// LEB blocks followed by the aligned block-index table.
    pub data: Vec<u8>,
    /// Minimum nonzero absolute offset.
    pub minimum_offset: u32,
    /// Byte offset of the block-index table within `data`.
    pub table_offset: u32,
}

impl CompactOffsetTable {
    /// Creates a table from exact absolute offsets, where zero means absent.
    ///
    /// # Errors
    ///
    /// Returns an error when nonzero values decrease within a 16-element block,
    /// because ART's unsigned deltas cannot represent that sequence.
    pub fn new(offsets: Vec<u32>) -> Result<Self> {
        validate_encodable(&offsets)?;
        Ok(Self { offsets })
    }

    /// Decodes a table whose minimum and index coordinates are stored externally.
    ///
    /// # Errors
    ///
    /// Returns an error for truncated tables, overflowing LEB values, or block
    /// coordinates outside `data`.
    pub fn parse(
        data: &[u8],
        minimum_offset: u32,
        table_offset: u32,
        count: usize,
        endian: Endian,
    ) -> Result<Self> {
        let table_offset = usize::try_from(table_offset)
            .map_err(|_| Error::invalid_dex(0, "compact offset table position is too large"))?;
        let block_count = count.div_ceil(ELEMENTS_PER_BLOCK);
        let table_bytes = block_count
            .checked_mul(WORD_BYTES)
            .ok_or_else(|| Error::invalid_dex(table_offset, "compact offset index overflowed"))?;
        require_range(
            data,
            table_offset,
            table_bytes,
            "compact offset block index",
        )?;

        let mut offsets = Vec::with_capacity(count);
        for block_index in 0..block_count {
            let entry = table_offset + block_index * WORD_BYTES;
            let block_offset = usize::try_from(read_u32(data, entry, endian)?)
                .map_err(|_| Error::invalid_dex(entry, "compact offset block is too large"))?;
            require_range(data, block_offset, BIT_MASK_BYTES, "compact offset block")?;
            if block_offset >= table_offset {
                return Err(Error::invalid_dex(
                    block_offset,
                    "compact offset block overlaps its index table",
                ));
            }
            let mask =
                (u16::from(data[block_offset]) << u8::BITS) | u16::from(data[block_offset + 1]);
            let mut cursor = block_offset + BIT_MASK_BYTES;
            let mut previous = minimum_offset;
            let block_len = (count - offsets.len()).min(ELEMENTS_PER_BLOCK);
            for bit in 0..block_len {
                if mask & (1u16 << bit) == 0 {
                    offsets.push(0);
                } else {
                    let delta = read_uleb(data, &mut cursor, table_offset)?;
                    previous = previous.checked_add(delta).ok_or_else(|| {
                        Error::invalid_dex(cursor, "compact offset delta overflowed")
                    })?;
                    offsets.push(previous);
                }
            }
        }
        Ok(Self { offsets })
    }

    /// Decodes the self-contained variant whose first two words contain the
    /// minimum and block-index coordinates.
    ///
    /// # Errors
    ///
    /// Returns the same malformed-table errors as [`Self::parse`].
    pub fn parse_embedded(data: &[u8], count: usize, endian: Endian) -> Result<Self> {
        require_range(
            data,
            0,
            EMBEDDED_HEADER_BYTES,
            "compact offset table header",
        )?;
        let minimum = read_u32(data, 0, endian)?;
        let table_offset = read_u32(data, WORD_BYTES, endian)?;
        Self::parse(
            &data[EMBEDDED_HEADER_BYTES..],
            minimum,
            table_offset,
            count,
            endian,
        )
    }

    /// Returns all decoded offsets in native index order.
    #[must_use]
    pub fn offsets(&self) -> &[u32] {
        &self.offsets
    }

    /// Returns one native-indexed offset, or `None` when the index is outside
    /// the table.
    #[must_use]
    pub fn get(&self, index: u32) -> Option<u32> {
        usize::try_from(index)
            .ok()
            .and_then(|index| self.offsets.get(index))
            .copied()
    }

    /// Encodes the data blocks and external coordinates used by `CompactDex`.
    ///
    /// # Errors
    ///
    /// Returns an error when the table is too large for ART's 32-bit offsets.
    pub fn encode(&self, endian: Endian) -> Result<EncodedCompactOffsetTable> {
        validate_encodable(&self.offsets)?;
        let minimum_offset = self
            .offsets
            .iter()
            .copied()
            .filter(|offset| *offset != 0)
            .min()
            .unwrap_or(0);
        let mut data = Vec::new();
        let mut block_offsets = Vec::new();
        for block in self.offsets.chunks(ELEMENTS_PER_BLOCK) {
            block_offsets.push(u32::try_from(data.len()).map_err(|_| {
                Error::invalid_assembly("compact offset data exceeds 32-bit coordinates")
            })?);
            let mut mask = 0u16;
            for (bit, offset) in block.iter().enumerate() {
                if *offset != 0 {
                    mask |= 1u16 << bit;
                }
            }
            data.extend_from_slice(&mask.to_be_bytes());
            let mut previous = minimum_offset;
            for offset in block.iter().copied().filter(|offset| *offset != 0) {
                write_uleb(&mut data, offset - previous);
                previous = offset;
            }
        }
        while !data.len().is_multiple_of(WORD_BYTES) {
            data.push(0);
        }
        let table_offset = u32::try_from(data.len()).map_err(|_| {
            Error::invalid_assembly("compact offset index exceeds 32-bit coordinates")
        })?;
        for offset in block_offsets {
            data.extend_from_slice(&match endian {
                Endian::Little => offset.to_le_bytes(),
                Endian::Reverse => offset.to_be_bytes(),
            });
        }
        Ok(EncodedCompactOffsetTable {
            data,
            minimum_offset,
            table_offset,
        })
    }

    /// Encodes the self-contained table variant used by VDEX quickening data.
    ///
    /// # Errors
    ///
    /// Returns an error when table coordinates exceed 32 bits.
    pub fn encode_embedded(&self, endian: Endian) -> Result<Vec<u8>> {
        let encoded = self.encode(endian)?;
        let mut output = Vec::with_capacity(EMBEDDED_HEADER_BYTES + encoded.data.len());
        output.extend_from_slice(&match endian {
            Endian::Little => encoded.minimum_offset.to_le_bytes(),
            Endian::Reverse => encoded.minimum_offset.to_be_bytes(),
        });
        output.extend_from_slice(&match endian {
            Endian::Little => encoded.table_offset.to_le_bytes(),
            Endian::Reverse => encoded.table_offset.to_be_bytes(),
        });
        output.extend_from_slice(&encoded.data);
        Ok(output)
    }
}

fn validate_encodable(offsets: &[u32]) -> Result<()> {
    for block in offsets.chunks(ELEMENTS_PER_BLOCK) {
        let mut previous = None;
        for offset in block.iter().copied().filter(|offset| *offset != 0) {
            if previous.is_some_and(|previous| offset < previous) {
                return Err(Error::invalid_assembly(
                    "nonzero compact offsets decrease within a 16-element block",
                ));
            }
            previous = Some(offset);
        }
    }
    Ok(())
}

fn read_uleb(data: &[u8], cursor: &mut usize, limit: usize) -> Result<u32> {
    let mut value = 0u32;
    for group in 0..5u32 {
        if *cursor >= limit {
            return Err(Error::invalid_dex(
                *cursor,
                "truncated compact offset LEB128",
            ));
        }
        let byte = data[*cursor];
        *cursor += 1;
        let payload = u32::from(byte & ULEB_PAYLOAD_MASK);
        if group == 4 && payload > 0x0f {
            return Err(Error::invalid_dex(
                *cursor - 1,
                "compact offset LEB128 exceeds 32 bits",
            ));
        }
        value |= payload << (group * ULEB_GROUP_BITS);
        if byte & ULEB_CONTINUATION == 0 {
            return Ok(value);
        }
    }
    Err(Error::invalid_dex(
        *cursor,
        "compact offset LEB128 has too many bytes",
    ))
}

fn write_uleb(output: &mut Vec<u8>, mut value: u32) {
    loop {
        let mut byte = u8::try_from(value & u32::from(ULEB_PAYLOAD_MASK))
            .expect("masked LEB payload fits eight bits");
        value >>= ULEB_GROUP_BITS;
        if value != 0 {
            byte |= ULEB_CONTINUATION;
        }
        output.push(byte);
        if value == 0 {
            break;
        }
    }
}

fn read_u32(data: &[u8], offset: usize, endian: Endian) -> Result<u32> {
    let bytes: [u8; WORD_BYTES] = data
        .get(offset..offset + WORD_BYTES)
        .ok_or_else(|| Error::invalid_dex(offset, "truncated compact offset word"))?
        .try_into()
        .map_err(|_| Error::invalid_dex(offset, "truncated compact offset word"))?;
    Ok(match endian {
        Endian::Little => u32::from_le_bytes(bytes),
        Endian::Reverse => u32::from_be_bytes(bytes),
    })
}

fn require_range(data: &[u8], offset: usize, length: usize, what: &str) -> Result<()> {
    let end = offset
        .checked_add(length)
        .ok_or_else(|| Error::invalid_dex(offset, format!("{what} range overflowed")))?;
    if end <= data.len() {
        Ok(())
    } else {
        Err(Error::invalid_dex(offset, format!("truncated {what}")))
    }
}

#[cfg(test)]
mod tests {
    use super::CompactOffsetTable;
    use crate::file::Endian;

    #[test]
    fn round_trips_sparse_blocks_and_embedded_header() {
        let offsets = vec![0, 100, 104, 0, 200, 0, 0, 205, 0, 0, 0, 0, 0, 0, 0, 0, 400];
        let table = CompactOffsetTable::new(offsets.clone()).unwrap();
        let embedded = table.encode_embedded(Endian::Little).unwrap();
        let parsed =
            CompactOffsetTable::parse_embedded(&embedded, offsets.len(), Endian::Little).unwrap();
        assert_eq!(parsed.offsets(), offsets);
    }

    #[test]
    fn rejects_decreasing_offsets_in_one_block() {
        assert!(CompactOffsetTable::new(vec![20, 10]).is_err());
    }
}

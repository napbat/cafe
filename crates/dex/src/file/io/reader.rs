//! Random-access and cursor-based DEX readers.

use crate::{Error, Result};

use super::leb128;
use crate::file::Endian;
use crate::file::layout::ItemWidth;

/// Bounds-checked view of one physical DEX buffer.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Reader<'a> {
    bytes: &'a [u8],
    endian: Endian,
}

impl<'a> Reader<'a> {
    pub(crate) const fn new(bytes: &'a [u8], endian: Endian) -> Self {
        Self { bytes, endian }
    }

    pub(crate) const fn len(self) -> usize {
        self.bytes.len()
    }

    pub(crate) fn bytes(self, offset: usize, length: usize) -> Result<&'a [u8]> {
        let end = offset
            .checked_add(length)
            .ok_or_else(|| Error::invalid_dex(offset, "byte range overflowed"))?;
        self.bytes.get(offset..end).ok_or_else(|| {
            Error::invalid_dex(
                offset,
                format!("truncated byte range: needs {length} bytes"),
            )
        })
    }

    pub(crate) fn u8(self, offset: usize) -> Result<u8> {
        self.bytes
            .get(offset)
            .copied()
            .ok_or_else(|| Error::invalid_dex(offset, "truncated byte"))
    }

    pub(crate) fn u16(self, offset: usize) -> Result<u16> {
        let bytes: [u8; size_of::<u16>()] = self
            .bytes(offset, ItemWidth::CODE_UNIT.bytes())?
            .try_into()
            .map_err(|_| Error::invalid_dex(offset, "truncated 16-bit value"))?;
        Ok(match self.endian {
            Endian::Little => u16::from_le_bytes(bytes),
            Endian::Reverse => u16::from_be_bytes(bytes),
        })
    }

    pub(crate) fn u32(self, offset: usize) -> Result<u32> {
        let bytes: [u8; size_of::<u32>()] = self
            .bytes(offset, ItemWidth::WORD.bytes())?
            .try_into()
            .map_err(|_| Error::invalid_dex(offset, "truncated 32-bit value"))?;
        Ok(match self.endian {
            Endian::Little => u32::from_le_bytes(bytes),
            Endian::Reverse => u32::from_be_bytes(bytes),
        })
    }

    pub(crate) fn cursor(self, offset: usize) -> Result<Cursor<'a>> {
        if offset <= self.bytes.len() {
            Ok(Cursor {
                reader: self,
                position: offset,
            })
        } else {
            Err(Error::invalid_dex(offset, "cursor starts beyond the file"))
        }
    }
}

/// Sequential bounds-checked reader for variable-sized DEX data items.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Cursor<'a> {
    reader: Reader<'a>,
    position: usize,
}

impl<'a> Cursor<'a> {
    pub(crate) const fn position(self) -> usize {
        self.position
    }

    pub(crate) fn u8(&mut self) -> Result<u8> {
        let value = self.reader.u8(self.position)?;
        self.position = self
            .position
            .checked_add(ItemWidth::BYTE.bytes())
            .ok_or_else(|| Error::invalid_dex(self.position, "cursor overflowed"))?;
        Ok(value)
    }

    pub(crate) fn bytes(&mut self, length: usize) -> Result<&'a [u8]> {
        let bytes = self.reader.bytes(self.position, length)?;
        self.advance(length)?;
        Ok(bytes)
    }

    pub(crate) fn uleb128(&mut self) -> Result<u32> {
        leb128::read_unsigned(self)
    }

    pub(crate) fn uleb128p1(&mut self) -> Result<Option<u32>> {
        let value = self.uleb128()?;
        Ok(value.checked_sub(leb128::P1_BIAS))
    }

    pub(crate) fn sleb128(&mut self) -> Result<i32> {
        leb128::read_signed(self)
    }

    fn advance(&mut self, amount: usize) -> Result<()> {
        self.position = self
            .position
            .checked_add(amount)
            .ok_or_else(|| Error::invalid_dex(self.position, "cursor overflowed"))?;
        Ok(())
    }
}

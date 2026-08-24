//! Endian-aware output, alignment, and patching primitives.

use crate::{Error, Result};

use crate::file::Endian;
use crate::file::io::leb128::{CONTINUATION_BIT, GROUP_BITS, P1_BIAS, PAYLOAD_MASK, SIGN_BIT};
use crate::file::layout::Alignment;

/// Growable DEX output with checked 32-bit positions.
#[derive(Debug)]
pub(crate) struct Writer {
    bytes: Vec<u8>,
    endian: Endian,
}

impl Writer {
    pub(crate) fn new(endian: Endian) -> Self {
        Self {
            bytes: Vec::new(),
            endian,
        }
    }

    pub(crate) fn position(&self) -> Result<u32> {
        u32::try_from(self.bytes.len())
            .map_err(|_| Error::invalid_assembly("DEX output exceeds 32-bit address space"))
    }

    pub(crate) fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }

    pub(crate) fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub(crate) fn reserve(&mut self, length: usize) -> Result<u32> {
        let offset = self.position()?;
        let new_length = self
            .bytes
            .len()
            .checked_add(length)
            .ok_or_else(|| Error::invalid_assembly("DEX output length overflowed"))?;
        self.bytes.resize(new_length, 0);
        Ok(offset)
    }

    pub(crate) fn align(&mut self, alignment: Alignment) -> Result<()> {
        let alignment = alignment.bytes_u32();
        while !self.position()?.is_multiple_of(alignment) {
            self.u8(0);
        }
        Ok(())
    }

    pub(crate) fn u8(&mut self, value: u8) {
        self.bytes.push(value);
    }

    pub(crate) fn u16(&mut self, value: u16) {
        let bytes = match self.endian {
            Endian::Little => value.to_le_bytes(),
            Endian::Reverse => value.to_be_bytes(),
        };
        self.bytes.extend_from_slice(&bytes);
    }

    pub(crate) fn u32(&mut self, value: u32) {
        let bytes = match self.endian {
            Endian::Little => value.to_le_bytes(),
            Endian::Reverse => value.to_be_bytes(),
        };
        self.bytes.extend_from_slice(&bytes);
    }

    pub(crate) fn bytes(&mut self, value: &[u8]) {
        self.bytes.extend_from_slice(value);
    }

    pub(crate) fn patch_u16(&mut self, offset: u32, value: u16) -> Result<()> {
        let bytes = match self.endian {
            Endian::Little => value.to_le_bytes(),
            Endian::Reverse => value.to_be_bytes(),
        };
        self.patch(offset, &bytes)
    }

    pub(crate) fn patch_u32(&mut self, offset: u32, value: u32) -> Result<()> {
        let bytes = match self.endian {
            Endian::Little => value.to_le_bytes(),
            Endian::Reverse => value.to_be_bytes(),
        };
        self.patch(offset, &bytes)
    }

    pub(crate) fn patch(&mut self, offset: u32, value: &[u8]) -> Result<()> {
        let start = usize::try_from(offset)
            .map_err(|_| Error::invalid_assembly("patch offset does not fit platform"))?;
        let end = start
            .checked_add(value.len())
            .ok_or_else(|| Error::invalid_assembly("patch range overflowed"))?;
        let target = self
            .bytes
            .get_mut(start..end)
            .ok_or_else(|| Error::invalid_assembly("patch range lies outside DEX output"))?;
        target.copy_from_slice(value);
        Ok(())
    }

    pub(crate) fn uleb128(&mut self, mut value: u32) {
        loop {
            let mut byte = value.to_le_bytes()[0] & PAYLOAD_MASK;
            value >>= GROUP_BITS;
            if value != 0 {
                byte |= CONTINUATION_BIT;
            }
            self.u8(byte);
            if value == 0 {
                break;
            }
        }
    }

    pub(crate) fn uleb128p1(&mut self, value: Option<u32>) -> Result<()> {
        let encoded = value.map_or(Ok(0), |value| {
            value
                .checked_add(P1_BIAS)
                .ok_or_else(|| Error::invalid_assembly("uleb128p1 value overflowed"))
        })?;
        self.uleb128(encoded);
        Ok(())
    }

    pub(crate) fn sleb128(&mut self, mut value: i32) {
        loop {
            let byte = value.to_le_bytes()[0] & PAYLOAD_MASK;
            let sign = byte & SIGN_BIT != 0;
            value >>= GROUP_BITS;
            let done = (value == 0 && !sign) || (value == -1 && sign);
            self.u8(if done { byte } else { byte | CONTINUATION_BIT });
            if done {
                break;
            }
        }
    }
}

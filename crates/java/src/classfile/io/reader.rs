use crate::{Error, Result};

const START_POSITION: usize = 0;
const BYTE_WIDTH: usize = size_of::<u8>();

pub(crate) struct Reader<'a> {
    bytes: &'a [u8],
    position: usize,
    base_offset: usize,
}

impl<'a> Reader<'a> {
    pub(crate) const fn new(bytes: &'a [u8]) -> Self {
        Self {
            bytes,
            position: START_POSITION,
            base_offset: START_POSITION,
        }
    }

    pub(crate) const fn with_base(bytes: &'a [u8], base_offset: usize) -> Self {
        Self {
            bytes,
            position: START_POSITION,
            base_offset,
        }
    }

    pub(crate) const fn absolute_position(&self) -> usize {
        self.base_offset + self.position
    }

    pub(crate) const fn remaining(&self) -> usize {
        self.bytes.len() - self.position
    }

    pub(crate) fn read_u8(&mut self) -> Result<u8> {
        let offset = self.absolute_position();
        let value = *self
            .bytes
            .get(self.position)
            .ok_or_else(|| Error::invalid_class(offset, "unexpected end of file"))?;
        self.position += BYTE_WIDTH;
        Ok(value)
    }

    pub(crate) fn read_u16(&mut self) -> Result<u16> {
        let bytes = self.read_array::<{ size_of::<u16>() }>()?;
        Ok(u16::from_be_bytes(bytes))
    }

    pub(crate) fn read_u32(&mut self) -> Result<u32> {
        let bytes = self.read_array::<{ size_of::<u32>() }>()?;
        Ok(u32::from_be_bytes(bytes))
    }

    pub(crate) fn read_u64(&mut self) -> Result<u64> {
        let bytes = self.read_array::<{ size_of::<u64>() }>()?;
        Ok(u64::from_be_bytes(bytes))
    }

    pub(crate) fn read_bytes(&mut self, length: usize) -> Result<&'a [u8]> {
        let start = self.position;
        let end = start.checked_add(length).ok_or_else(|| {
            Error::invalid_class(self.absolute_position(), "byte range length overflow")
        })?;
        let bytes = self.bytes.get(start..end).ok_or_else(|| {
            Error::invalid_class(
                self.absolute_position(),
                format!(
                    "unexpected end of file: need {length} bytes, only {} remain",
                    self.remaining()
                ),
            )
        })?;
        self.position = end;
        Ok(bytes)
    }

    pub(crate) fn finish(&self, context: &str) -> Result<()> {
        if self.remaining() == 0 {
            Ok(())
        } else {
            Err(Error::invalid_class(
                self.absolute_position(),
                format!(
                    "{context} has {} unexpected trailing bytes",
                    self.remaining()
                ),
            ))
        }
    }

    fn read_array<const N: usize>(&mut self) -> Result<[u8; N]> {
        let bytes = self.read_bytes(N)?;
        let mut array = [0_u8; N];
        array.copy_from_slice(bytes);
        Ok(array)
    }
}

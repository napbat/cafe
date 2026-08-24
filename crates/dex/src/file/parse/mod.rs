//! Complete logical DEX parsing.

mod annotations;
mod class;
mod code;
mod debug;
mod header;
mod hidden_api;
mod map;
mod tables;
mod value;

use crate::{Error, Result};

use super::DexFile;
use super::header::{ABSENT_OFFSET, DexHeader, DexVersion, HeaderField, Section};
use super::io::Reader;
use super::layout::{
    Alignment, EMPTY_ITEM_COUNT, ItemWidth, UNLOCATED_ERROR_OFFSET, UNREPRESENTABLE_FILE_OFFSET,
};
use super::model::{MapItem, MapItemType};

pub(super) fn parse(bytes: &[u8], header_offset: usize) -> Result<DexFile> {
    let header = header::parse(bytes, header_offset)?;
    let reader = Reader::new(bytes, header.endian);
    let map = map::parse(reader, &header)?;
    let context = Context {
        reader,
        header: &header,
        map: &map,
    };

    let strings = tables::strings(&context)?;
    let types = tables::types(&context)?;
    let prototypes = tables::prototypes(&context)?;
    let fields = tables::fields(&context)?;
    let methods = tables::methods(&context)?;
    let method_handles = tables::method_handles(&context)?;
    let call_sites = tables::call_sites(&context)?;
    let classes = class::classes(&context, &fields, &methods)?;
    let hidden_api = hidden_api::data(&context, &classes)?;
    let link_data = parse_link_data(&context)?;

    validate::file(
        &header,
        &strings,
        &types,
        &prototypes,
        &fields,
        &methods,
        &classes,
        &call_sites,
        &method_handles,
    )?;

    let original_end = header_offset
        .checked_add(usize::try_from(header.file_size).map_err(|_| {
            Error::invalid_dex(
                header_offset,
                "logical file size does not fit this platform",
            )
        })?)
        .ok_or_else(|| Error::invalid_dex(header_offset, "logical file range overflowed"))?;
    let original = bytes
        .get(header_offset..original_end)
        .ok_or_else(|| Error::invalid_dex(header_offset, "logical file is truncated"))?
        .to_vec();

    Ok(DexFile {
        header,
        strings,
        types,
        prototypes,
        fields,
        methods,
        classes,
        call_sites,
        method_handles,
        map,
        link_data,
        hidden_api,
        original: Some(original),
        dirty: false,
    })
}

fn parse_link_data(context: &Context<'_>) -> Result<Vec<u8>> {
    if context.header.link_size == EMPTY_ITEM_COUNT {
        if context.header.link_off != ABSENT_OFFSET {
            return Err(Error::invalid_dex(
                HeaderField::LinkOffset.offset(),
                "link offset is nonzero for an empty link section",
            ));
        }
        return Ok(Vec::new());
    }
    let offset = context.offset(context.header.link_off, Alignment::Byte, "link data")?;
    let size = usize::try_from(context.header.link_size)
        .map_err(|_| Error::invalid_dex(offset, "link size does not fit this platform"))?;
    Ok(context.reader.bytes(offset, size)?.to_vec())
}

#[derive(Debug, Clone, Copy)]
pub(super) struct Context<'a> {
    pub(super) reader: Reader<'a>,
    pub(super) header: &'a DexHeader,
    pub(super) map: &'a [MapItem],
}

impl Context<'_> {
    pub(super) fn offset(self, value: u32, alignment: Alignment, what: &str) -> Result<usize> {
        let offset = usize::try_from(value).map_err(|_| {
            Error::invalid_dex(
                UNLOCATED_ERROR_OFFSET,
                format!("{what} offset does not fit platform"),
            )
        })?;
        if value == ABSENT_OFFSET {
            return Err(Error::invalid_dex(
                UNLOCATED_ERROR_OFFSET,
                format!("{what} offset is zero"),
            ));
        }
        if !value.is_multiple_of(alignment.bytes_u32()) {
            return Err(Error::invalid_dex(
                offset,
                format!("{what} is not aligned to {} bytes", alignment.bytes()),
            ));
        }
        if offset >= self.reader.len() {
            return Err(Error::invalid_dex(
                offset,
                format!("{what} begins beyond the container"),
            ));
        }
        if self.header.version == DexVersion::V041 && value < self.header.header_offset {
            return Err(Error::invalid_dex(
                offset,
                format!("{what} points before its version 041 header"),
            ));
        }
        Ok(offset)
    }

    pub(super) fn fixed_section(
        self,
        section: Section,
        item_width: ItemWidth,
        what: &str,
    ) -> Result<Option<(usize, usize)>> {
        if section.size == EMPTY_ITEM_COUNT {
            if section.offset != ABSENT_OFFSET {
                return Err(Error::invalid_dex(
                    usize::try_from(section.offset).unwrap_or(UNREPRESENTABLE_FILE_OFFSET),
                    format!("{what} offset is nonzero for an empty section"),
                ));
            }
            return Ok(None);
        }
        let offset = self.offset(section.offset, Alignment::Word, what)?;
        let count = self.count(section.size, item_width, offset, what)?;
        self.reader.bytes(
            offset,
            count
                .checked_mul(item_width.bytes())
                .ok_or_else(|| Error::invalid_dex(offset, format!("{what} size overflowed")))?,
        )?;
        Ok(Some((offset, count)))
    }

    pub(super) fn count(
        self,
        encoded: u32,
        minimum_width: ItemWidth,
        offset: usize,
        what: &str,
    ) -> Result<usize> {
        let count = usize::try_from(encoded).map_err(|_| {
            Error::invalid_dex(offset, format!("{what} count does not fit platform"))
        })?;
        let available = self.reader.len().saturating_sub(offset);
        if count > available / minimum_width.bytes() {
            return Err(Error::invalid_dex(
                offset,
                format!("{what} count {count} exceeds the remaining file"),
            ));
        }
        Ok(count)
    }

    pub(super) fn map_item(self, item_type: MapItemType) -> Option<MapItem> {
        self.map
            .iter()
            .find(|item| item.item_type == item_type)
            .copied()
    }

    pub(super) fn index(self, value: u32, limit: u32, offset: usize, what: &str) -> Result<u32> {
        if value < limit {
            Ok(value)
        } else {
            Err(Error::invalid_dex(
                offset,
                format!(
                    "{what} index {value} is outside 0..{limit} for DEX {}",
                    String::from_utf8_lossy(&self.header.version.digits())
                ),
            ))
        }
    }
}

mod validate {
    pub(super) use crate::file::validation::file;
}

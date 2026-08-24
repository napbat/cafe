//! Map-list parsing and top-level consistency checks.

use std::collections::BTreeSet;

use crate::{Error, Result};

use crate::file::header::{ABSENT_OFFSET, DexHeader, HeaderField};
use crate::file::io::Reader;
use crate::file::layout::{
    Alignment, EMPTY_ITEM_COUNT, ItemWidth, ListField, MapField, SINGLE_ITEM_COUNT,
    UNUSED_FIELD_VALUE,
};
use crate::file::model::{MapItem, MapItemType};

pub(super) fn parse(reader: Reader<'_>, header: &DexHeader) -> Result<Vec<MapItem>> {
    let map_offset = usize::try_from(header.map_off).map_err(|_| {
        Error::invalid_dex(
            HeaderField::MapOffset.offset(),
            "map offset does not fit platform",
        )
    })?;
    if header.map_off == ABSENT_OFFSET
        || !header.map_off.is_multiple_of(Alignment::Word.bytes_u32())
    {
        return Err(Error::invalid_dex(
            map_offset,
            "map list offset is zero or unaligned",
        ));
    }
    let count = reader.u32(map_offset + ListField::Size.offset())?;
    let count = usize::try_from(count)
        .map_err(|_| Error::invalid_dex(map_offset, "map count does not fit platform"))?;
    let byte_count = count
        .checked_mul(ItemWidth::MAP_ITEM.bytes())
        .ok_or_else(|| Error::invalid_dex(map_offset, "map size overflowed"))?;
    let entries_offset = map_offset + ListField::Entries.offset();
    reader.bytes(entries_offset, byte_count)?;

    let mut items = Vec::with_capacity(count);
    let mut type_codes = BTreeSet::new();
    let mut last_offset = None;
    for index in 0..count {
        let offset = entries_offset + index * ItemWidth::MAP_ITEM.bytes();
        let type_code = reader.u16(offset + MapField::Type.offset())?;
        if reader.u16(offset + MapField::Unused.offset())? != UNUSED_FIELD_VALUE {
            return Err(Error::invalid_dex(
                offset + MapField::Unused.offset(),
                "map unused field is nonzero",
            ));
        }
        if !type_codes.insert(type_code) {
            return Err(Error::invalid_dex(
                offset,
                format!("duplicate map item type 0x{type_code:04x}"),
            ));
        }
        let size = reader.u32(offset + MapField::Size.offset())?;
        let item_offset = reader.u32(offset + MapField::Offset.offset())?;
        if size == EMPTY_ITEM_COUNT
            || (item_offset == ABSENT_OFFSET && type_code != MapItemType::Header.as_u16())
        {
            return Err(Error::invalid_dex(
                offset + MapField::Size.offset(),
                "map entries must have nonzero size and offset",
            ));
        }
        if let Some(previous) = last_offset
            && item_offset <= previous
        {
            return Err(Error::invalid_dex(
                offset + MapField::Offset.offset(),
                "map entries are not in strictly increasing offset order",
            ));
        }
        if usize::try_from(item_offset).map_or(true, |value| value >= reader.len()) {
            return Err(Error::invalid_dex(
                offset + MapField::Offset.offset(),
                "map item begins beyond the container",
            ));
        }
        last_offset = Some(item_offset);
        items.push(MapItem {
            item_type: MapItemType::from_u16(type_code),
            size,
            offset: item_offset,
        });
    }
    validate_required(&items, header)?;
    Ok(items)
}

fn validate_required(items: &[MapItem], header: &DexHeader) -> Result<()> {
    require(
        items,
        MapItemType::Header,
        SINGLE_ITEM_COUNT,
        header.header_offset,
    )?;
    require(
        items,
        MapItemType::MapList,
        SINGLE_ITEM_COUNT,
        header.map_off,
    )?;
    require_section(items, MapItemType::StringId, header.string_ids)?;
    require_section(items, MapItemType::TypeId, header.type_ids)?;
    require_section(items, MapItemType::PrototypeId, header.proto_ids)?;
    require_section(items, MapItemType::FieldId, header.field_ids)?;
    require_section(items, MapItemType::MethodId, header.method_ids)?;
    require_section(items, MapItemType::ClassDefinition, header.class_defs)?;
    Ok(())
}

fn require(items: &[MapItem], item_type: MapItemType, size: u32, offset: u32) -> Result<()> {
    let item = items.iter().find(|item| item.item_type == item_type);
    if item.is_some_and(|item| item.size == size && item.offset == offset) {
        Ok(())
    } else {
        Err(Error::invalid_dex(
            usize::try_from(offset).unwrap_or(usize::MAX),
            format!("map entry for {item_type:?} does not match the header"),
        ))
    }
}

fn require_section(
    items: &[MapItem],
    item_type: MapItemType,
    section: crate::file::Section,
) -> Result<()> {
    if section.size == EMPTY_ITEM_COUNT {
        if items.iter().any(|item| item.item_type == item_type) {
            return Err(Error::invalid_dex(
                usize::try_from(section.offset).unwrap_or(usize::MAX),
                format!("empty header section has a {item_type:?} map entry"),
            ));
        }
        Ok(())
    } else {
        require(items, item_type, section.size, section.offset)
    }
}

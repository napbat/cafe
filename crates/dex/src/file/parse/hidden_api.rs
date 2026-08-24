//! Hidden-API member-flag section parsing.

use crate::{Error, Result};

use super::Context;
use crate::file::header::ABSENT_OFFSET;
use crate::file::layout::{
    Alignment, EMPTY_ITEM_COUNT_USIZE, HiddenApiField, ItemWidth, SINGLE_ITEM_COUNT,
    UNREPRESENTABLE_FILE_OFFSET,
};
use crate::file::model::{ClassDefinition, HiddenApiClassData, MapItemType};

pub(super) fn data(
    context: &Context<'_>,
    classes: &[ClassDefinition],
) -> Result<Option<HiddenApiClassData>> {
    let Some(item) = context.map_item(MapItemType::HiddenApiClassData) else {
        return Ok(None);
    };
    if item.size != SINGLE_ITEM_COUNT {
        return Err(Error::invalid_dex(
            usize::try_from(item.offset).unwrap_or(UNREPRESENTABLE_FILE_OFFSET),
            "hidden-API map entry must describe one section",
        ));
    }
    let offset = context.offset(item.offset, Alignment::Word, "hidden-API class data")?;
    let total_size = context.reader.u32(offset + HiddenApiField::Size.offset())?;
    let total_size = usize::try_from(total_size)
        .map_err(|_| Error::invalid_dex(offset, "hidden-API section size does not fit platform"))?;
    let section = context.reader.bytes(offset, total_size)?;
    let offsets_bytes = classes
        .len()
        .checked_mul(ItemWidth::WORD.bytes())
        .and_then(|size| size.checked_add(HiddenApiField::ClassOffsets.offset()))
        .ok_or_else(|| Error::invalid_dex(offset, "hidden-API offset table overflowed"))?;
    if total_size < offsets_bytes {
        return Err(Error::invalid_dex(
            offset,
            "hidden-API section is shorter than its class offset table",
        ));
    }

    let mut flags = Vec::with_capacity(classes.len());
    for (index, class) in classes.iter().enumerate() {
        let entry_offset =
            offset + HiddenApiField::ClassOffsets.offset() + index * ItemWidth::WORD.bytes();
        let relative = context.reader.u32(entry_offset)?;
        if relative == ABSENT_OFFSET {
            flags.push(Vec::new());
            continue;
        }
        let relative = usize::try_from(relative).map_err(|_| {
            Error::invalid_dex(entry_offset, "hidden-API offset does not fit platform")
        })?;
        if relative < offsets_bytes || relative >= section.len() {
            return Err(Error::invalid_dex(
                entry_offset,
                "hidden-API flags offset is outside the flag data",
            ));
        }
        let member_count = class
            .class_data
            .as_ref()
            .map_or(EMPTY_ITEM_COUNT_USIZE, |data| {
                data.static_fields.len()
                    + data.instance_fields.len()
                    + data.direct_methods.len()
                    + data.virtual_methods.len()
            });
        let mut cursor = context.reader.cursor(offset + relative)?;
        let mut class_flags = Vec::with_capacity(member_count);
        for _ in 0..member_count {
            let before = cursor.position();
            let flag = cursor.uleb128()?;
            if cursor.position() > offset + total_size {
                return Err(Error::invalid_dex(
                    before,
                    "hidden-API flags extend past their section",
                ));
            }
            class_flags.push(flag);
        }
        flags.push(class_flags);
    }
    Ok(Some(HiddenApiClassData {
        classes: flags,
        data_offset: item.offset,
    }))
}

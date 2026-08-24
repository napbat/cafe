//! String data, type lists, encoded arrays, and hidden-API flags.

use std::collections::BTreeMap;

use crate::file::header::ABSENT_OFFSET;
use crate::file::io::Writer;
use crate::file::layout::{
    Alignment, EMPTY_ITEM_COUNT, HiddenApiField, ItemWidth, SINGLE_ITEM_COUNT,
};
use crate::file::model::{
    ClassDefinition, DexString, HiddenApiClassData, MapItem, MapItemType, PrototypeId,
    ROOT_ENCODED_VALUE_DEPTH, TypeIndex,
};
use crate::file::mutf8::{
    CONTINUATION_PREFIX as MUTF8_CONTINUATION_PREFIX, FIVE_BIT_MASK, FOUR_BIT_MASK,
    ONE_BYTE_MAXIMUM, ONE_BYTE_MINIMUM, SIX_BIT_MASK, SIX_BIT_SHIFT,
    TERMINATOR as MUTF8_TERMINATOR, THREE_BYTE_PREFIX as MUTF8_THREE_BYTE_PREFIX, TWELVE_BIT_SHIFT,
    TWO_BYTE_PREFIX as MUTF8_TWO_BYTE_PREFIX, TWO_BYTE_VALUE_LIMIT,
};
use crate::{Error, Result};

const UNSHIFTED_FRAGMENT: u32 = 0;

pub(super) struct CoreDataLayout {
    pub(super) string_offsets: Vec<u32>,
    pub(super) prototype_parameter_offsets: Vec<u32>,
    pub(super) class_interface_offsets: Vec<u32>,
    pub(super) call_site_offsets: Vec<u32>,
    pub(super) static_value_offsets: Vec<u32>,
    pub(super) sections: Vec<MapItem>,
}

pub(super) fn write_core(
    writer: &mut Writer,
    strings: &[DexString],
    prototypes: &[PrototypeId],
    classes: &[ClassDefinition],
    call_sites: &[crate::file::CallSite],
) -> Result<CoreDataLayout> {
    let mut sections = Vec::new();
    let (string_offsets, string_section) = write_strings(writer, strings)?;
    if let Some(section) = string_section {
        sections.push(section);
    }
    let (prototype_parameter_offsets, class_interface_offsets, type_section) =
        write_type_lists(writer, prototypes, classes)?;
    if let Some(section) = type_section {
        sections.push(section);
    }
    let (call_site_offsets, static_value_offsets, array_section) =
        write_arrays(writer, call_sites, classes)?;
    if let Some(section) = array_section {
        sections.push(section);
    }
    Ok(CoreDataLayout {
        string_offsets,
        prototype_parameter_offsets,
        class_interface_offsets,
        call_site_offsets,
        static_value_offsets,
        sections,
    })
}

pub(super) fn write_hidden_api(
    writer: &mut Writer,
    hidden_api: Option<&HiddenApiClassData>,
    classes: &[ClassDefinition],
) -> Result<Option<MapItem>> {
    let Some(hidden_api) = hidden_api else {
        return Ok(None);
    };
    if hidden_api.classes.len() != classes.len() {
        return Err(Error::invalid_assembly(
            "hidden-API class count does not match class definitions",
        ));
    }
    writer.align(Alignment::Word)?;
    let start = writer.position()?;
    let size_offset = writer.reserve(ItemWidth::WORD.bytes())?;
    if size_offset != start + HiddenApiField::Size.offset_u32() {
        return Err(Error::invalid_assembly(
            "hidden-API size field violated its typed layout",
        ));
    }
    let offsets_start = writer.reserve(
        classes
            .len()
            .checked_mul(ItemWidth::WORD.bytes())
            .ok_or_else(|| Error::invalid_assembly("hidden-API offset table overflowed"))?,
    )?;
    if offsets_start != start + HiddenApiField::ClassOffsets.offset_u32() {
        return Err(Error::invalid_assembly(
            "hidden-API offset table violated its typed layout",
        ));
    }
    for (index, (flags, class)) in hidden_api.classes.iter().zip(classes).enumerate() {
        if flags.is_empty() {
            continue;
        }
        let expected = class.class_data.as_ref().map_or(0, |data| {
            data.static_fields.len()
                + data.instance_fields.len()
                + data.direct_methods.len()
                + data.virtual_methods.len()
        });
        if flags.len() != expected {
            return Err(Error::invalid_assembly(format!(
                "hidden-API class {index} has {} flags but needs {expected}",
                flags.len()
            )));
        }
        let relative = writer
            .position()?
            .checked_sub(start)
            .ok_or_else(|| Error::invalid_assembly("hidden-API offset underflowed"))?;
        let entry =
            offsets_start
                .checked_add(u32::try_from(index * ItemWidth::WORD.bytes()).map_err(|_| {
                    Error::invalid_assembly("hidden-API class offset exceeds 32 bits")
                })?)
                .ok_or_else(|| Error::invalid_assembly("hidden-API class offset overflowed"))?;
        writer.patch_u32(entry, relative)?;
        for flag in flags {
            writer.uleb128(*flag);
        }
    }
    let size = writer
        .position()?
        .checked_sub(start)
        .ok_or_else(|| Error::invalid_assembly("hidden-API size underflowed"))?;
    writer.patch_u32(size_offset, size)?;
    Ok(Some(MapItem {
        item_type: MapItemType::HiddenApiClassData,
        size: SINGLE_ITEM_COUNT,
        offset: start,
    }))
}

fn write_strings(
    writer: &mut Writer,
    strings: &[DexString],
) -> Result<(Vec<u32>, Option<MapItem>)> {
    writer.align(Alignment::Byte)?;
    let start = writer.position()?;
    let mut offsets = Vec::with_capacity(strings.len());
    for string in strings {
        offsets.push(writer.position()?);
        writer.uleb128(
            u32::try_from(string.utf16_units.len())
                .map_err(|_| Error::invalid_assembly("DEX string exceeds 32-bit UTF-16 length"))?,
        );
        for unit in &string.utf16_units {
            write_mutf8_unit(writer, *unit)?;
        }
        writer.u8(MUTF8_TERMINATOR);
    }
    let count = u32::try_from(strings.len())
        .map_err(|_| Error::invalid_assembly("string item count exceeds 32 bits"))?;
    Ok((offsets, section(MapItemType::StringData, count, start)))
}

fn write_mutf8_unit(writer: &mut Writer, unit: u16) -> Result<()> {
    if (u16::from(ONE_BYTE_MINIMUM)..=u16::from(ONE_BYTE_MAXIMUM)).contains(&unit) {
        writer.u8(u8::try_from(unit).map_err(|_| {
            Error::invalid_assembly("single-byte MUTF-8 unit exceeds its typed width")
        })?);
    } else if unit <= TWO_BYTE_VALUE_LIMIT {
        writer.u8(MUTF8_TWO_BYTE_PREFIX | fragment(unit, SIX_BIT_SHIFT, FIVE_BIT_MASK)?);
        writer.u8(MUTF8_CONTINUATION_PREFIX | fragment(unit, UNSHIFTED_FRAGMENT, SIX_BIT_MASK)?);
    } else {
        writer.u8(MUTF8_THREE_BYTE_PREFIX | fragment(unit, TWELVE_BIT_SHIFT, FOUR_BIT_MASK)?);
        writer.u8(MUTF8_CONTINUATION_PREFIX | fragment(unit, SIX_BIT_SHIFT, SIX_BIT_MASK)?);
        writer.u8(MUTF8_CONTINUATION_PREFIX | fragment(unit, UNSHIFTED_FRAGMENT, SIX_BIT_MASK)?);
    }
    Ok(())
}

fn fragment(unit: u16, shift: u32, mask: u8) -> Result<u8> {
    u8::try_from((unit >> shift) & u16::from(mask))
        .map_err(|_| Error::invalid_assembly("MUTF-8 fragment exceeds one byte"))
}

fn write_type_lists(
    writer: &mut Writer,
    prototypes: &[PrototypeId],
    classes: &[ClassDefinition],
) -> Result<(Vec<u32>, Vec<u32>, Option<MapItem>)> {
    writer.align(Alignment::Word)?;
    let start = writer.position()?;
    let mut known = BTreeMap::<Vec<TypeIndex>, u32>::new();
    let mut count = 0u32;
    let mut prototype_offsets = Vec::with_capacity(prototypes.len());
    for prototype in prototypes {
        prototype_offsets.push(type_list(
            writer,
            &prototype.parameters,
            &mut known,
            &mut count,
        )?);
    }
    let mut class_offsets = Vec::with_capacity(classes.len());
    for class in classes {
        class_offsets.push(type_list(
            writer,
            &class.interfaces,
            &mut known,
            &mut count,
        )?);
    }
    Ok((
        prototype_offsets,
        class_offsets,
        section(MapItemType::TypeList, count, start),
    ))
}

fn type_list(
    writer: &mut Writer,
    types: &[TypeIndex],
    known: &mut BTreeMap<Vec<TypeIndex>, u32>,
    count: &mut u32,
) -> Result<u32> {
    if types.is_empty() {
        return Ok(ABSENT_OFFSET);
    }
    if let Some(offset) = known.get(types) {
        return Ok(*offset);
    }
    writer.align(Alignment::Word)?;
    let offset = writer.position()?;
    writer
        .u32(u32::try_from(types.len()).map_err(|_| {
            Error::invalid_assembly("type-list count exceeds 32-bit address space")
        })?);
    for item in types {
        writer.u16(u16::try_from(item.get()).map_err(|_| {
            Error::invalid_assembly("type-list index exceeds the 16-bit DEX limit")
        })?);
    }
    *count = count
        .checked_add(1)
        .ok_or_else(|| Error::invalid_assembly("type-list item count overflowed"))?;
    known.insert(types.to_vec(), offset);
    Ok(offset)
}

fn write_arrays(
    writer: &mut Writer,
    call_sites: &[crate::file::CallSite],
    classes: &[ClassDefinition],
) -> Result<(Vec<u32>, Vec<u32>, Option<MapItem>)> {
    let start = writer.position()?;
    let mut count = 0u32;
    let mut call_site_offsets = Vec::with_capacity(call_sites.len());
    for call_site in call_sites {
        call_site_offsets.push(writer.position()?);
        super::value::array(writer, &call_site.values, ROOT_ENCODED_VALUE_DEPTH)?;
        count = count
            .checked_add(1)
            .ok_or_else(|| Error::invalid_assembly("encoded-array item count overflowed"))?;
    }
    let mut static_offsets = Vec::with_capacity(classes.len());
    for class in classes {
        if class.static_values.is_empty() {
            static_offsets.push(ABSENT_OFFSET);
        } else {
            static_offsets.push(writer.position()?);
            super::value::array(writer, &class.static_values, ROOT_ENCODED_VALUE_DEPTH)?;
            count = count
                .checked_add(1)
                .ok_or_else(|| Error::invalid_assembly("encoded-array item count overflowed"))?;
        }
    }
    Ok((
        call_site_offsets,
        static_offsets,
        section(MapItemType::EncodedArray, count, start),
    ))
}

fn section(item_type: MapItemType, count: u32, offset: u32) -> Option<MapItem> {
    (count != EMPTY_ITEM_COUNT).then_some(MapItem {
        item_type,
        size: count,
        offset,
    })
}

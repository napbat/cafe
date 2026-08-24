//! Canonical DEX assembly and integrity generation.

mod annotations;
mod code;
mod data;
mod layout;
mod value;

use crate::file::header::{
    ABSENT_OFFSET, ENDIAN_CONSTANT, HeaderField, LEGACY_HEADER_OFFSET, MAGIC_PREFIX, MAGIC_SIZE,
    MAGIC_TERMINATOR, MAGIC_TERMINATOR_INDEX, MAGIC_VERSION_SIZE, SECTION_OFFSET_DELTA_U32,
};
use crate::file::integrity;
use crate::file::io::Writer;
use crate::file::layout::{Alignment, ItemWidth, ListField, SINGLE_ITEM_COUNT, UNUSED_FIELD_VALUE};
use crate::file::model::{MapItem, MapItemType};
use crate::file::{DexFile, DexVersion};
use crate::{Error, Result};

const VERSION_START: usize = MAGIC_PREFIX.len();
const VERSION_END: usize = VERSION_START + MAGIC_VERSION_SIZE;

pub(super) fn assemble(file: &DexFile) -> Result<Vec<u8>> {
    if file.header.version == DexVersion::V041 {
        return Err(Error::invalid_assembly(
            "edited version 041 members must be assembled through a DEX container",
        ));
    }
    assemble_at(file, LEGACY_HEADER_OFFSET, None)
}

/// Assembles one version 041 member with container-relative offsets.
pub(super) fn assemble_member(
    file: &DexFile,
    header_offset: u32,
    container_size: u32,
) -> Result<Vec<u8>> {
    if file.header.version != DexVersion::V041 {
        return Err(Error::invalid_assembly(
            "a DEX container member must use version 041",
        ));
    }
    assemble_at(
        file,
        usize::try_from(header_offset).map_err(|_| {
            Error::invalid_assembly("DEX container header offset does not fit this platform")
        })?,
        Some(container_size),
    )
}

fn assemble_at(
    file: &DexFile,
    header_offset: usize,
    container_size: Option<u32>,
) -> Result<Vec<u8>> {
    if file
        .map
        .iter()
        .any(|item| matches!(item.item_type, MapItemType::Unknown(_)))
    {
        return Err(Error::invalid_assembly(
            "cannot relocate an unknown DEX section without its format contract",
        ));
    }
    validate_model(file)?;

    let header_offset = u32::try_from(header_offset)
        .map_err(|_| Error::invalid_assembly("DEX header offset exceeds 32 bits"))?;
    let mut writer = if header_offset == 0 {
        Writer::new(file.header.endian)
    } else {
        Writer::new_at(file.header.endian, header_offset)
    };
    writer.reserve(
        usize::try_from(file.header.version.header_size())
            .map_err(|_| Error::invalid_assembly("DEX header size does not fit this platform"))?,
    )?;
    let tables = layout::write(&mut writer, file)?;

    let (link_offset, link_size) = write_link_data(&mut writer, &file.link_data)?;
    writer.align(Alignment::Word)?;
    let data_offset = writer.position()?;

    let core_data = data::write_core(
        &mut writer,
        &file.strings,
        &file.prototypes,
        &file.classes,
        &file.call_sites,
    )?;
    let annotations = annotations::write(&mut writer, &file.classes)?;
    let executable_code = code::write(&mut writer, &file.classes)?;
    let hidden_api = data::write_hidden_api(&mut writer, file.hidden_api.as_ref(), &file.classes)?;
    tables.patch_data_offsets(
        &mut writer,
        file,
        &core_data,
        &annotations,
        &executable_code,
    )?;

    let mut sections = tables.sections;
    sections.extend(core_data.sections);
    sections.extend(annotations.sections);
    sections.extend(executable_code.sections);
    if let Some(hidden_api) = hidden_api {
        sections.push(hidden_api);
    }
    let map_offset = write_map(&mut writer, &mut sections)?;
    let logical_end = writer.position()?;
    let file_size = writer.local_position()?;
    let data_size = logical_end
        .checked_sub(data_offset)
        .ok_or_else(|| Error::invalid_assembly("DEX data size underflowed"))?;
    patch_header(
        &mut writer,
        file,
        &sections,
        file_size,
        link_size,
        link_offset,
        map_offset,
        data_size,
        data_offset,
        container_size,
    )?;
    patch_integrity(&mut writer)?;
    let bytes = writer.into_bytes();
    if container_size.is_none() {
        super::parse::parse(&bytes, LEGACY_HEADER_OFFSET).map_err(|error| {
            Error::invalid_assembly(format!("canonical output failed self-validation: {error}"))
        })?;
    }
    Ok(bytes)
}

fn validate_model(file: &DexFile) -> Result<()> {
    let mut header = file.header.clone();
    header.string_ids.size = count(file.strings.len(), "string identifiers")?;
    header.type_ids.size = count(file.types.len(), "type identifiers")?;
    header.proto_ids.size = count(file.prototypes.len(), "prototype identifiers")?;
    header.field_ids.size = count(file.fields.len(), "field identifiers")?;
    header.method_ids.size = count(file.methods.len(), "method identifiers")?;
    header.class_defs.size = count(file.classes.len(), "class definitions")?;
    super::validation::file(
        &header,
        &file.strings,
        &file.types,
        &file.prototypes,
        &file.fields,
        &file.methods,
        &file.classes,
        &file.call_sites,
        &file.method_handles,
    )
    .map_err(|error| Error::invalid_assembly(format!("invalid edited DEX model: {error}")))
}

fn write_link_data(writer: &mut Writer, link_data: &[u8]) -> Result<(u32, u32)> {
    if link_data.is_empty() {
        return Ok((ABSENT_OFFSET, ABSENT_OFFSET));
    }
    writer.align(Alignment::Word)?;
    let offset = writer.position()?;
    writer.bytes(link_data);
    Ok((
        offset,
        u32::try_from(link_data.len())
            .map_err(|_| Error::invalid_assembly("link data exceeds 32-bit size"))?,
    ))
}

fn write_map(writer: &mut Writer, sections: &mut Vec<MapItem>) -> Result<u32> {
    writer.align(Alignment::Word)?;
    let map_offset = writer.position()?;
    sections.push(MapItem {
        item_type: MapItemType::MapList,
        size: SINGLE_ITEM_COUNT,
        offset: map_offset,
    });
    sections.sort_by_key(|section| section.offset);
    for pair in sections.windows(2) {
        if pair[0].offset >= pair[1].offset {
            return Err(Error::invalid_assembly(
                "canonical map sections do not have unique increasing offsets",
            ));
        }
    }
    writer.u32(count(sections.len(), "map entries")?);
    let expected_bytes = sections
        .len()
        .checked_mul(ItemWidth::MAP_ITEM.bytes())
        .and_then(|size| size.checked_add(ListField::Entries.offset()))
        .ok_or_else(|| Error::invalid_assembly("map list size overflowed"))?;
    for section in sections.iter() {
        writer.u16(section.item_type.as_u16());
        writer.u16(UNUSED_FIELD_VALUE);
        writer.u32(section.size);
        writer.u32(section.offset);
    }
    let actual_bytes = writer
        .position()?
        .checked_sub(map_offset)
        .ok_or_else(|| Error::invalid_assembly("map size underflowed"))?;
    if actual_bytes
        != u32::try_from(expected_bytes)
            .map_err(|_| Error::invalid_assembly("map list exceeds 32-bit size"))?
    {
        return Err(Error::invalid_assembly(
            "map writer violated its typed item width",
        ));
    }
    Ok(map_offset)
}

#[allow(clippy::too_many_arguments)]
fn patch_header(
    writer: &mut Writer,
    file: &DexFile,
    sections: &[MapItem],
    file_size: u32,
    link_size: u32,
    link_offset: u32,
    map_offset: u32,
    data_size: u32,
    data_offset: u32,
    container_size: Option<u32>,
) -> Result<()> {
    let mut magic = [MAGIC_TERMINATOR; MAGIC_SIZE];
    magic[..VERSION_START].copy_from_slice(MAGIC_PREFIX);
    magic[VERSION_START..VERSION_END].copy_from_slice(&file.header.version.digits());
    magic[MAGIC_TERMINATOR_INDEX] = MAGIC_TERMINATOR;
    writer.patch(field(writer, HeaderField::Magic)?, &magic)?;
    writer.patch_u32(field(writer, HeaderField::FileSize)?, file_size)?;
    writer.patch_u32(
        field(writer, HeaderField::HeaderSize)?,
        file.header.version.header_size(),
    )?;
    writer.patch_u32(field(writer, HeaderField::EndianTag)?, ENDIAN_CONSTANT)?;
    writer.patch_u32(field(writer, HeaderField::LinkSize)?, link_size)?;
    writer.patch_u32(field(writer, HeaderField::LinkOffset)?, link_offset)?;
    writer.patch_u32(field(writer, HeaderField::MapOffset)?, map_offset)?;
    patch_section(
        writer,
        HeaderField::StringIds,
        sections,
        MapItemType::StringId,
    )?;
    patch_section(writer, HeaderField::TypeIds, sections, MapItemType::TypeId)?;
    patch_section(
        writer,
        HeaderField::PrototypeIds,
        sections,
        MapItemType::PrototypeId,
    )?;
    patch_section(
        writer,
        HeaderField::FieldIds,
        sections,
        MapItemType::FieldId,
    )?;
    patch_section(
        writer,
        HeaderField::MethodIds,
        sections,
        MapItemType::MethodId,
    )?;
    patch_section(
        writer,
        HeaderField::ClassDefinitions,
        sections,
        MapItemType::ClassDefinition,
    )?;
    let (data_size, data_offset) = if container_size.is_some() {
        (ABSENT_OFFSET, ABSENT_OFFSET)
    } else {
        (data_size, data_offset)
    };
    writer.patch_u32(field(writer, HeaderField::Data)?, data_size)?;
    writer.patch_u32(
        field(writer, HeaderField::Data)?
            .checked_add(SECTION_OFFSET_DELTA_U32)
            .ok_or_else(|| Error::invalid_assembly("data-offset header field overflowed"))?,
        data_offset,
    )?;
    if let Some(container_size) = container_size {
        writer.patch_u32(field(writer, HeaderField::ContainerSize)?, container_size)?;
        writer.patch_u32(field(writer, HeaderField::HeaderOffset)?, writer.base())?;
    }
    Ok(())
}

fn patch_section(
    writer: &mut Writer,
    field_name: HeaderField,
    sections: &[MapItem],
    item_type: MapItemType,
) -> Result<()> {
    let section = sections
        .iter()
        .find(|section| section.item_type == item_type);
    let (size, offset) = section.map_or((ABSENT_OFFSET, ABSENT_OFFSET), |section| {
        (section.size, section.offset)
    });
    let base = field(writer, field_name)?;
    writer.patch_u32(base, size)?;
    writer.patch_u32(
        base.checked_add(SECTION_OFFSET_DELTA_U32)
            .ok_or_else(|| Error::invalid_assembly("header section offset field overflowed"))?,
        offset,
    )
}

fn patch_integrity(writer: &mut Writer) -> Result<()> {
    let signature_start = HeaderField::FileSize.offset();
    let signature =
        integrity::signature(writer.as_bytes().get(signature_start..).ok_or_else(|| {
            Error::invalid_assembly("DEX output is shorter than its signature range")
        })?);
    writer.patch(field(writer, HeaderField::Signature)?, &signature)?;
    let checksum_start = HeaderField::Signature.offset();
    let checksum =
        integrity::adler32(writer.as_bytes().get(checksum_start..).ok_or_else(|| {
            Error::invalid_assembly("DEX output is shorter than its checksum range")
        })?);
    writer.patch_u32(field(writer, HeaderField::Checksum)?, checksum)
}

fn field(writer: &Writer, field: HeaderField) -> Result<u32> {
    writer
        .base()
        .checked_add(
            u32::try_from(field.offset())
                .map_err(|_| Error::invalid_assembly("header field offset exceeds 32 bits"))?,
        )
        .ok_or_else(|| Error::invalid_assembly("header field offset overflowed"))
}

fn count(value: usize, what: &str) -> Result<u32> {
    u32::try_from(value)
        .map_err(|_| Error::invalid_assembly(format!("{what} count exceeds 32 bits")))
}

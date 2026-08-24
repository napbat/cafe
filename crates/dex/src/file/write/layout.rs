//! Fixed identifier tables and their deferred data offsets.

use crate::file::DexFile;
use crate::file::header::NO_INDEX;
use crate::file::io::Writer;
use crate::file::layout::{
    Alignment, ClassField, EMPTY_ITEM_COUNT_USIZE, ItemWidth, PrototypeField, SINGLE_ITEM_COUNT,
    UNUSED_FIELD_VALUE,
};
use crate::file::model::{MapItem, MapItemType, StringIndex, TypeIndex};
use crate::{Error, Result};

pub(super) struct TableLayout {
    pub(super) string_ids: u32,
    pub(super) prototype_ids: u32,
    pub(super) class_definitions: u32,
    pub(super) call_site_ids: u32,
    pub(super) sections: Vec<MapItem>,
}

pub(super) fn write(writer: &mut Writer, file: &DexFile) -> Result<TableLayout> {
    let mut sections = vec![MapItem {
        item_type: MapItemType::Header,
        size: SINGLE_ITEM_COUNT,
        offset: writer.base(),
    }];
    let string_ids = reserve_table(
        writer,
        MapItemType::StringId,
        file.strings.len(),
        ItemWidth::STRING_ID.bytes(),
        &mut sections,
    )?;
    let type_ids = table_start(writer, MapItemType::TypeId, file.types.len(), &mut sections)?;
    for entry in &file.types {
        writer.u32(entry.descriptor.get());
    }
    debug_assert_table_width(writer, type_ids, file.types.len(), ItemWidth::TYPE_ID)?;

    let prototype_ids = reserve_table(
        writer,
        MapItemType::PrototypeId,
        file.prototypes.len(),
        ItemWidth::PROTOTYPE_ID.bytes(),
        &mut sections,
    )?;
    for (index, prototype) in file.prototypes.iter().enumerate() {
        let offset = item_offset(prototype_ids, index, ItemWidth::PROTOTYPE_ID)?;
        writer.patch_u32(
            offset + PrototypeField::Shorty.offset_u32(),
            prototype.shorty.get(),
        )?;
        writer.patch_u32(
            offset + PrototypeField::ReturnType.offset_u32(),
            prototype.return_type.get(),
        )?;
    }

    let field_ids = table_start(
        writer,
        MapItemType::FieldId,
        file.fields.len(),
        &mut sections,
    )?;
    for field in &file.fields {
        writer.u16(u16_index(field.class.get(), "field class")?);
        writer.u16(u16_index(field.field_type.get(), "field type")?);
        writer.u32(field.name.get());
    }
    debug_assert_table_width(writer, field_ids, file.fields.len(), ItemWidth::FIELD_ID)?;

    let method_ids = table_start(
        writer,
        MapItemType::MethodId,
        file.methods.len(),
        &mut sections,
    )?;
    for method in &file.methods {
        writer.u16(u16_index(method.class.get(), "method class")?);
        writer.u16(u16_index(method.prototype.get(), "method prototype")?);
        writer.u32(method.name.get());
    }
    debug_assert_table_width(writer, method_ids, file.methods.len(), ItemWidth::METHOD_ID)?;

    let class_definitions = reserve_table(
        writer,
        MapItemType::ClassDefinition,
        file.classes.len(),
        ItemWidth::CLASS_DEFINITION.bytes(),
        &mut sections,
    )?;
    let call_site_ids = reserve_table(
        writer,
        MapItemType::CallSiteId,
        file.call_sites.len(),
        ItemWidth::CALL_SITE_ID.bytes(),
        &mut sections,
    )?;

    let method_handles = table_start(
        writer,
        MapItemType::MethodHandle,
        file.method_handles.len(),
        &mut sections,
    )?;
    for handle in &file.method_handles {
        writer.u16(handle.kind.as_u16());
        writer.u16(UNUSED_FIELD_VALUE);
        writer.u16(handle.target_index);
        writer.u16(UNUSED_FIELD_VALUE);
    }
    debug_assert_table_width(
        writer,
        method_handles,
        file.method_handles.len(),
        ItemWidth::METHOD_HANDLE,
    )?;

    Ok(TableLayout {
        string_ids,
        prototype_ids,
        class_definitions,
        call_site_ids,
        sections,
    })
}

impl TableLayout {
    pub(super) fn patch_data_offsets(
        &self,
        writer: &mut Writer,
        file: &DexFile,
        core_data: &super::data::CoreDataLayout,
        annotations: &super::annotations::AnnotationLayout,
        executable_code: &super::code::CodeLayout,
    ) -> Result<()> {
        require_count(
            file.strings.len(),
            core_data.string_offsets.len(),
            "string offsets",
        )?;
        for (index, offset) in core_data.string_offsets.iter().enumerate() {
            writer.patch_u32(
                item_offset(self.string_ids, index, ItemWidth::STRING_ID)?,
                *offset,
            )?;
        }
        require_count(
            file.prototypes.len(),
            core_data.prototype_parameter_offsets.len(),
            "prototype parameter offsets",
        )?;
        for (index, offset) in core_data.prototype_parameter_offsets.iter().enumerate() {
            writer.patch_u32(
                item_offset(self.prototype_ids, index, ItemWidth::PROTOTYPE_ID)?
                    + PrototypeField::ParametersOffset.offset_u32(),
                *offset,
            )?;
        }
        require_count(
            file.classes.len(),
            core_data.class_interface_offsets.len(),
            "class interfaces",
        )?;
        require_count(
            file.classes.len(),
            core_data.static_value_offsets.len(),
            "static values",
        )?;
        require_count(
            file.classes.len(),
            annotations.directory_offsets.len(),
            "annotation directories",
        )?;
        require_count(
            file.classes.len(),
            executable_code.class_data_offsets.len(),
            "class data offsets",
        )?;
        for (index, class) in file.classes.iter().enumerate() {
            let offset = item_offset(self.class_definitions, index, ItemWidth::CLASS_DEFINITION)?;
            writer.patch_u32(offset + ClassField::Class.offset_u32(), class.class.get())?;
            writer.patch_u32(
                offset + ClassField::AccessFlags.offset_u32(),
                class.access_flags.bits(),
            )?;
            writer.patch_u32(
                offset + ClassField::Superclass.offset_u32(),
                class.superclass.map_or(NO_INDEX, TypeIndex::get),
            )?;
            writer.patch_u32(
                offset + ClassField::InterfacesOffset.offset_u32(),
                core_data.class_interface_offsets[index],
            )?;
            writer.patch_u32(
                offset + ClassField::SourceFile.offset_u32(),
                class.source_file.map_or(NO_INDEX, StringIndex::get),
            )?;
            writer.patch_u32(
                offset + ClassField::AnnotationsOffset.offset_u32(),
                annotations.directory_offsets[index],
            )?;
            writer.patch_u32(
                offset + ClassField::ClassDataOffset.offset_u32(),
                executable_code.class_data_offsets[index],
            )?;
            writer.patch_u32(
                offset + ClassField::StaticValuesOffset.offset_u32(),
                core_data.static_value_offsets[index],
            )?;
        }
        require_count(
            file.call_sites.len(),
            core_data.call_site_offsets.len(),
            "call-site offsets",
        )?;
        for (index, offset) in core_data.call_site_offsets.iter().enumerate() {
            writer.patch_u32(
                item_offset(self.call_site_ids, index, ItemWidth::CALL_SITE_ID)?,
                *offset,
            )?;
        }
        Ok(())
    }
}

fn reserve_table(
    writer: &mut Writer,
    item_type: MapItemType,
    count: usize,
    width: usize,
    sections: &mut Vec<MapItem>,
) -> Result<u32> {
    let offset = table_start(writer, item_type, count, sections)?;
    writer.reserve(count.checked_mul(width).ok_or_else(|| {
        Error::invalid_assembly(format!("{item_type:?} table size overflowed"))
    })?)?;
    Ok(offset)
}

fn table_start(
    writer: &mut Writer,
    item_type: MapItemType,
    count: usize,
    sections: &mut Vec<MapItem>,
) -> Result<u32> {
    if count == EMPTY_ITEM_COUNT_USIZE {
        return Ok(crate::file::header::ABSENT_OFFSET);
    }
    writer.align(Alignment::Word)?;
    let offset = writer.position()?;
    sections.push(MapItem {
        item_type,
        size: u32::try_from(count)
            .map_err(|_| Error::invalid_assembly(format!("{item_type:?} count exceeds 32 bits")))?,
        offset,
    });
    Ok(offset)
}

fn debug_assert_table_width(
    writer: &Writer,
    start: u32,
    count: usize,
    width: ItemWidth,
) -> Result<()> {
    if count == EMPTY_ITEM_COUNT_USIZE {
        return Ok(());
    }
    let expected = u32::try_from(
        count
            .checked_mul(width.bytes())
            .ok_or_else(|| Error::invalid_assembly("fixed identifier table size overflowed"))?,
    )
    .map_err(|_| Error::invalid_assembly("fixed identifier table exceeds 32 bits"))?;
    if writer.position()?.checked_sub(start) == Some(expected) {
        Ok(())
    } else {
        Err(Error::invalid_assembly(
            "fixed identifier table writer used an incorrect typed width",
        ))
    }
}

fn item_offset(start: u32, index: usize, width: ItemWidth) -> Result<u32> {
    let delta = u32::try_from(
        index
            .checked_mul(width.bytes())
            .ok_or_else(|| Error::invalid_assembly("identifier item offset overflowed"))?,
    )
    .map_err(|_| Error::invalid_assembly("identifier item offset exceeds 32 bits"))?;
    start
        .checked_add(delta)
        .ok_or_else(|| Error::invalid_assembly("identifier item address overflowed"))
}

fn u16_index(index: u32, what: &str) -> Result<u16> {
    u16::try_from(index).map_err(|_| {
        Error::invalid_assembly(format!("{what} index {index} exceeds the 16-bit DEX limit"))
    })
}

fn require_count(expected: usize, actual: usize, what: &str) -> Result<()> {
    if expected == actual {
        Ok(())
    } else {
        Err(Error::invalid_assembly(format!(
            "{what} layout has {actual} entries; expected {expected}"
        )))
    }
}

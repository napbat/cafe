//! Identifier-table and referenced type-list parsing.

use crate::{Error, Result};

use super::{Context, value};
use crate::file::header::ABSENT_OFFSET;
use crate::file::layout::{
    Alignment, FieldIdField, ItemWidth, ListField, MethodHandleField, MethodIdField,
    PrototypeField, UNUSED_FIELD_VALUE,
};
use crate::file::model::{
    CallSite, DexString, FieldId, MapItemType, MethodHandle, MethodHandleKind, MethodId,
    PrototypeId, StringIndex, TypeId, TypeIndex,
};
use crate::file::mutf8;

pub(super) fn strings(context: &Context<'_>) -> Result<Vec<DexString>> {
    let Some((offset, count)) = context.fixed_section(
        context.header.string_ids,
        ItemWidth::STRING_ID,
        "string identifiers",
    )?
    else {
        return Ok(Vec::new());
    };
    let mut strings = Vec::with_capacity(count);
    for index in 0..count {
        let item_offset = offset + index * ItemWidth::STRING_ID.bytes();
        let data_offset = context.reader.u32(item_offset)?;
        let data = context.offset(data_offset, Alignment::Byte, "string data")?;
        let mut cursor = context.reader.cursor(data)?;
        let utf16_size = cursor.uleb128()?;
        let utf16_units = mutf8::decode(&mut cursor, utf16_size)?;
        strings.push(DexString {
            text: String::from_utf16_lossy(&utf16_units),
            utf16_units,
            data_offset: Some(data_offset),
        });
    }
    Ok(strings)
}

pub(super) fn types(context: &Context<'_>) -> Result<Vec<TypeId>> {
    let Some((offset, count)) = context.fixed_section(
        context.header.type_ids,
        ItemWidth::TYPE_ID,
        "type identifiers",
    )?
    else {
        return Ok(Vec::new());
    };
    let mut types = Vec::with_capacity(count);
    for index in 0..count {
        let item_offset = offset + index * ItemWidth::TYPE_ID.bytes();
        let descriptor = context.reader.u32(item_offset)?;
        context.index(
            descriptor,
            context.header.string_ids.size,
            item_offset,
            "type descriptor string",
        )?;
        types.push(TypeId {
            descriptor: StringIndex(descriptor),
        });
    }
    Ok(types)
}

pub(super) fn prototypes(context: &Context<'_>) -> Result<Vec<PrototypeId>> {
    let Some((offset, count)) = context.fixed_section(
        context.header.proto_ids,
        ItemWidth::PROTOTYPE_ID,
        "prototype identifiers",
    )?
    else {
        return Ok(Vec::new());
    };
    let mut prototypes = Vec::with_capacity(count);
    for index in 0..count {
        let item_offset = offset + index * ItemWidth::PROTOTYPE_ID.bytes();
        let shorty = context
            .reader
            .u32(item_offset + PrototypeField::Shorty.offset())?;
        let return_type = context
            .reader
            .u32(item_offset + PrototypeField::ReturnType.offset())?;
        let parameters_offset = context
            .reader
            .u32(item_offset + PrototypeField::ParametersOffset.offset())?;
        context.index(
            shorty,
            context.header.string_ids.size,
            item_offset,
            "prototype shorty string",
        )?;
        context.index(
            return_type,
            context.header.type_ids.size,
            item_offset + PrototypeField::ReturnType.offset(),
            "prototype return type",
        )?;
        let parameters = type_list(context, parameters_offset, "prototype parameters")?;
        prototypes.push(PrototypeId {
            shorty: StringIndex(shorty),
            return_type: TypeIndex(return_type),
            parameters,
            parameters_offset,
        });
    }
    Ok(prototypes)
}

pub(super) fn fields(context: &Context<'_>) -> Result<Vec<FieldId>> {
    let Some((offset, count)) = context.fixed_section(
        context.header.field_ids,
        ItemWidth::FIELD_ID,
        "field identifiers",
    )?
    else {
        return Ok(Vec::new());
    };
    let mut fields = Vec::with_capacity(count);
    for index in 0..count {
        let item_offset = offset + index * ItemWidth::FIELD_ID.bytes();
        let class = u32::from(
            context
                .reader
                .u16(item_offset + FieldIdField::Class.offset())?,
        );
        let field_type = u32::from(
            context
                .reader
                .u16(item_offset + FieldIdField::Type.offset())?,
        );
        let name = context
            .reader
            .u32(item_offset + FieldIdField::Name.offset())?;
        context.index(
            class,
            context.header.type_ids.size,
            item_offset,
            "field class",
        )?;
        context.index(
            field_type,
            context.header.type_ids.size,
            item_offset + FieldIdField::Type.offset(),
            "field type",
        )?;
        context.index(
            name,
            context.header.string_ids.size,
            item_offset + FieldIdField::Name.offset(),
            "field name",
        )?;
        fields.push(FieldId {
            class: TypeIndex(class),
            field_type: TypeIndex(field_type),
            name: StringIndex(name),
        });
    }
    Ok(fields)
}

pub(super) fn methods(context: &Context<'_>) -> Result<Vec<MethodId>> {
    let Some((offset, count)) = context.fixed_section(
        context.header.method_ids,
        ItemWidth::METHOD_ID,
        "method identifiers",
    )?
    else {
        return Ok(Vec::new());
    };
    let mut methods = Vec::with_capacity(count);
    for index in 0..count {
        let item_offset = offset + index * ItemWidth::METHOD_ID.bytes();
        let class = u32::from(
            context
                .reader
                .u16(item_offset + MethodIdField::Class.offset())?,
        );
        let prototype = u32::from(
            context
                .reader
                .u16(item_offset + MethodIdField::Prototype.offset())?,
        );
        let name = context
            .reader
            .u32(item_offset + MethodIdField::Name.offset())?;
        context.index(
            class,
            context.header.type_ids.size,
            item_offset,
            "method class",
        )?;
        context.index(
            prototype,
            context.header.proto_ids.size,
            item_offset + MethodIdField::Prototype.offset(),
            "method prototype",
        )?;
        context.index(
            name,
            context.header.string_ids.size,
            item_offset + MethodIdField::Name.offset(),
            "method name",
        )?;
        methods.push(MethodId {
            class: TypeIndex(class),
            prototype: crate::file::PrototypeIndex(prototype),
            name: StringIndex(name),
        });
    }
    Ok(methods)
}

pub(super) fn method_handles(context: &Context<'_>) -> Result<Vec<MethodHandle>> {
    let Some(item) = context.map_item(MapItemType::MethodHandle) else {
        return Ok(Vec::new());
    };
    let section = crate::file::Section {
        size: item.size,
        offset: item.offset,
    };
    let Some((offset, count)) =
        context.fixed_section(section, ItemWidth::METHOD_HANDLE, "method handles")?
    else {
        return Ok(Vec::new());
    };
    let mut handles = Vec::with_capacity(count);
    for index in 0..count {
        let item_offset = offset + index * ItemWidth::METHOD_HANDLE.bytes();
        let kind_value = context
            .reader
            .u16(item_offset + MethodHandleField::Kind.offset())?;
        let kind = MethodHandleKind::from_u16(kind_value).ok_or_else(|| {
            Error::invalid_dex(
                item_offset,
                format!("unknown method-handle kind {kind_value}"),
            )
        })?;
        if context
            .reader
            .u16(item_offset + MethodHandleField::FirstUnused.offset())?
            != UNUSED_FIELD_VALUE
            || context
                .reader
                .u16(item_offset + MethodHandleField::SecondUnused.offset())?
                != UNUSED_FIELD_VALUE
        {
            return Err(Error::invalid_dex(
                item_offset,
                "method-handle unused field is nonzero",
            ));
        }
        let target_index = context
            .reader
            .u16(item_offset + MethodHandleField::Target.offset())?;
        let limit = if kind.references_field() {
            context.header.field_ids.size
        } else {
            context.header.method_ids.size
        };
        context.index(
            u32::from(target_index),
            limit,
            item_offset + MethodHandleField::Target.offset(),
            "method-handle target",
        )?;
        handles.push(MethodHandle { kind, target_index });
    }
    Ok(handles)
}

pub(super) fn call_sites(context: &Context<'_>) -> Result<Vec<CallSite>> {
    let Some(item) = context.map_item(MapItemType::CallSiteId) else {
        return Ok(Vec::new());
    };
    let section = crate::file::Section {
        size: item.size,
        offset: item.offset,
    };
    let Some((offset, count)) =
        context.fixed_section(section, ItemWidth::CALL_SITE_ID, "call-site identifiers")?
    else {
        return Ok(Vec::new());
    };
    let mut sites = Vec::with_capacity(count);
    for index in 0..count {
        let item_offset = offset + index * ItemWidth::CALL_SITE_ID.bytes();
        let data_offset = context.reader.u32(item_offset)?;
        let values = value::array_at(context, data_offset, "call-site data")?;
        sites.push(CallSite {
            values,
            data_offset,
        });
    }
    Ok(sites)
}

pub(super) fn type_list(
    context: &Context<'_>,
    encoded_offset: u32,
    what: &str,
) -> Result<Vec<TypeIndex>> {
    if encoded_offset == ABSENT_OFFSET {
        return Ok(Vec::new());
    }
    let offset = context.offset(encoded_offset, Alignment::Word, what)?;
    let entries_offset = offset + ListField::Entries.offset();
    let count = context.reader.u32(offset + ListField::Size.offset())?;
    let count = context.count(count, ItemWidth::CODE_UNIT, entries_offset, what)?;
    context
        .reader
        .bytes(entries_offset, count * ItemWidth::CODE_UNIT.bytes())?;
    let mut types = Vec::with_capacity(count);
    for index in 0..count {
        let entry_offset = entries_offset + index * ItemWidth::CODE_UNIT.bytes();
        let value = u32::from(context.reader.u16(entry_offset)?);
        context.index(value, context.header.type_ids.size, entry_offset, what)?;
        types.push(TypeIndex(value));
    }
    Ok(types)
}

//! Class definitions, delta-encoded members, and hidden-API flags.

use crate::{Error, Result};

use super::{Context, annotations, code, tables, value};
use crate::file::header::{ABSENT_OFFSET, NO_INDEX};
use crate::file::layout::{
    Alignment, ClassField, DUPLICATE_INDEX_DELTA, EMPTY_ITEM_COUNT_USIZE, ItemWidth,
    UNREPRESENTABLE_FILE_OFFSET,
};
use crate::file::model::{
    AccessFlags, ClassData, ClassDefinition, EncodedField, EncodedMethod, FieldId, FieldIndex,
    MethodId, MethodIndex, StringIndex, TypeIndex,
};

pub(super) fn classes(
    context: &Context<'_>,
    fields: &[FieldId],
    methods: &[MethodId],
) -> Result<Vec<ClassDefinition>> {
    let Some((offset, count)) = context.fixed_section(
        context.header.class_defs,
        ItemWidth::CLASS_DEFINITION,
        "class definitions",
    )?
    else {
        return Ok(Vec::new());
    };
    let mut classes = Vec::with_capacity(count);
    for index in 0..count {
        let item_offset = offset + index * ItemWidth::CLASS_DEFINITION.bytes();
        let class = context
            .reader
            .u32(item_offset + ClassField::Class.offset())?;
        context.index(
            class,
            context.header.type_ids.size,
            item_offset,
            "defined class",
        )?;
        let access_flags = AccessFlags::from_bits_retain(
            context
                .reader
                .u32(item_offset + ClassField::AccessFlags.offset())?,
        );
        let superclass = optional_index(
            context,
            context
                .reader
                .u32(item_offset + ClassField::Superclass.offset())?,
            context.header.type_ids.size,
            item_offset + ClassField::Superclass.offset(),
            "superclass",
        )?
        .map(TypeIndex);
        let interfaces_offset = context
            .reader
            .u32(item_offset + ClassField::InterfacesOffset.offset())?;
        let interfaces = tables::type_list(context, interfaces_offset, "class interfaces")?;
        let source_file = optional_index(
            context,
            context
                .reader
                .u32(item_offset + ClassField::SourceFile.offset())?,
            context.header.string_ids.size,
            item_offset + ClassField::SourceFile.offset(),
            "source file",
        )?
        .map(StringIndex);
        let annotation_offset = context
            .reader
            .u32(item_offset + ClassField::AnnotationsOffset.offset())?;
        let class_data_offset = context
            .reader
            .u32(item_offset + ClassField::ClassDataOffset.offset())?;
        let static_values_offset = context
            .reader
            .u32(item_offset + ClassField::StaticValuesOffset.offset())?;
        let class_data = if class_data_offset == ABSENT_OFFSET {
            None
        } else {
            Some(data(
                context,
                class_data_offset,
                TypeIndex(class),
                fields,
                methods,
            )?)
        };
        let static_values = if static_values_offset == ABSENT_OFFSET {
            Vec::new()
        } else {
            value::array_at(context, static_values_offset, "static values")?
        };
        if static_values.len()
            > class_data
                .as_ref()
                .map_or(EMPTY_ITEM_COUNT_USIZE, |data| data.static_fields.len())
        {
            return Err(Error::invalid_dex(
                item_offset + ClassField::StaticValuesOffset.offset(),
                "static value count exceeds declared static fields",
            ));
        }
        classes.push(ClassDefinition {
            class: TypeIndex(class),
            access_flags,
            superclass,
            interfaces,
            source_file,
            annotations: annotations::directory(context, annotation_offset)?,
            class_data,
            static_values,
            definition_index: u32::try_from(index).map_err(|_| {
                Error::invalid_dex(item_offset, "class definition index exceeds 32 bits")
            })?,
        });
    }
    Ok(classes)
}

fn data(
    context: &Context<'_>,
    encoded_offset: u32,
    defining_class: TypeIndex,
    fields: &[FieldId],
    methods: &[MethodId],
) -> Result<ClassData> {
    let offset = context.offset(encoded_offset, Alignment::Byte, "class data")?;
    let mut cursor = context.reader.cursor(offset)?;
    let static_count = cursor.uleb128()?;
    let instance_count = cursor.uleb128()?;
    let direct_count = cursor.uleb128()?;
    let virtual_count = cursor.uleb128()?;
    let static_fields = encoded_fields(
        context,
        &mut cursor,
        static_count,
        defining_class,
        fields,
        "static fields",
    )?;
    let instance_fields = encoded_fields(
        context,
        &mut cursor,
        instance_count,
        defining_class,
        fields,
        "instance fields",
    )?;
    let direct_methods = encoded_methods(
        context,
        &mut cursor,
        direct_count,
        defining_class,
        methods,
        "direct methods",
    )?;
    let virtual_methods = encoded_methods(
        context,
        &mut cursor,
        virtual_count,
        defining_class,
        methods,
        "virtual methods",
    )?;
    Ok(ClassData {
        static_fields,
        instance_fields,
        direct_methods,
        virtual_methods,
        data_offset: encoded_offset,
    })
}

fn encoded_fields(
    context: &Context<'_>,
    cursor: &mut crate::file::io::Cursor<'_>,
    encoded_count: u32,
    defining_class: TypeIndex,
    fields: &[FieldId],
    what: &str,
) -> Result<Vec<EncodedField>> {
    let count = context.count(
        encoded_count,
        ItemWidth::ENCODED_FIELD_MINIMUM,
        cursor.position(),
        what,
    )?;
    let mut output = Vec::with_capacity(count);
    let mut previous = None;
    for _ in 0..count {
        let item_offset = cursor.position();
        let difference = cursor.uleb128()?;
        if previous.is_some() && difference == DUPLICATE_INDEX_DELTA {
            return Err(Error::invalid_dex(
                item_offset,
                format!("{what} contains a duplicate index"),
            ));
        }
        let field = previous
            .map_or(Some(difference), |previous: u32| {
                previous.checked_add(difference)
            })
            .ok_or_else(|| Error::invalid_dex(item_offset, format!("{what} index overflowed")))?;
        context.index(field, context.header.field_ids.size, item_offset, what)?;
        if fields
            .get(usize::try_from(field).unwrap_or(UNREPRESENTABLE_FILE_OFFSET))
            .is_none_or(|entry| entry.class != defining_class)
        {
            return Err(Error::invalid_dex(
                item_offset,
                format!("{what} entry {field} belongs to another class"),
            ));
        }
        output.push(EncodedField {
            field: FieldIndex(field),
            access_flags: AccessFlags::from_bits_retain(cursor.uleb128()?),
        });
        previous = Some(field);
    }
    Ok(output)
}

fn encoded_methods(
    context: &Context<'_>,
    cursor: &mut crate::file::io::Cursor<'_>,
    encoded_count: u32,
    defining_class: TypeIndex,
    methods: &[MethodId],
    what: &str,
) -> Result<Vec<EncodedMethod>> {
    let count = context.count(
        encoded_count,
        ItemWidth::ENCODED_METHOD_MINIMUM,
        cursor.position(),
        what,
    )?;
    let mut output = Vec::with_capacity(count);
    let mut previous = None;
    for _ in 0..count {
        let item_offset = cursor.position();
        let difference = cursor.uleb128()?;
        if previous.is_some() && difference == DUPLICATE_INDEX_DELTA {
            return Err(Error::invalid_dex(
                item_offset,
                format!("{what} contains a duplicate index"),
            ));
        }
        let method = previous
            .map_or(Some(difference), |previous: u32| {
                previous.checked_add(difference)
            })
            .ok_or_else(|| Error::invalid_dex(item_offset, format!("{what} index overflowed")))?;
        context.index(method, context.header.method_ids.size, item_offset, what)?;
        if methods
            .get(usize::try_from(method).unwrap_or(UNREPRESENTABLE_FILE_OFFSET))
            .is_none_or(|entry| entry.class != defining_class)
        {
            return Err(Error::invalid_dex(
                item_offset,
                format!("{what} entry {method} belongs to another class"),
            ));
        }
        let access_flags = AccessFlags::from_bits_retain(cursor.uleb128()?);
        let code_offset = cursor.uleb128()?;
        let code = if code_offset == ABSENT_OFFSET {
            None
        } else {
            Some(code::item(context, code_offset)?)
        };
        output.push(EncodedMethod {
            method: MethodIndex(method),
            access_flags,
            code,
        });
        previous = Some(method);
    }
    Ok(output)
}

fn optional_index(
    context: &Context<'_>,
    value: u32,
    limit: u32,
    offset: usize,
    what: &str,
) -> Result<Option<u32>> {
    if value == NO_INDEX {
        Ok(None)
    } else {
        context.index(value, limit, offset, what).map(Some)
    }
}

//! Annotation sets, parameter lists, and directories.

use crate::{Error, Result};

use super::{Context, value};
use crate::file::header::ABSENT_OFFSET;
use crate::file::layout::{
    Alignment, AnnotationAssociationField, AnnotationDirectoryField, ItemWidth, ListField,
};
use crate::file::model::{
    AnnotationDirectory, AnnotationItem, AnnotationVisibility, FieldAnnotations, FieldIndex,
    MethodAnnotations, MethodIndex, ParameterAnnotations, ROOT_ENCODED_VALUE_DEPTH,
};

pub(super) fn directory(context: &Context<'_>, encoded_offset: u32) -> Result<AnnotationDirectory> {
    if encoded_offset == ABSENT_OFFSET {
        return Ok(AnnotationDirectory::default());
    }
    let offset = context.offset(encoded_offset, Alignment::Word, "annotation directory")?;
    let class_annotations_offset = context
        .reader
        .u32(offset + AnnotationDirectoryField::ClassAnnotationsOffset.offset())?;
    let fields_count = context
        .reader
        .u32(offset + AnnotationDirectoryField::FieldsSize.offset())?;
    let methods_count = context
        .reader
        .u32(offset + AnnotationDirectoryField::MethodsSize.offset())?;
    let parameters_count = context
        .reader
        .u32(offset + AnnotationDirectoryField::ParametersSize.offset())?;
    let associations_offset = offset + AnnotationDirectoryField::Associations.offset();
    let fields_count = context.count(
        fields_count,
        ItemWidth::ANNOTATION_ASSOCIATION,
        associations_offset,
        "field annotations",
    )?;
    let methods_offset = offset
        .checked_add(
            ItemWidth::ANNOTATION_DIRECTORY_HEADER.bytes()
                + fields_count * ItemWidth::ANNOTATION_ASSOCIATION.bytes(),
        )
        .ok_or_else(|| Error::invalid_dex(offset, "annotation directory size overflowed"))?;
    let methods_count = context.count(
        methods_count,
        ItemWidth::ANNOTATION_ASSOCIATION,
        methods_offset,
        "method annotations",
    )?;
    let parameters_offset = methods_offset
        .checked_add(methods_count * ItemWidth::ANNOTATION_ASSOCIATION.bytes())
        .ok_or_else(|| Error::invalid_dex(offset, "annotation directory size overflowed"))?;
    let parameters_count = context.count(
        parameters_count,
        ItemWidth::ANNOTATION_ASSOCIATION,
        parameters_offset,
        "parameter annotations",
    )?;
    context.reader.bytes(
        parameters_offset,
        parameters_count
            .checked_mul(ItemWidth::ANNOTATION_ASSOCIATION.bytes())
            .ok_or_else(|| Error::invalid_dex(offset, "parameter annotation size overflowed"))?,
    )?;

    Ok(AnnotationDirectory {
        class_annotations: annotation_set(context, class_annotations_offset)?,
        fields: field_associations(context, associations_offset, fields_count)?,
        methods: method_associations(context, methods_offset, methods_count)?,
        parameters: parameter_associations(context, parameters_offset, parameters_count)?,
        data_offset: encoded_offset,
    })
}

fn field_associations(
    context: &Context<'_>,
    offset: usize,
    count: usize,
) -> Result<Vec<FieldAnnotations>> {
    let mut output = Vec::with_capacity(count);
    let mut previous = None;
    for index in 0..count {
        let item_offset = offset + index * ItemWidth::ANNOTATION_ASSOCIATION.bytes();
        let field = context
            .reader
            .u32(item_offset + AnnotationAssociationField::Identity.offset())?;
        context.index(
            field,
            context.header.field_ids.size,
            item_offset,
            "annotated field",
        )?;
        require_increasing(previous, field, item_offset, "annotated field")?;
        previous = Some(field);
        output.push(FieldAnnotations {
            field: FieldIndex(field),
            annotations: annotation_set(
                context,
                context
                    .reader
                    .u32(item_offset + AnnotationAssociationField::AnnotationsOffset.offset())?,
            )?,
        });
    }
    Ok(output)
}

fn method_associations(
    context: &Context<'_>,
    offset: usize,
    count: usize,
) -> Result<Vec<MethodAnnotations>> {
    let mut output = Vec::with_capacity(count);
    let mut previous = None;
    for index in 0..count {
        let item_offset = offset + index * ItemWidth::ANNOTATION_ASSOCIATION.bytes();
        let method = context
            .reader
            .u32(item_offset + AnnotationAssociationField::Identity.offset())?;
        context.index(
            method,
            context.header.method_ids.size,
            item_offset,
            "annotated method",
        )?;
        require_increasing(previous, method, item_offset, "annotated method")?;
        previous = Some(method);
        output.push(MethodAnnotations {
            method: MethodIndex(method),
            annotations: annotation_set(
                context,
                context
                    .reader
                    .u32(item_offset + AnnotationAssociationField::AnnotationsOffset.offset())?,
            )?,
        });
    }
    Ok(output)
}

fn parameter_associations(
    context: &Context<'_>,
    offset: usize,
    count: usize,
) -> Result<Vec<ParameterAnnotations>> {
    let mut output = Vec::with_capacity(count);
    let mut previous = None;
    for index in 0..count {
        let item_offset = offset + index * ItemWidth::ANNOTATION_ASSOCIATION.bytes();
        let method = context
            .reader
            .u32(item_offset + AnnotationAssociationField::Identity.offset())?;
        context.index(
            method,
            context.header.method_ids.size,
            item_offset,
            "parameter-annotated method",
        )?;
        require_increasing(previous, method, item_offset, "parameter-annotated method")?;
        previous = Some(method);
        let list_offset = context
            .reader
            .u32(item_offset + AnnotationAssociationField::AnnotationsOffset.offset())?;
        output.push(ParameterAnnotations {
            method: MethodIndex(method),
            parameters: annotation_set_ref_list(context, list_offset)?,
        });
    }
    Ok(output)
}

fn annotation_set_ref_list(
    context: &Context<'_>,
    encoded_offset: u32,
) -> Result<Vec<Vec<AnnotationItem>>> {
    let offset = context.offset(
        encoded_offset,
        Alignment::Word,
        "annotation-set reference list",
    )?;
    let entries_offset = offset + ListField::Entries.offset();
    let count = context.reader.u32(offset + ListField::Size.offset())?;
    let count = context.count(
        count,
        ItemWidth::WORD,
        entries_offset,
        "annotation-set reference list",
    )?;
    context
        .reader
        .bytes(entries_offset, count * ItemWidth::WORD.bytes())?;
    let mut sets = Vec::with_capacity(count);
    for index in 0..count {
        sets.push(annotation_set(
            context,
            context
                .reader
                .u32(entries_offset + index * ItemWidth::WORD.bytes())?,
        )?);
    }
    Ok(sets)
}

fn annotation_set(context: &Context<'_>, encoded_offset: u32) -> Result<Vec<AnnotationItem>> {
    if encoded_offset == ABSENT_OFFSET {
        return Ok(Vec::new());
    }
    let offset = context.offset(encoded_offset, Alignment::Word, "annotation set")?;
    let entries_offset = offset + ListField::Entries.offset();
    let count = context.reader.u32(offset + ListField::Size.offset())?;
    let count = context.count(count, ItemWidth::WORD, entries_offset, "annotation set")?;
    context
        .reader
        .bytes(entries_offset, count * ItemWidth::WORD.bytes())?;
    let mut annotations = Vec::with_capacity(count);
    let mut previous_type = None;
    for index in 0..count {
        let entry_offset = entries_offset + index * ItemWidth::WORD.bytes();
        let item_offset = context.reader.u32(entry_offset)?;
        let annotation = annotation_item(context, item_offset)?;
        let current_type = annotation.annotation.annotation_type.get();
        require_increasing(previous_type, current_type, entry_offset, "annotation type")?;
        previous_type = Some(current_type);
        annotations.push(annotation);
    }
    Ok(annotations)
}

fn annotation_item(context: &Context<'_>, encoded_offset: u32) -> Result<AnnotationItem> {
    let offset = context.offset(encoded_offset, Alignment::Byte, "annotation item")?;
    let mut cursor = context.reader.cursor(offset)?;
    let visibility_byte = cursor.u8()?;
    let visibility = AnnotationVisibility::from_byte(visibility_byte).ok_or_else(|| {
        Error::invalid_dex(
            offset,
            format!("unknown annotation visibility {visibility_byte}"),
        )
    })?;
    Ok(AnnotationItem {
        visibility,
        annotation: value::annotation(context, &mut cursor, ROOT_ENCODED_VALUE_DEPTH)?,
        data_offset: encoded_offset,
    })
}

fn require_increasing(
    previous: Option<u32>,
    current: u32,
    offset: usize,
    what: &str,
) -> Result<()> {
    if previous.is_none_or(|previous| current > previous) {
        Ok(())
    } else {
        Err(Error::invalid_dex(
            offset,
            format!("{what} indices are not strictly increasing"),
        ))
    }
}

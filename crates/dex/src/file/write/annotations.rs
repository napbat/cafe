//! Annotation items, sets, parameter lists, and directories.

use crate::file::header::ABSENT_OFFSET;
use crate::file::io::Writer;
use crate::file::layout::{
    Alignment, AnnotationAssociationField, AnnotationDirectoryField, EMPTY_ITEM_COUNT,
    ITEM_COUNT_INCREMENT, ItemWidth,
};
use crate::file::model::{
    AnnotationItem, ClassDefinition, FieldIndex, MapItem, MapItemType, MethodIndex,
    ROOT_ENCODED_VALUE_DEPTH,
};
use crate::{Error, Result};

pub(super) struct AnnotationLayout {
    pub(super) directory_offsets: Vec<u32>,
    pub(super) sections: Vec<MapItem>,
}

pub(super) fn write(writer: &mut Writer, classes: &[ClassDefinition]) -> Result<AnnotationLayout> {
    let mut sections = Vec::new();
    let (pending, item_section) = write_items(writer, classes)?;
    if let Some(section) = item_section {
        sections.push(section);
    }
    let (sets, set_section) = write_sets(writer, &pending)?;
    if let Some(section) = set_section {
        sections.push(section);
    }
    let (parameters, reference_section) = write_parameter_references(writer, &sets)?;
    if let Some(section) = reference_section {
        sections.push(section);
    }
    let (directory_offsets, directory_section) = write_directories(writer, &sets, &parameters)?;
    if let Some(section) = directory_section {
        sections.push(section);
    }
    Ok(AnnotationLayout {
        directory_offsets,
        sections,
    })
}

struct PendingDirectory {
    class_annotations: Vec<u32>,
    fields: Vec<(FieldIndex, Vec<u32>)>,
    methods: Vec<(MethodIndex, Vec<u32>)>,
    parameters: Vec<(MethodIndex, Vec<Vec<u32>>)>,
}

struct SetDirectory {
    class_annotations: u32,
    fields: Vec<(FieldIndex, u32)>,
    methods: Vec<(MethodIndex, u32)>,
    parameters: Vec<(MethodIndex, Vec<u32>)>,
}

fn write_items(
    writer: &mut Writer,
    classes: &[ClassDefinition],
) -> Result<(Vec<PendingDirectory>, Option<MapItem>)> {
    let start = writer.position()?;
    let mut count = 0u32;
    let mut pending = Vec::with_capacity(classes.len());
    for class in classes {
        let directory = &class.annotations;
        pending.push(PendingDirectory {
            class_annotations: item_set(writer, &directory.class_annotations, &mut count)?,
            fields: directory
                .fields
                .iter()
                .map(|association| {
                    Ok((
                        association.field,
                        item_set(writer, &association.annotations, &mut count)?,
                    ))
                })
                .collect::<Result<_>>()?,
            methods: directory
                .methods
                .iter()
                .map(|association| {
                    Ok((
                        association.method,
                        item_set(writer, &association.annotations, &mut count)?,
                    ))
                })
                .collect::<Result<_>>()?,
            parameters: directory
                .parameters
                .iter()
                .map(|association| {
                    Ok((
                        association.method,
                        association
                            .parameters
                            .iter()
                            .map(|annotations| item_set(writer, annotations, &mut count))
                            .collect::<Result<_>>()?,
                    ))
                })
                .collect::<Result<_>>()?,
        });
    }
    Ok((pending, section(MapItemType::Annotation, count, start)))
}

fn item_set(
    writer: &mut Writer,
    annotations: &[AnnotationItem],
    count: &mut u32,
) -> Result<Vec<u32>> {
    annotations
        .iter()
        .map(|annotation| {
            let offset = writer.position()?;
            writer.u8(annotation.visibility.byte());
            super::value::annotation(writer, &annotation.annotation, ROOT_ENCODED_VALUE_DEPTH)?;
            *count = count
                .checked_add(ITEM_COUNT_INCREMENT)
                .ok_or_else(|| Error::invalid_assembly("annotation item count overflowed"))?;
            Ok(offset)
        })
        .collect()
}

fn write_sets(
    writer: &mut Writer,
    pending: &[PendingDirectory],
) -> Result<(Vec<SetDirectory>, Option<MapItem>)> {
    writer.align(Alignment::Word)?;
    let start = writer.position()?;
    let mut count = 0u32;
    let mut directories = Vec::with_capacity(pending.len());
    for directory in pending {
        directories.push(SetDirectory {
            class_annotations: annotation_set(writer, &directory.class_annotations, &mut count)?,
            fields: directory
                .fields
                .iter()
                .map(|(field, offsets)| Ok((*field, annotation_set(writer, offsets, &mut count)?)))
                .collect::<Result<_>>()?,
            methods: directory
                .methods
                .iter()
                .map(|(method, offsets)| {
                    Ok((*method, annotation_set(writer, offsets, &mut count)?))
                })
                .collect::<Result<_>>()?,
            parameters: directory
                .parameters
                .iter()
                .map(|(method, parameters)| {
                    Ok((
                        *method,
                        parameters
                            .iter()
                            .map(|offsets| annotation_set(writer, offsets, &mut count))
                            .collect::<Result<_>>()?,
                    ))
                })
                .collect::<Result<_>>()?,
        });
    }
    Ok((
        directories,
        section(MapItemType::AnnotationSet, count, start),
    ))
}

fn annotation_set(writer: &mut Writer, offsets: &[u32], count: &mut u32) -> Result<u32> {
    if offsets.is_empty() {
        return Ok(ABSENT_OFFSET);
    }
    let offset = writer.position()?;
    writer.u32(u32::try_from(offsets.len()).map_err(|_| {
        Error::invalid_assembly("annotation set count exceeds 32-bit address space")
    })?);
    for item in offsets {
        writer.u32(*item);
    }
    *count = count
        .checked_add(ITEM_COUNT_INCREMENT)
        .ok_or_else(|| Error::invalid_assembly("annotation set count overflowed"))?;
    Ok(offset)
}

fn write_parameter_references(
    writer: &mut Writer,
    sets: &[SetDirectory],
) -> Result<(Vec<Vec<u32>>, Option<MapItem>)> {
    writer.align(Alignment::Word)?;
    let start = writer.position()?;
    let mut count = 0u32;
    let mut directories = Vec::with_capacity(sets.len());
    for directory in sets {
        let mut offsets = Vec::with_capacity(directory.parameters.len());
        for (_, parameters) in &directory.parameters {
            let offset = writer.position()?;
            writer.u32(u32::try_from(parameters.len()).map_err(|_| {
                Error::invalid_assembly("parameter annotation count exceeds 32 bits")
            })?);
            for parameter in parameters {
                writer.u32(*parameter);
            }
            count = count.checked_add(ITEM_COUNT_INCREMENT).ok_or_else(|| {
                Error::invalid_assembly("annotation-set reference count overflowed")
            })?;
            offsets.push(offset);
        }
        directories.push(offsets);
    }
    Ok((
        directories,
        section(MapItemType::AnnotationSetRefList, count, start),
    ))
}

fn write_directories(
    writer: &mut Writer,
    sets: &[SetDirectory],
    parameter_references: &[Vec<u32>],
) -> Result<(Vec<u32>, Option<MapItem>)> {
    writer.align(Alignment::Word)?;
    let start = writer.position()?;
    let mut count = 0u32;
    let mut offsets = Vec::with_capacity(sets.len());
    for (directory, references) in sets.iter().zip(parameter_references) {
        if directory.class_annotations == ABSENT_OFFSET
            && directory.fields.is_empty()
            && directory.methods.is_empty()
            && directory.parameters.is_empty()
        {
            offsets.push(ABSENT_OFFSET);
            continue;
        }
        if references.len() != directory.parameters.len() {
            return Err(Error::invalid_assembly(
                "parameter annotation layout lost an association",
            ));
        }
        let offset = writer.position()?;
        writer.reserve(ItemWidth::ANNOTATION_DIRECTORY_HEADER.bytes())?;
        writer.patch_u32(
            offset + AnnotationDirectoryField::ClassAnnotationsOffset.offset_u32(),
            directory.class_annotations,
        )?;
        writer.patch_u32(
            offset + AnnotationDirectoryField::FieldsSize.offset_u32(),
            u32::try_from(directory.fields.len())
                .map_err(|_| Error::invalid_assembly("field annotation count exceeds 32 bits"))?,
        )?;
        writer.patch_u32(
            offset + AnnotationDirectoryField::MethodsSize.offset_u32(),
            u32::try_from(directory.methods.len())
                .map_err(|_| Error::invalid_assembly("method annotation count exceeds 32 bits"))?,
        )?;
        writer.patch_u32(
            offset + AnnotationDirectoryField::ParametersSize.offset_u32(),
            u32::try_from(directory.parameters.len()).map_err(|_| {
                Error::invalid_assembly("parameter annotation association count exceeds 32 bits")
            })?,
        )?;
        write_associations(writer, &directory.fields)?;
        write_associations(writer, &directory.methods)?;
        for ((method, _), reference) in directory.parameters.iter().zip(references) {
            writer.u32(method.get());
            writer.u32(*reference);
        }
        count = count
            .checked_add(ITEM_COUNT_INCREMENT)
            .ok_or_else(|| Error::invalid_assembly("annotation directory count overflowed"))?;
        offsets.push(offset);
    }
    Ok((
        offsets,
        section(MapItemType::AnnotationDirectory, count, start),
    ))
}

fn write_associations<T: Copy + IntoIndex>(writer: &mut Writer, values: &[(T, u32)]) -> Result<()> {
    let bytes = values
        .len()
        .checked_mul(ItemWidth::ANNOTATION_ASSOCIATION.bytes())
        .ok_or_else(|| Error::invalid_assembly("annotation association size overflowed"))?;
    writer.reserve(bytes)?;
    let end = writer.position()?;
    let start = end
        .checked_sub(
            u32::try_from(bytes).map_err(|_| {
                Error::invalid_assembly("annotation association size exceeds 32 bits")
            })?,
        )
        .ok_or_else(|| Error::invalid_assembly("annotation association start underflowed"))?;
    for (index, (identity, offset)) in values.iter().enumerate() {
        let delta =
            u32::try_from(index * ItemWidth::ANNOTATION_ASSOCIATION.bytes()).map_err(|_| {
                Error::invalid_assembly("annotation association offset exceeds 32 bits")
            })?;
        writer.patch_u32(
            start + delta + AnnotationAssociationField::Identity.offset_u32(),
            identity.index(),
        )?;
        writer.patch_u32(
            start + delta + AnnotationAssociationField::AnnotationsOffset.offset_u32(),
            *offset,
        )?;
    }
    Ok(())
}

trait IntoIndex {
    fn index(self) -> u32;
}

impl IntoIndex for FieldIndex {
    fn index(self) -> u32 {
        self.get()
    }
}

impl IntoIndex for MethodIndex {
    fn index(self) -> u32 {
        self.get()
    }
}

fn section(item_type: MapItemType, count: u32, offset: u32) -> Option<MapItem> {
    (count != EMPTY_ITEM_COUNT).then_some(MapItem {
        item_type,
        size: count,
        offset,
    })
}

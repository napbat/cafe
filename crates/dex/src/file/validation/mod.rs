//! Cross-table, descriptor, declaration, and bytecode validation.

mod bytecode;
mod descriptor;

use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};

use crate::{Error, Result};

use self::descriptor::{DescriptorKind, descriptor, member_name, shorty};
use super::header::{DexHeader, DexVersion};
use super::layout::{
    ItemWidth, MAXIMUM_SMALL_ID_COUNT, UNLOCATED_ERROR_OFFSET, UNREPRESENTABLE_FILE_OFFSET,
};
use super::model::{
    CallSite, ClassDefinition, DexString, FieldId, MethodHandle, MethodId, PrototypeId, TypeId,
};

#[allow(clippy::too_many_arguments)]
pub(super) fn file(
    header: &DexHeader,
    strings: &[DexString],
    types: &[TypeId],
    prototypes: &[PrototypeId],
    fields: &[FieldId],
    methods: &[MethodId],
    classes: &[ClassDefinition],
    call_sites: &[CallSite],
    method_handles: &[MethodHandle],
) -> Result<()> {
    validate_table_sizes(header)?;
    validate_strings(header, strings)?;
    let descriptors = validate_types(header, strings, types)?;
    validate_prototypes(header, strings, &descriptors, prototypes)?;
    validate_fields(header, strings, &descriptors, fields)?;
    validate_methods(header, strings, &descriptors, methods)?;
    validate_dynamic_data(header.version, call_sites, method_handles)?;
    validate_classes(
        header,
        strings,
        &descriptors,
        prototypes,
        fields,
        methods,
        types,
        classes,
        call_sites,
        method_handles,
    )
}

fn validate_table_sizes(header: &DexHeader) -> Result<()> {
    for (name, section) in [
        ("type", header.type_ids),
        ("prototype", header.proto_ids),
        ("field", header.field_ids),
        ("method", header.method_ids),
    ] {
        if section.size > MAXIMUM_SMALL_ID_COUNT {
            return Err(Error::invalid_dex(
                usize::try_from(section.offset).unwrap_or(UNREPRESENTABLE_FILE_OFFSET),
                format!("{name} identifier table exceeds {MAXIMUM_SMALL_ID_COUNT} entries"),
            ));
        }
    }
    Ok(())
}

fn validate_strings(header: &DexHeader, strings: &[DexString]) -> Result<()> {
    for (index, pair) in strings.windows(2).enumerate() {
        if pair[0].utf16_units >= pair[1].utf16_units {
            return Err(table_error(
                header.string_ids.offset,
                index + 1,
                ItemWidth::STRING_ID,
                "string identifiers are not strictly ordered by UTF-16 contents",
            ));
        }
    }
    Ok(())
}

fn validate_types(
    header: &DexHeader,
    strings: &[DexString],
    types: &[TypeId],
) -> Result<Vec<DescriptorKind>> {
    let mut output = Vec::with_capacity(types.len());
    let mut previous = None;
    for (index, entry) in types.iter().enumerate() {
        let descriptor_index = entry.descriptor.get();
        if previous.is_some_and(|previous| descriptor_index <= previous) {
            return Err(table_error(
                header.type_ids.offset,
                index,
                ItemWidth::TYPE_ID,
                "type identifiers are not strictly ordered by descriptor index",
            ));
        }
        previous = Some(descriptor_index);
        let string = get(strings, descriptor_index, "type descriptor")?;
        output.push(descriptor(string, header.version).map_err(|message| {
            table_error(header.type_ids.offset, index, ItemWidth::TYPE_ID, &message)
        })?);
    }
    Ok(output)
}

fn validate_prototypes(
    header: &DexHeader,
    strings: &[DexString],
    descriptors: &[DescriptorKind],
    prototypes: &[PrototypeId],
) -> Result<()> {
    let mut previous: Option<&PrototypeId> = None;
    for (index, prototype) in prototypes.iter().enumerate() {
        let return_descriptor = get(descriptors, prototype.return_type.get(), "return type")?;
        let mut expected = String::new();
        expected.push(return_descriptor.shorty());
        for parameter in &prototype.parameters {
            let descriptor = get(descriptors, parameter.get(), "parameter type")?;
            if descriptor.is_void() {
                return Err(table_error(
                    header.proto_ids.offset,
                    index,
                    ItemWidth::PROTOTYPE_ID,
                    "prototype parameter is void",
                ));
            }
            expected.push(descriptor.shorty());
        }
        let actual = get(strings, prototype.shorty.get(), "shorty descriptor")?;
        shorty(actual).map_err(|message| {
            table_error(
                header.proto_ids.offset,
                index,
                ItemWidth::PROTOTYPE_ID,
                &message,
            )
        })?;
        if actual.utf16_units != expected.encode_utf16().collect::<Vec<_>>() {
            return Err(table_error(
                header.proto_ids.offset,
                index,
                ItemWidth::PROTOTYPE_ID,
                "shorty descriptor does not match its return and parameter types",
            ));
        }
        if let Some(previous) = previous
            && compare_prototypes(previous, prototype) != Ordering::Less
        {
            return Err(table_error(
                header.proto_ids.offset,
                index,
                ItemWidth::PROTOTYPE_ID,
                "prototype identifiers are not strictly ordered",
            ));
        }
        previous = Some(prototype);
    }
    Ok(())
}

fn validate_fields(
    header: &DexHeader,
    strings: &[DexString],
    descriptors: &[DescriptorKind],
    fields: &[FieldId],
) -> Result<()> {
    let mut previous = None;
    for (index, field) in fields.iter().enumerate() {
        if !get(descriptors, field.class.get(), "field class")?.is_class() {
            return Err(table_error(
                header.field_ids.offset,
                index,
                ItemWidth::FIELD_ID,
                "field definer is not a class type",
            ));
        }
        if get(descriptors, field.field_type.get(), "field type")?.is_void() {
            return Err(table_error(
                header.field_ids.offset,
                index,
                ItemWidth::FIELD_ID,
                "field type is void",
            ));
        }
        member_name(
            get(strings, field.name.get(), "field name")?,
            header.version,
        )
        .map_err(|message| {
            table_error(
                header.field_ids.offset,
                index,
                ItemWidth::FIELD_ID,
                &message,
            )
        })?;
        let key = (field.class.get(), field.name.get(), field.field_type.get());
        if previous.is_some_and(|previous| key <= previous) {
            return Err(table_error(
                header.field_ids.offset,
                index,
                ItemWidth::FIELD_ID,
                "field identifiers are not strictly ordered",
            ));
        }
        previous = Some(key);
    }
    Ok(())
}

fn validate_methods(
    header: &DexHeader,
    strings: &[DexString],
    descriptors: &[DescriptorKind],
    methods: &[MethodId],
) -> Result<()> {
    let mut previous = None;
    for (index, method) in methods.iter().enumerate() {
        if get(descriptors, method.class.get(), "method class")?.is_primitive_or_void() {
            return Err(table_error(
                header.method_ids.offset,
                index,
                ItemWidth::METHOD_ID,
                "method definer is a primitive or void type",
            ));
        }
        member_name(
            get(strings, method.name.get(), "method name")?,
            header.version,
        )
        .map_err(|message| {
            table_error(
                header.method_ids.offset,
                index,
                ItemWidth::METHOD_ID,
                &message,
            )
        })?;
        let key = (
            method.class.get(),
            method.name.get(),
            method.prototype.get(),
        );
        if previous.is_some_and(|previous| key <= previous) {
            return Err(table_error(
                header.method_ids.offset,
                index,
                ItemWidth::METHOD_ID,
                "method identifiers are not strictly ordered",
            ));
        }
        previous = Some(key);
    }
    Ok(())
}

fn validate_dynamic_data(
    version: DexVersion,
    call_sites: &[CallSite],
    method_handles: &[MethodHandle],
) -> Result<()> {
    if version < DexVersion::V038 && (!call_sites.is_empty() || !method_handles.is_empty()) {
        return Err(Error::invalid_dex(
            UNLOCATED_ERROR_OFFSET,
            "call sites and method handles require DEX version 038 or newer",
        ));
    }
    let mut previous = None;
    for call_site in call_sites {
        if previous.is_some_and(|previous| call_site.data_offset <= previous) {
            return Err(Error::invalid_dex(
                usize::try_from(call_site.data_offset).unwrap_or(UNREPRESENTABLE_FILE_OFFSET),
                "call-site identifiers are not ordered by data offset",
            ));
        }
        previous = Some(call_site.data_offset);
        if call_site.components().is_none() {
            return Err(Error::invalid_dex(
                usize::try_from(call_site.data_offset).unwrap_or(UNREPRESENTABLE_FILE_OFFSET),
                "call site does not begin with method-handle, string, and method-type values",
            ));
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn validate_classes(
    header: &DexHeader,
    strings: &[DexString],
    descriptors: &[DescriptorKind],
    prototypes: &[PrototypeId],
    fields: &[FieldId],
    methods: &[MethodId],
    types: &[TypeId],
    classes: &[ClassDefinition],
    call_sites: &[CallSite],
    method_handles: &[MethodHandle],
) -> Result<()> {
    let definitions: BTreeMap<_, _> = classes
        .iter()
        .enumerate()
        .map(|(position, class)| (class.class.get(), position))
        .collect();
    if definitions.len() != classes.len() {
        return Err(Error::invalid_dex(
            usize::try_from(header.class_defs.offset).unwrap_or(UNREPRESENTABLE_FILE_OFFSET),
            "a class type is defined more than once",
        ));
    }
    for (position, class) in classes.iter().enumerate() {
        if !get(descriptors, class.class.get(), "defined class")?.is_class() {
            return Err(table_error(
                header.class_defs.offset,
                position,
                ItemWidth::CLASS_DEFINITION,
                "class definition type is not a class",
            ));
        }
        validate_dependencies(position, class, descriptors, &definitions, header)?;
        validate_class_annotations(class, fields, methods, prototypes)?;
        if let Some(data) = &class.class_data {
            let direct: BTreeSet<_> = data
                .direct_methods
                .iter()
                .map(|method| method.method)
                .collect();
            if data
                .virtual_methods
                .iter()
                .any(|method| direct.contains(&method.method))
            {
                return Err(Error::invalid_dex(
                    usize::try_from(data.data_offset).unwrap_or(UNREPRESENTABLE_FILE_OFFSET),
                    "a method appears in both direct and virtual lists",
                ));
            }
            for method in data.direct_methods.iter().chain(&data.virtual_methods) {
                bytecode::method(
                    header.version,
                    strings,
                    types,
                    descriptors,
                    prototypes,
                    fields,
                    methods,
                    call_sites,
                    method_handles,
                    method,
                )?;
            }
        }
    }
    Ok(())
}

fn validate_dependencies(
    position: usize,
    class: &ClassDefinition,
    descriptors: &[DescriptorKind],
    definitions: &BTreeMap<u32, usize>,
    header: &DexHeader,
) -> Result<()> {
    let mut seen = BTreeSet::new();
    for dependency in class.superclass.iter().chain(&class.interfaces) {
        if !get(descriptors, dependency.get(), "class dependency")?.is_class() {
            return Err(table_error(
                header.class_defs.offset,
                position,
                ItemWidth::CLASS_DEFINITION,
                "superclass or interface is not a class type",
            ));
        }
        if !seen.insert(*dependency) {
            return Err(table_error(
                header.class_defs.offset,
                position,
                ItemWidth::CLASS_DEFINITION,
                "class dependency list contains a duplicate",
            ));
        }
        if definitions
            .get(&dependency.get())
            .is_some_and(|dependency_position| *dependency_position >= position)
        {
            return Err(table_error(
                header.class_defs.offset,
                position,
                ItemWidth::CLASS_DEFINITION,
                "defined superclass or interface does not precede this class",
            ));
        }
    }
    Ok(())
}

fn validate_class_annotations(
    class: &ClassDefinition,
    fields: &[FieldId],
    methods: &[MethodId],
    prototypes: &[PrototypeId],
) -> Result<()> {
    for association in &class.annotations.fields {
        if get(fields, association.field.get(), "annotated field")?.class != class.class {
            return Err(Error::invalid_dex(
                usize::try_from(class.annotations.data_offset)
                    .unwrap_or(UNREPRESENTABLE_FILE_OFFSET),
                "field annotation belongs to another class",
            ));
        }
    }
    for association in &class.annotations.methods {
        if get(methods, association.method.get(), "annotated method")?.class != class.class {
            return Err(Error::invalid_dex(
                usize::try_from(class.annotations.data_offset)
                    .unwrap_or(UNREPRESENTABLE_FILE_OFFSET),
                "method annotation belongs to another class",
            ));
        }
    }
    for association in &class.annotations.parameters {
        let method = get(
            methods,
            association.method.get(),
            "parameter-annotated method",
        )?;
        if method.class != class.class {
            return Err(Error::invalid_dex(
                usize::try_from(class.annotations.data_offset)
                    .unwrap_or(UNREPRESENTABLE_FILE_OFFSET),
                "parameter annotation belongs to another class",
            ));
        }
        let prototype = get(prototypes, method.prototype.get(), "annotated prototype")?;
        if association.parameters.len() != prototype.parameters.len() {
            return Err(Error::invalid_dex(
                usize::try_from(class.annotations.data_offset)
                    .unwrap_or(UNREPRESENTABLE_FILE_OFFSET),
                "parameter annotation count does not match the method prototype",
            ));
        }
    }
    Ok(())
}

fn compare_prototypes(first: &PrototypeId, second: &PrototypeId) -> Ordering {
    first
        .return_type
        .cmp(&second.return_type)
        .then_with(|| first.parameters.cmp(&second.parameters))
}

fn get<'a, T>(values: &'a [T], index: u32, what: &str) -> Result<&'a T> {
    usize::try_from(index)
        .ok()
        .and_then(|index| values.get(index))
        .ok_or_else(|| {
            Error::invalid_dex(
                UNLOCATED_ERROR_OFFSET,
                format!("{what} index {index} is out of bounds"),
            )
        })
}

fn table_error(base: u32, index: usize, width: ItemWidth, message: &str) -> Error {
    let offset = usize::try_from(base)
        .ok()
        .and_then(|base| {
            index
                .checked_mul(width.bytes())
                .and_then(|delta| base.checked_add(delta))
        })
        .unwrap_or(UNREPRESENTABLE_FILE_OFFSET);
    Error::invalid_dex(offset, message)
}

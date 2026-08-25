//! DEX hierarchy, member, and executable-body lifting into the program model.

use ::program::{
    FieldDefinition, FieldId, MethodDefinition, MethodId, Module, ModuleId, ModuleSource,
    RawAccessFlags, TypeDefinition, TypeId,
};
use disassembler::BinaryFormat;

use crate::disassembly;
use crate::file::{ClassDefinition, DexFile, EncodedMethod};
use crate::{DEFAULT_DEX_FILE_NAME, Result};

/// Controls whether program method definitions retain decoded DEX bodies.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum MethodBodyMode {
    /// Lift every available DEX `code_item` into shared disassembly.
    #[default]
    Disassemble,
    /// Retain declarations without constructing shared instruction bodies.
    DeclarationsOnly,
}

/// Configuration for lifting a DEX file into the shared program model.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct ProgramOptions {
    /// Method-body loading behavior.
    pub method_bodies: MethodBodyMode,
}

/// Builds a shared module using [`DEFAULT_DEX_FILE_NAME`] as its identity.
///
/// # Errors
///
/// Returns an error when identifiers, definitions, or executable bodies cannot
/// be represented by the shared model.
pub fn lift_file(file: &DexFile) -> Result<Module> {
    lift_file_named_with_options(file, DEFAULT_DEX_FILE_NAME, ProgramOptions::default())
}

/// Builds a shared module using the default artifact name and explicit options.
///
/// # Errors
///
/// Returns an error when identifiers, definitions, or requested executable
/// bodies cannot be represented by the shared model.
pub fn lift_file_with_options(file: &DexFile, options: ProgramOptions) -> Result<Module> {
    lift_file_named_with_options(file, DEFAULT_DEX_FILE_NAME, options)
}

/// Builds a shared module using an explicit artifact name and default options.
///
/// # Errors
///
/// Returns an error when identifiers, definitions, or executable bodies cannot
/// be represented by the shared model.
pub fn lift_file_named(file: &DexFile, name: impl Into<String>) -> Result<Module> {
    lift_file_named_with_options(file, name, ProgramOptions::default())
}

/// Builds a shared module using an explicit artifact name and body policy.
///
/// # Errors
///
/// Returns an error when identifiers, definitions, or requested executable
/// bodies cannot be represented by the shared model.
pub fn lift_file_named_with_options(
    file: &DexFile,
    name: impl Into<String>,
    options: ProgramOptions,
) -> Result<Module> {
    let format = BinaryFormat::Dex;
    let mut module = Module::new(ModuleId::new(format, name))?;
    for class in file.classes() {
        module.insert_type(lift_class(file, class, options)?)?;
    }
    Ok(module)
}

impl ModuleSource for DexFile {
    type Error = crate::Error;

    fn to_module(&self) -> Result<Module> {
        lift_file(self)
    }
}

fn lift_class(
    file: &DexFile,
    class: &ClassDefinition,
    options: ProgramOptions,
) -> Result<TypeDefinition> {
    let format = BinaryFormat::Dex;
    let owner = file.type_descriptor(class.class)?;
    let mut definition = TypeDefinition::new(
        TypeId::new(format, owner),
        RawAccessFlags::new(class.access_flags.bits()),
    )?;
    definition.set_superclass(
        class
            .superclass
            .map(|index| file.type_descriptor(index))
            .transpose()?
            .map(|name| TypeId::new(format, name)),
    );
    for &interface in &class.interfaces {
        definition.add_interface(TypeId::new(format, file.type_descriptor(interface)?));
    }

    let Some(data) = &class.class_data else {
        return Ok(definition);
    };
    for field in data.static_fields.iter().chain(&data.instance_fields) {
        let identity = file.resolve_field(field.field)?;
        definition.insert_field(FieldDefinition::new(
            FieldId::new(identity.name, identity.field_type),
            RawAccessFlags::new(field.access_flags.bits()),
        )?)?;
    }
    for method in data.direct_methods.iter().chain(&data.virtual_methods) {
        definition.insert_method(lift_method(file, method, options.method_bodies)?)?;
    }
    Ok(definition)
}

fn lift_method(
    file: &DexFile,
    method: &EncodedMethod,
    body_mode: MethodBodyMode,
) -> Result<MethodDefinition> {
    let identity = file.resolve_method(method.method)?;
    let body = match body_mode {
        MethodBodyMode::Disassemble => method
            .code
            .as_ref()
            .map(|code| disassembly::lift_body(file, code))
            .transpose()
            .map_err(|error| {
                error.in_method(identity.owner, identity.name, identity.signature.clone())
            })?,
        MethodBodyMode::DeclarationsOnly => None,
    };
    Ok(MethodDefinition::new(
        MethodId::new(identity.name, identity.signature),
        RawAccessFlags::new(method.access_flags.bits()),
        body,
    )?)
}

#[cfg(test)]
mod tests {
    use ::program::{ModuleSource, TypeId as ProgramTypeId};
    use disassembler::BinaryFormat;

    use super::{MethodBodyMode, ProgramOptions, lift_file, lift_file_with_options};
    use crate::file::{
        AccessFlags, AnnotationDirectory, ClassData, ClassDefinition, DexFile, DexString,
        DexVersion, EncodedField, FieldId, TypeId,
    };

    fn metadata_file() -> DexFile {
        let mut file = DexFile::new(DexVersion::V040);
        let field_name = file.push_string(DexString::new("value")).unwrap();
        let owner_descriptor = file.push_string(DexString::new("LChild;")).unwrap();
        let field_descriptor = file.push_string(DexString::new("I")).unwrap();
        let parent_descriptor = file.push_string(DexString::new("LParent;")).unwrap();
        let owner = file
            .push_type(TypeId {
                descriptor: owner_descriptor,
            })
            .unwrap();
        let field_type = file
            .push_type(TypeId {
                descriptor: field_descriptor,
            })
            .unwrap();
        let parent = file
            .push_type(TypeId {
                descriptor: parent_descriptor,
            })
            .unwrap();
        let field = file
            .push_field(FieldId {
                class: owner,
                field_type,
                name: field_name,
            })
            .unwrap();
        file.push_class(ClassDefinition {
            class: owner,
            access_flags: AccessFlags::PUBLIC,
            superclass: Some(parent),
            interfaces: Vec::new(),
            source_file: None,
            annotations: AnnotationDirectory::default(),
            class_data: Some(ClassData {
                static_fields: vec![EncodedField {
                    field,
                    access_flags: AccessFlags::PUBLIC,
                }],
                instance_fields: Vec::new(),
                direct_methods: Vec::new(),
                virtual_methods: Vec::new(),
                data_offset: 0,
            }),
            static_values: Vec::new(),
            definition_index: 0,
        })
        .unwrap();
        file
    }

    #[test]
    fn retains_dex_hierarchy_fields_and_format_identity() {
        let file = metadata_file();
        let direct = lift_file(&file).unwrap();
        let through_trait = file.to_module().unwrap();
        assert_eq!(direct, through_trait);

        let id = ProgramTypeId::new(BinaryFormat::Dex, "LChild;");
        let definition = direct.type_definition(&id).unwrap();
        assert_eq!(definition.superclass().unwrap().name, "LParent;");
        assert_eq!(definition.field_count(), 1);

        let declarations = lift_file_with_options(
            &file,
            ProgramOptions {
                method_bodies: MethodBodyMode::DeclarationsOnly,
            },
        )
        .unwrap();
        assert_eq!(declarations.field_count(), 1);
    }
}

//! JVM class metadata and disassembled-body lifting into a shared module.

use disassembler::BinaryFormat;
use program::{
    FieldDefinition, FieldId, MethodDefinition, MethodId, Module, ModuleId, ModuleSource,
    RawAccessFlags, TypeDefinition, TypeId,
};

use crate::classfile::ClassFile;
use crate::{Result, disassembly};

/// Controls whether program method definitions retain decoded instruction bodies.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum MethodBodyMode {
    /// Decode and lift every available JVM `Code` attribute.
    #[default]
    Disassemble,
    /// Retain declarations only, avoiding bytecode decoding for metadata tools.
    DeclarationsOnly,
}

/// Configuration for lifting a JVM class into the shared program model.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct ProgramOptions {
    /// Method-body loading behavior.
    pub method_bodies: MethodBodyMode,
}

/// Builds a shared program module from a parsed JVM class.
///
/// The module retains class hierarchy, field metadata, method metadata, and
/// each method's format-neutral disassembled body.
///
/// # Errors
///
/// Returns an error if class metadata or bytecode cannot be resolved, decoded,
/// or represented by the shared model.
pub fn lift_class(class: &ClassFile) -> Result<Module> {
    lift_class_with_options(class, ProgramOptions::default())
}

/// Builds a shared program module using explicit body-loading options.
///
/// # Errors
///
/// Returns an error if class metadata or requested bytecode bodies cannot be
/// resolved, decoded, or represented by the shared model.
pub fn lift_class_with_options(class: &ClassFile, options: ProgramOptions) -> Result<Module> {
    let format = BinaryFormat::JavaClass;
    let owner = class.class_name()?.to_owned();
    let type_id = TypeId::new(format, &owner);
    let mut definition = TypeDefinition::new(
        type_id,
        RawAccessFlags::new(u32::from(class.access_flags.bits())),
    )?;
    definition.set_superclass(class.super_name()?.map(|name| TypeId::new(format, name)));
    for &interface_index in &class.interfaces {
        let interface = class.constant_pool.class_name(interface_index)?;
        definition.add_interface(TypeId::new(format, interface));
    }
    for field in &class.fields {
        definition.insert_field(FieldDefinition::new(
            FieldId::new(
                field.name(&class.constant_pool)?,
                field.descriptor(&class.constant_pool)?,
            ),
            RawAccessFlags::new(u32::from(field.access_flags.bits())),
        )?)?;
    }

    match options.method_bodies {
        MethodBodyMode::Disassemble => add_disassembled_methods(&mut definition, class)?,
        MethodBodyMode::DeclarationsOnly => add_declared_methods(&mut definition, class)?,
    }

    let mut module = Module::new(ModuleId::new(format, owner))?;
    module.insert_type(definition)?;
    Ok(module)
}

fn add_disassembled_methods(definition: &mut TypeDefinition, class: &ClassFile) -> Result<()> {
    for function in disassembly::lift_class(class)?.functions {
        definition.insert_method(MethodDefinition::new(
            MethodId::new(function.symbol.name, function.symbol.signature),
            function.access_flags,
            function.body,
        )?)?;
    }
    Ok(())
}

fn add_declared_methods(definition: &mut TypeDefinition, class: &ClassFile) -> Result<()> {
    for method in &class.methods {
        definition.insert_method(MethodDefinition::new(
            MethodId::new(
                method.name(&class.constant_pool)?,
                method.descriptor(&class.constant_pool)?,
            ),
            RawAccessFlags::new(u32::from(method.access_flags.bits())),
            None,
        )?)?;
    }
    Ok(())
}

impl ModuleSource for ClassFile {
    type Error = crate::Error;

    fn to_module(&self) -> Result<Module> {
        lift_class(self)
    }
}

#[cfg(test)]
mod tests {
    use program::{ModuleSource, TypeId};

    use super::{MethodBodyMode, ProgramOptions, lift_class, lift_class_with_options};
    use crate::classfile::{
        ClassAccessFlags, ClassFile, Constant, ConstantPool, FieldAccessFlags, FieldInfo,
    };

    const JAVA_8_CLASS_MINOR: u16 = 0;
    const JAVA_8_CLASS_MAJOR: u16 = 52;

    #[test]
    fn retains_java_hierarchy_fields_and_flags() {
        let mut pool = ConstantPool::new();
        let class_name = pool.push_utf8("sample/Child").unwrap();
        let this_class = pool
            .push(Constant::Class {
                name_index: class_name,
            })
            .unwrap();
        let super_name = pool.push_utf8("sample/Parent").unwrap();
        let super_class = pool
            .push(Constant::Class {
                name_index: super_name,
            })
            .unwrap();
        let interface_name = pool.push_utf8("sample/Contract").unwrap();
        let interface = pool
            .push(Constant::Class {
                name_index: interface_name,
            })
            .unwrap();
        let field_name = pool.push_utf8("value").unwrap();
        let field_descriptor = pool.push_utf8("I").unwrap();
        let class = ClassFile {
            minor_version: JAVA_8_CLASS_MINOR,
            major_version: JAVA_8_CLASS_MAJOR,
            constant_pool: pool,
            access_flags: ClassAccessFlags::PUBLIC,
            this_class,
            super_class,
            interfaces: vec![interface],
            fields: vec![FieldInfo {
                access_flags: FieldAccessFlags::PUBLIC,
                name_index: field_name,
                descriptor_index: field_descriptor,
                attributes: Vec::new(),
            }],
            methods: Vec::new(),
            attributes: Vec::new(),
        };

        let direct = lift_class(&class).unwrap();
        let through_trait = class.to_module().unwrap();
        assert_eq!(direct, through_trait);
        let id = TypeId::new(disassembler::BinaryFormat::JavaClass, "sample/Child");
        let definition = direct.type_definition(&id).unwrap();
        assert_eq!(definition.superclass().unwrap().name, "sample/Parent");
        assert_eq!(definition.interfaces()[0].name, "sample/Contract");
        assert_eq!(definition.field_count(), 1);
        assert_eq!(
            definition.access_flags().bits(),
            u32::from(ClassAccessFlags::PUBLIC.bits())
        );

        let declarations = lift_class_with_options(
            &class,
            ProgramOptions {
                method_bodies: MethodBodyMode::DeclarationsOnly,
            },
        )
        .unwrap();
        assert_eq!(declarations.method_count(), 0);
    }
}

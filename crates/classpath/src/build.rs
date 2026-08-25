//! Native artifact and Program aggregation.

use std::collections::BTreeSet;

use dex::aab::{AabDexVisitControl, AabFile};
use dex::apk::{ApkFile, DexVisitControl};
use dex::file::DexFile;
use disassembler::BinaryFormat;
use java::classfile::ClassFile;
use java::jar::{ClassVisitControl, JarFile};
use java::jimage::{JimageFile, JimageVisitControl};
use java::jmod::JmodFile;
use program::{Program, TypeDefinition, TypeId};

use crate::{ClassDeclaration, ClassDescriptor, ClasspathHierarchy, DirectParents, Error, Result};

const OBJECT_DESCRIPTOR: &str = "Ljava/lang/Object;";

#[derive(Debug)]
struct PendingDeclaration {
    descriptor: ClassDescriptor,
    parents: DirectParents,
    format: BinaryFormat,
}

impl ClasspathHierarchy {
    /// Builds a hierarchy from JVM class declarations.
    ///
    /// # Errors
    ///
    /// Returns an error for invalid constant-pool references or conflicting
    /// duplicate declarations.
    pub fn from_java_classes<'a>(classes: impl IntoIterator<Item = &'a ClassFile>) -> Result<Self> {
        let mut hierarchy = Self::new();
        hierarchy.extend_java_classes(classes)?;
        Ok(hierarchy)
    }

    /// Adds JVM class declarations transactionally.
    ///
    /// # Errors
    ///
    /// Returns an error for invalid constant-pool references or conflicting
    /// duplicate declarations. This hierarchy is unchanged on failure.
    pub fn extend_java_classes<'a>(
        &mut self,
        classes: impl IntoIterator<Item = &'a ClassFile>,
    ) -> Result<()> {
        let pending = classes
            .into_iter()
            .map(pending_java)
            .collect::<Result<Vec<_>>>()?;
        self.extend_pending(pending)
    }

    /// Builds a hierarchy from one or more DEX files.
    ///
    /// # Errors
    ///
    /// Returns an error for invalid table references or conflicting duplicate
    /// declarations.
    pub fn from_dex_files<'a>(files: impl IntoIterator<Item = &'a DexFile>) -> Result<Self> {
        let mut hierarchy = Self::new();
        hierarchy.extend_dex_files(files)?;
        Ok(hierarchy)
    }

    /// Adds declarations from one or more DEX files transactionally.
    ///
    /// # Errors
    ///
    /// Returns an error for invalid table references or conflicting duplicate
    /// declarations. This hierarchy is unchanged on failure.
    pub fn extend_dex_files<'a>(
        &mut self,
        files: impl IntoIterator<Item = &'a DexFile>,
    ) -> Result<()> {
        let pending = files
            .into_iter()
            .flat_map(|file| {
                file.classes()
                    .iter()
                    .map(move |class| pending_dex(file, class))
            })
            .collect::<Result<Vec<_>>>()?;
        self.extend_pending(pending)
    }

    /// Builds a hierarchy from every definition in an owned Program.
    ///
    /// # Errors
    ///
    /// Returns an error for invalid native names or conflicting declarations.
    pub fn from_program(program: &Program) -> Result<Self> {
        let mut hierarchy = Self::new();
        hierarchy.extend_program(program)?;
        Ok(hierarchy)
    }

    /// Adds every Program definition transactionally.
    ///
    /// # Errors
    ///
    /// Returns an error for invalid native names or conflicting declarations.
    /// This hierarchy is unchanged on failure.
    pub fn extend_program(&mut self, program: &Program) -> Result<()> {
        let pending = program
            .types()
            .map(pending_program)
            .collect::<Result<Vec<_>>>()?;
        self.extend_pending(pending)
    }

    /// Adds every class in a JAR through its single-reader visitor.
    ///
    /// # Errors
    ///
    /// Returns the first archive, class-file, or declaration conflict error.
    pub fn extend_jar(&mut self, jar: &JarFile) -> Result<()> {
        let mut pending = Vec::new();
        jar.visit_classes(
            |_| true,
            |_, class| -> Result<ClassVisitControl> {
                pending.push(pending_java(&class)?);
                Ok(ClassVisitControl::Continue)
            },
        )?;
        self.extend_pending(pending)
    }

    /// Adds every class in a JMOD through its single-reader visitor.
    ///
    /// # Errors
    ///
    /// Returns the first archive, class-file, or declaration conflict error.
    pub fn extend_jmod(&mut self, jmod: &JmodFile) -> Result<()> {
        let mut pending = Vec::new();
        jmod.visit_classes(
            |_| true,
            |_, class| -> Result<ClassVisitControl> {
                pending.push(pending_java(&class)?);
                Ok(ClassVisitControl::Continue)
            },
        )?;
        self.extend_pending(pending)
    }

    /// Adds every class in a JIMAGE without reopening the image.
    ///
    /// # Errors
    ///
    /// Returns the first image, class-file, or declaration conflict error.
    pub fn extend_jimage(&mut self, image: &JimageFile) -> Result<()> {
        let mut pending = Vec::new();
        image.visit_classes(
            |_| true,
            |_, class| -> Result<JimageVisitControl> {
                pending.push(pending_java(&class)?);
                Ok(JimageVisitControl::Continue)
            },
        )?;
        self.extend_pending(pending)
    }

    /// Adds every canonical multidex member of an APK through one ZIP reader.
    ///
    /// # Errors
    ///
    /// Returns the first layout, entry, DEX, or declaration conflict error.
    pub fn extend_apk(&mut self, apk: &ApkFile) -> Result<()> {
        let mut pending = Vec::new();
        apk.visit_dex(
            |_| true,
            |artifact| -> Result<DexVisitControl> {
                pending.extend(pending_dex_file(&artifact.file)?);
                Ok(DexVisitControl::Continue)
            },
        )?;
        self.extend_pending(pending)
    }

    /// Adds every module-qualified DEX member of an AAB through one ZIP reader.
    ///
    /// # Errors
    ///
    /// Returns the first layout, entry, DEX, or declaration conflict error.
    pub fn extend_aab(&mut self, aab: &AabFile) -> Result<()> {
        let mut pending = Vec::new();
        aab.visit_dex(
            |_| true,
            |artifact| -> Result<AabDexVisitControl> {
                pending.extend(pending_dex_file(&artifact.file)?);
                Ok(AabDexVisitControl::Continue)
            },
        )?;
        self.extend_pending(pending)
    }

    fn extend_pending(
        &mut self,
        pending: impl IntoIterator<Item = PendingDeclaration>,
    ) -> Result<()> {
        let mut declarations = self.declarations.clone();
        for pending in pending {
            match declarations.get_mut(&pending.descriptor) {
                Some(existing) if existing.parents == pending.parents => {
                    existing.formats.insert(pending.format);
                }
                Some(existing) => {
                    return Err(Error::ConflictingDeclaration {
                        descriptor: pending.descriptor.to_string(),
                        existing: existing.parents.clone(),
                        incoming: pending.parents,
                    });
                }
                None => {
                    declarations.insert(
                        pending.descriptor.clone(),
                        ClassDeclaration {
                            descriptor: pending.descriptor,
                            parents: pending.parents,
                            formats: BTreeSet::from([pending.format]),
                        },
                    );
                }
            }
        }
        self.declarations = declarations;
        Ok(())
    }
}

fn pending_java(class: &ClassFile) -> Result<PendingDeclaration> {
    let descriptor = ClassDescriptor::from_jvm_internal(class.class_name()?)?;
    let superclass = class
        .super_name()?
        .map(ClassDescriptor::from_jvm_internal)
        .transpose()?;
    let interfaces = class
        .interfaces
        .iter()
        .map(|&index| {
            class
                .constant_pool
                .class_name(index)
                .map_err(Error::from)
                .and_then(ClassDescriptor::from_jvm_internal)
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(PendingDeclaration {
        parents: normalized_parents(&descriptor, superclass, interfaces)?,
        descriptor,
        format: BinaryFormat::JavaClass,
    })
}

fn pending_dex_file(file: &DexFile) -> Result<Vec<PendingDeclaration>> {
    file.classes()
        .iter()
        .map(|class| pending_dex(file, class))
        .collect()
}

fn pending_dex(file: &DexFile, class: &dex::file::ClassDefinition) -> Result<PendingDeclaration> {
    let descriptor = ClassDescriptor::from_dex_descriptor(file.type_descriptor(class.class)?)?;
    let superclass = class
        .superclass
        .map(|index| file.type_descriptor(index).map(str::to_owned))
        .transpose()?
        .map(ClassDescriptor::from_dex_descriptor)
        .transpose()?;
    let interfaces = class
        .interfaces
        .iter()
        .map(|&index| {
            file.type_descriptor(index)
                .map(str::to_owned)
                .map_err(Error::from)
                .and_then(ClassDescriptor::from_dex_descriptor)
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(PendingDeclaration {
        parents: normalized_parents(&descriptor, superclass, interfaces)?,
        descriptor,
        format: BinaryFormat::Dex,
    })
}

fn pending_program(definition: &TypeDefinition) -> Result<PendingDeclaration> {
    let descriptor = normalize_type_id(definition.id())?;
    let superclass = definition.superclass().map(normalize_type_id).transpose()?;
    let interfaces = definition
        .interfaces()
        .iter()
        .map(normalize_type_id)
        .collect::<Result<Vec<_>>>()?;
    Ok(PendingDeclaration {
        parents: normalized_parents(&descriptor, superclass, interfaces)?,
        descriptor,
        format: definition.id().format,
    })
}

fn normalize_type_id(id: &TypeId) -> Result<ClassDescriptor> {
    match id.format {
        BinaryFormat::JavaClass => ClassDescriptor::from_jvm_internal(id.name.clone()),
        BinaryFormat::Dex => ClassDescriptor::from_dex_descriptor(id.name.clone()),
    }
}

fn normalized_parents(
    descriptor: &ClassDescriptor,
    superclass: Option<ClassDescriptor>,
    interfaces: impl IntoIterator<Item = ClassDescriptor>,
) -> Result<DirectParents> {
    let superclass = if superclass.is_none() && descriptor.as_descriptor() != OBJECT_DESCRIPTOR {
        Some(ClassDescriptor::from_dex_descriptor(OBJECT_DESCRIPTOR)?)
    } else {
        superclass
    };
    Ok(DirectParents::new(superclass, interfaces))
}

#[cfg(test)]
mod tests {
    use dex::analysis::ReferenceHierarchy as _;
    use java::analysis::ReferenceHierarchy as _;
    use program::{Module, ModuleId, RawAccessFlags, TypeDefinition};

    use super::*;

    fn definition(format: BinaryFormat, name: &str, superclass: Option<&str>) -> TypeDefinition {
        let mut definition =
            TypeDefinition::new(TypeId::new(format, name), RawAccessFlags::default()).unwrap();
        definition.set_superclass(superclass.map(|name| TypeId::new(format, name)));
        definition
    }

    #[test]
    fn merges_equivalent_java_and_dex_declarations_into_both_views() {
        let mut java = Module::new(ModuleId::new(BinaryFormat::JavaClass, "java")).unwrap();
        java.insert_type(definition(
            BinaryFormat::JavaClass,
            "sample/Base",
            Some("java/lang/Object"),
        ))
        .unwrap();
        java.insert_type(definition(
            BinaryFormat::JavaClass,
            "sample/Child",
            Some("sample/Base"),
        ))
        .unwrap();
        let mut dex = Module::new(ModuleId::new(BinaryFormat::Dex, "dex")).unwrap();
        dex.insert_type(definition(
            BinaryFormat::Dex,
            "Lsample/Base;",
            Some("Ljava/lang/Object;"),
        ))
        .unwrap();
        let program = Program::from_modules([java, dex]);

        let hierarchy = ClasspathHierarchy::from_program(&program).unwrap();
        assert_eq!(hierarchy.len(), 2);
        let base = ClassDescriptor::from_dex_descriptor("Lsample/Base;").unwrap();
        assert_eq!(
            hierarchy
                .declaration(&base)
                .unwrap()
                .formats()
                .collect::<Vec<_>>(),
            vec![BinaryFormat::JavaClass, BinaryFormat::Dex]
        );
        assert!(
            hierarchy
                .jvm_view()
                .is_assignable("sample/Child", "sample/Base")
        );
        assert!(
            hierarchy
                .dex_view()
                .is_assignable("Lsample/Child;", "Lsample/Base;")
        );
    }

    #[test]
    fn conflicting_cross_format_declarations_are_atomic() {
        let mut hierarchy = ClasspathHierarchy::new();
        let original = Program::from_modules([{
            let mut module =
                Module::new(ModuleId::new(BinaryFormat::JavaClass, "original")).unwrap();
            module
                .insert_type(definition(
                    BinaryFormat::JavaClass,
                    "sample/Type",
                    Some("sample/First"),
                ))
                .unwrap();
            module
        }]);
        hierarchy.extend_program(&original).unwrap();
        let before = hierarchy.clone();
        let conflicting = Program::from_modules([{
            let mut module = Module::new(ModuleId::new(BinaryFormat::Dex, "incoming")).unwrap();
            module
                .insert_type(definition(
                    BinaryFormat::Dex,
                    "Lsample/Type;",
                    Some("Lsample/Second;"),
                ))
                .unwrap();
            module
        }]);

        assert!(matches!(
            hierarchy.extend_program(&conflicting),
            Err(Error::ConflictingDeclaration { .. })
        ));
        assert_eq!(hierarchy, before);
    }
}

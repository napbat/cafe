//! Extraction of native declarations from JVM class files.

use std::collections::BTreeMap;

use ::java::classfile::{ClassFile, Constant, MethodAccessFlags, Utf8Constant};
use ::java::jar::{ClassVisitControl, EntryId, JarFile, ResolvedEntry};

use crate::Result;
use crate::binding::NativeMethods;
use crate::descriptor::MethodDescriptor;
use crate::method::{InvocationKind, NativeMethod};
use crate::report::{NativeBindingReport, NativeOrigin, ProvenancedNativeMethod};
use crate::text::JavaText;

mod access;

pub use self::access::{
    JNI_NATIVE_ACCESS_RELEASE, ModuleIdentity, NativeAccessMode, NativeAccessRequirement,
    module_identity, native_access_requirement,
};

/// Extracts native declarations from one JVM class file in declaration order.
///
/// Names and descriptors retain their exact class-file UTF-16 code units.
/// Ordinary non-native methods are ignored and therefore do not affect JNI
/// overload selection.
///
/// # Errors
///
/// Returns an error for invalid constant-pool references, malformed method
/// descriptors, or duplicate native declarations.
pub fn native_methods(class: &ClassFile) -> Result<NativeMethods> {
    let owner = class_name(class)?;
    let mut methods = NativeMethods::new();
    for method in &class.methods {
        if !method.access_flags.contains(MethodAccessFlags::NATIVE) {
            continue;
        }
        let name = utf8_text(class, method.name_index)?;
        let descriptor = class.constant_pool.utf8_constant(method.descriptor_index)?;
        let descriptor = MethodDescriptor::from_utf16(descriptor.utf16_units().to_vec())?;
        let invocation = if method.access_flags.contains(MethodAccessFlags::STATIC) {
            InvocationKind::Static
        } else {
            InvocationKind::Instance
        };
        methods.insert(NativeMethod::from_parts(
            owner.clone(),
            name,
            descriptor,
            invocation,
        ))?;
    }
    Ok(methods)
}

/// Extracts the effective native declarations from a possibly multi-release
/// JAR for one target Java feature release.
///
/// Version selection occurs before class parsing. All selected payloads share
/// one ZIP reader.
///
/// # Errors
///
/// Returns an error for an ambiguous effective view, unreadable or malformed
/// selected class, invalid native declaration, or duplicate identity.
pub fn native_methods_in_jar(jar: &JarFile, target_release: u16) -> Result<NativeMethods> {
    binding_report_in_jar(jar, target_release, "<jar>")?.native_methods()
}

/// Builds a provenance-retaining binding report for a standalone class.
///
/// # Errors
///
/// Returns an error for invalid metadata, duplicate identities, or symbol
/// mapping failures.
pub fn binding_report(
    class: &ClassFile,
    artifact: impl Into<String>,
) -> Result<NativeBindingReport> {
    let origin = NativeOrigin::ClassFile {
        artifact: artifact.into(),
    };
    let declarations = native_methods(class)?
        .into_iter()
        .map(|method| ProvenancedNativeMethod::new(origin.clone(), method))
        .collect();
    NativeBindingReport::new(declarations)
}

/// Builds a provenance-retaining effective binding report for a JAR target.
///
/// # Errors
///
/// Returns an error for effective-view ambiguity, selected payload failures,
/// invalid native metadata, duplicate identities, or export collisions.
pub fn binding_report_in_jar(
    jar: &JarFile,
    target_release: u16,
    artifact: impl Into<String>,
) -> Result<NativeBindingReport> {
    let artifact = artifact.into();
    let selected = jar
        .effective_entries(target_release)?
        .into_iter()
        .filter(|entry| {
            entry
                .logical_name
                .ends_with(::java::jar::CLASS_ENTRY_SUFFIX)
        })
        .map(|entry| (entry.id, entry))
        .collect::<BTreeMap<EntryId, ResolvedEntry>>();
    let mut declarations = Vec::new();
    jar.visit_class_bytes(
        |entry| selected.contains_key(&entry.id),
        |entry, bytes| -> Result<ClassVisitControl> {
            let Some(resolution) = selected.get(&entry.id) else {
                return Ok(ClassVisitControl::Continue);
            };
            let bytes = bytes?;
            let class = ClassFile::parse(bytes).map_err(|error| error.in_jar_entry(entry.name))?;
            let origin = NativeOrigin::JarClass {
                artifact: artifact.clone(),
                logical_entry: resolution.logical_name.clone(),
                physical_entry: resolution.physical_name.clone(),
                target_release,
                selected_release: resolution.release,
            };
            declarations.extend(
                native_methods(&class)?
                    .into_iter()
                    .map(|method| ProvenancedNativeMethod::new(origin.clone(), method)),
            );
            Ok(ClassVisitControl::Continue)
        },
    )?;
    NativeBindingReport::new(declarations)
}

fn class_name(class: &ClassFile) -> Result<JavaText> {
    class.constant_pool.class_name(class.this_class)?;
    let Constant::Class { name_index } = class.constant_pool.get(class.this_class)? else {
        unreachable!("class_name accepted the same immutable constant-pool entry")
    };
    utf8_text(class, *name_index)
}

fn utf8_text(class: &ClassFile, index: u16) -> Result<JavaText> {
    let value: &Utf8Constant = class.constant_pool.utf8_constant(index)?;
    Ok(JavaText::from_utf16(value.utf16_units().to_vec()))
}

#[cfg(test)]
mod tests {
    use super::{binding_report_in_jar, native_methods, native_methods_in_jar};
    use crate::method::InvocationKind;
    use crate::symbol::SymbolStyle;
    use ::java::classfile::{ClassAccessFlags, ClassFile, JAVA_8_MAJOR_VERSION, MethodAccessFlags};
    use ::java::jar::JarFile;

    #[test]
    fn extracts_only_native_classfile_declarations() {
        let mut class = ClassFile::new(
            JAVA_8_MAJOR_VERSION,
            "sample/Native",
            Some("java/lang/Object"),
            ClassAccessFlags::PUBLIC,
        )
        .unwrap();
        class
            .add_method(
                MethodAccessFlags::PUBLIC | MethodAccessFlags::NATIVE,
                "read",
                "(I)I",
            )
            .unwrap();
        class
            .add_method(
                MethodAccessFlags::PUBLIC | MethodAccessFlags::STATIC | MethodAccessFlags::NATIVE,
                "open",
                "()V",
            )
            .unwrap();
        class
            .add_method(MethodAccessFlags::PUBLIC, "read", "()I")
            .unwrap();

        let methods = native_methods(&class).unwrap();
        let bindings = methods.bindings().unwrap();

        assert_eq!(methods.len(), 2);
        assert_eq!(methods.as_slice()[0].invocation(), InvocationKind::Instance);
        assert_eq!(methods.as_slice()[1].invocation(), InvocationKind::Static);
        assert_eq!(bindings[0].style(), SymbolStyle::Short);
        assert_eq!(bindings[0].symbol().as_str(), "Java_sample_Native_read");
    }

    #[test]
    fn selects_multi_release_classes_before_native_scanning() {
        let base = native_class("baseMethod");
        let modern = native_class("modernMethod");
        let mut jar = JarFile::new();
        jar.add_class(&base).unwrap();
        jar.set_multi_release(true).unwrap();
        jar.add_versioned_file(17, "sample/Native.class", modern.to_bytes().unwrap())
            .unwrap();

        let base_methods = native_methods_in_jar(&jar, 11).unwrap();
        let modern_report = binding_report_in_jar(&jar, 17, "native.jar").unwrap();

        assert_eq!(base_methods.as_slice()[0].name().as_str(), "baseMethod");
        assert_eq!(
            modern_report.declarations()[0].method().name().as_str(),
            "modernMethod"
        );
        let crate::report::NativeOrigin::JarClass {
            selected_release, ..
        } = modern_report.declarations()[0].origin()
        else {
            panic!("expected JAR provenance")
        };
        assert_eq!(*selected_release, Some(17));
    }

    fn native_class(method: &str) -> ClassFile {
        let mut class = ClassFile::new(
            JAVA_8_MAJOR_VERSION,
            "sample/Native",
            Some("java/lang/Object"),
            ClassAccessFlags::PUBLIC,
        )
        .unwrap();
        class
            .add_method(
                MethodAccessFlags::PUBLIC | MethodAccessFlags::NATIVE,
                method,
                "()V",
            )
            .unwrap();
        class
    }
}

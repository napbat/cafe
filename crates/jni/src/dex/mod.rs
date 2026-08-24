//! Extraction of native declarations from DEX files and APK multidex sets.

use ::dex::aab::{AabDexVisitControl, AabFile};
use ::dex::apk::{ApkFile, DexVisitControl};
use ::dex::file::{
    AccessFlags, DexFile, DexString, EncodedMethod, PrototypeIndex, StringIndex, TypeIndex,
};

use crate::binding::NativeMethods;
use crate::descriptor::{DescriptorTag, MethodDescriptor};
use crate::method::{InvocationKind, NativeMethod};
use crate::report::{NativeBindingReport, NativeOrigin, ProvenancedNativeMethod};
use crate::text::JavaText;
use crate::{Error, Result};

/// Extracts native declarations from one DEX file in class-data order.
///
/// Each class's direct methods precede its virtual methods, matching the two
/// declaration sequences in DEX `class_data_item`. Exact DEX UTF-16 names and
/// type descriptors are retained.
///
/// # Errors
///
/// Returns an error for invalid identifier references, non-object declaring
/// types, malformed method descriptors, or duplicate declarations.
pub fn native_methods(file: &DexFile) -> Result<NativeMethods> {
    let mut methods = NativeMethods::new();
    for (_, method) in native_methods_with_class(file)? {
        methods.insert(method)?;
    }
    Ok(methods)
}

/// Extracts native declarations from every DEX member of an APK in canonical
/// multidex order.
///
/// # Errors
///
/// Returns an error for an invalid APK multidex layout, an unreadable DEX
/// member, invalid native metadata, or a declaration repeated across members.
pub fn native_methods_in_apk(apk: &ApkFile) -> Result<NativeMethods> {
    let mut methods = NativeMethods::new();
    apk.visit_dex(
        |_| true,
        |artifact| -> Result<DexVisitControl> {
            methods.extend(native_methods(&artifact.file)?)?;
            Ok(DexVisitControl::Continue)
        },
    )?;
    Ok(methods)
}

/// Extracts native declarations from every module DEX in an App Bundle.
///
/// # Errors
///
/// Returns an error for an invalid bundle layout, unreadable DEX payload,
/// invalid native metadata, or a declaration repeated across modules.
pub fn native_methods_in_aab(aab: &AabFile) -> Result<NativeMethods> {
    let mut methods = NativeMethods::new();
    aab.visit_dex(
        |_| true,
        |artifact| -> Result<AabDexVisitControl> {
            methods.extend(native_methods(&artifact.file)?)?;
            Ok(AabDexVisitControl::Continue)
        },
    )?;
    Ok(methods)
}

/// Builds a provenance-retaining binding report for a standalone DEX file.
///
/// # Errors
///
/// Returns an error for invalid native metadata, duplicate identities, or
/// conventional export mapping failures.
pub fn binding_report(file: &DexFile, artifact: impl Into<String>) -> Result<NativeBindingReport> {
    let artifact = artifact.into();
    let declarations = native_methods_with_class(file)?
        .into_iter()
        .map(|(class_definition, method)| {
            ProvenancedNativeMethod::new(
                NativeOrigin::DexFile {
                    artifact: artifact.clone(),
                    class_definition,
                },
                method,
            )
        })
        .collect();
    NativeBindingReport::new(declarations)
}

/// Builds a provenance-retaining aggregate binding report for an APK.
///
/// # Errors
///
/// Returns an error for invalid multidex layout, a selected payload failure,
/// native metadata, duplicate identity, or export collision.
pub fn binding_report_in_apk(
    apk: &ApkFile,
    artifact: impl Into<String>,
) -> Result<NativeBindingReport> {
    let artifact = artifact.into();
    let mut declarations = Vec::new();
    apk.visit_dex(
        |_| true,
        |dex| -> Result<DexVisitControl> {
            for (class_definition, method) in native_methods_with_class(&dex.file)? {
                declarations.push(ProvenancedNativeMethod::new(
                    NativeOrigin::ApkDex {
                        artifact: artifact.clone(),
                        entry: dex.origin.entry_name.clone(),
                        ordinal: dex.origin.ordinal,
                        class_definition,
                    },
                    method,
                ));
            }
            Ok(DexVisitControl::Continue)
        },
    )?;
    NativeBindingReport::new(declarations)
}

/// Builds a module- and entry-provenanced binding report for an App Bundle.
///
/// # Errors
///
/// Returns an error for invalid module multidex layout, a selected payload
/// failure, native metadata, duplicate identity, or export collision.
pub fn binding_report_in_aab(
    aab: &AabFile,
    artifact: impl Into<String>,
) -> Result<NativeBindingReport> {
    let artifact = artifact.into();
    let mut declarations = Vec::new();
    aab.visit_dex(
        |_| true,
        |dex| -> Result<AabDexVisitControl> {
            for (class_definition, method) in native_methods_with_class(&dex.file)? {
                declarations.push(ProvenancedNativeMethod::new(
                    NativeOrigin::AabDex {
                        artifact: artifact.clone(),
                        module: dex.origin.module.clone(),
                        entry: dex.origin.entry_name.clone(),
                        ordinal: dex.origin.ordinal,
                        class_definition,
                    },
                    method,
                ));
            }
            Ok(AabDexVisitControl::Continue)
        },
    )?;
    NativeBindingReport::new(declarations)
}

fn native_methods_with_class(file: &DexFile) -> Result<Vec<(u32, NativeMethod)>> {
    let mut methods = Vec::new();
    for class in file.classes() {
        let Some(data) = &class.class_data else {
            continue;
        };
        for declaration in data.direct_methods.iter().chain(&data.virtual_methods) {
            if declaration.access_flags.contains(AccessFlags::NATIVE) {
                methods.push((class.definition_index, native_method(file, declaration)?));
            }
        }
    }
    Ok(methods)
}

fn native_method(file: &DexFile, declaration: &EncodedMethod) -> Result<NativeMethod> {
    let method = file.resolve_method_id(declaration.method)?;
    let owner = declaring_class_name(exact_type_descriptor(file, method.class)?)?;
    let name = exact_string(file, method.name)?;
    let descriptor = MethodDescriptor::from_utf16(prototype_descriptor(file, method.prototype)?)?;
    let invocation = if declaration.access_flags.contains(AccessFlags::STATIC) {
        InvocationKind::Static
    } else {
        InvocationKind::Instance
    };
    Ok(NativeMethod::from_parts(
        owner, name, descriptor, invocation,
    ))
}

fn declaring_class_name(descriptor: &DexString) -> Result<JavaText> {
    let prefix = [DescriptorTag::Object.unit()];
    let suffix = [DescriptorTag::ObjectEnd.unit()];
    let Some(internal_name) = descriptor
        .utf16_units
        .strip_prefix(&prefix)
        .and_then(|units| units.strip_suffix(&suffix))
        .filter(|units| !units.is_empty())
    else {
        return Err(Error::InvalidDexDeclaringType {
            descriptor: Box::new(exact_text(descriptor)),
        });
    };
    Ok(JavaText::from_utf16(internal_name.to_vec()))
}

fn prototype_descriptor(file: &DexFile, index: PrototypeIndex) -> Result<Vec<u16>> {
    let prototype = file.resolve_prototype(index)?;
    let mut units = vec![DescriptorTag::ParameterListStart.unit()];
    for &parameter in &prototype.parameters {
        units.extend_from_slice(&exact_type_descriptor(file, parameter)?.utf16_units);
    }
    units.push(DescriptorTag::ParameterListEnd.unit());
    units.extend_from_slice(&exact_type_descriptor(file, prototype.return_type)?.utf16_units);
    Ok(units)
}

fn exact_type_descriptor(file: &DexFile, index: TypeIndex) -> Result<&DexString> {
    let descriptor = file.resolve_type(index)?.descriptor;
    exact_string_value(file, descriptor)
}

fn exact_string(file: &DexFile, index: StringIndex) -> Result<JavaText> {
    Ok(exact_text(exact_string_value(file, index)?))
}

fn exact_string_value(file: &DexFile, index: StringIndex) -> Result<&DexString> {
    file.resolve_string(index).map_err(Error::from)
}

fn exact_text(value: &DexString) -> JavaText {
    JavaText::from_utf16(value.utf16_units.clone())
}

#[cfg(test)]
mod tests {
    use super::{binding_report_in_apk, native_methods, native_methods_in_apk};
    use crate::descriptor::NativeType;
    use crate::method::InvocationKind;
    use ::dex::apk::{ApkFile, DexOrdinal};
    use ::dex::file::{
        AccessFlags, AnnotationDirectory, ClassData, ClassDefinition, DexFile, DexString,
        DexVersion, EncodedMethod, MethodId, PrototypeId, TypeId,
    };

    #[test]
    fn extracts_typed_native_dex_declarations() {
        let file = native_dex("sample/Native", "read");

        let methods = native_methods(&file).unwrap();
        let method = &methods.as_slice()[0];

        assert_eq!(method.owner().as_str(), "sample/Native");
        assert_eq!(method.invocation(), InvocationKind::Static);
        assert_eq!(method.prototype().return_type(), NativeType::Int);
        assert_eq!(
            method.prototype().parameters()[2].native_type(),
            NativeType::Int
        );
    }

    #[test]
    fn streams_native_declarations_across_multidex_members() {
        let mut apk = ApkFile::new();
        apk.put_dex(DexOrdinal::PRIMARY, &native_dex("sample/First", "read"))
            .unwrap();
        apk.put_dex(
            DexOrdinal::new(2).expect("test ordinal is nonzero"),
            &native_dex("sample/Second", "write"),
        )
        .unwrap();

        let methods = native_methods_in_apk(&apk).unwrap();

        assert_eq!(methods.len(), 2);
        assert_eq!(methods.as_slice()[0].owner().as_str(), "sample/First");
        assert_eq!(methods.as_slice()[1].owner().as_str(), "sample/Second");
        let report = binding_report_in_apk(&apk, "application.apk").unwrap();
        let crate::report::NativeOrigin::ApkDex { entry, ordinal, .. } =
            report.declarations()[1].origin()
        else {
            panic!("expected APK provenance")
        };
        assert_eq!(entry, "classes2.dex");
        assert_eq!(ordinal.get(), 2);
    }

    fn native_dex(owner: &str, method_name: &str) -> DexFile {
        let mut file = DexFile::new(DexVersion::V035);
        let int_descriptor = file.push_string(DexString::new("I")).unwrap();
        let shorty = file.push_string(DexString::new("II")).unwrap();
        let owner_descriptor_text = format!("L{owner};");
        let owner_descriptor = file
            .push_string(DexString::new(&owner_descriptor_text))
            .unwrap();
        let name = file.push_string(DexString::new(method_name)).unwrap();
        let int_type = file
            .push_type(TypeId {
                descriptor: int_descriptor,
            })
            .unwrap();
        let owner = file
            .push_type(TypeId {
                descriptor: owner_descriptor,
            })
            .unwrap();
        let prototype = file
            .push_prototype(PrototypeId {
                shorty,
                return_type: int_type,
                parameters: vec![int_type],
                parameters_offset: 0,
            })
            .unwrap();
        let method = file
            .push_method(MethodId {
                class: owner,
                prototype,
                name,
            })
            .unwrap();
        let native_static =
            AccessFlags::from_bits_retain(AccessFlags::NATIVE.bits() | AccessFlags::STATIC.bits());
        file.push_class(ClassDefinition {
            class: owner,
            access_flags: AccessFlags::PUBLIC,
            superclass: None,
            interfaces: Vec::new(),
            source_file: None,
            annotations: AnnotationDirectory::default(),
            class_data: Some(ClassData {
                static_fields: Vec::new(),
                instance_fields: Vec::new(),
                direct_methods: vec![EncodedMethod {
                    method,
                    access_flags: native_static,
                    code: None,
                }],
                virtual_methods: Vec::new(),
                data_offset: 0,
            }),
            static_values: Vec::new(),
            definition_index: 0,
        })
        .unwrap();
        file
    }
}

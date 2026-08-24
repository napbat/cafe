//! Extraction of native declarations from DEX files and APK multidex sets.

use ::dex::apk::ApkFile;
use ::dex::file::{
    AccessFlags, DexFile, DexString, EncodedMethod, PrototypeIndex, StringIndex, TypeIndex,
};

use crate::binding::NativeMethods;
use crate::descriptor::{DescriptorTag, MethodDescriptor};
use crate::method::{InvocationKind, NativeMethod};
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
    for class in file.classes() {
        let Some(data) = &class.class_data else {
            continue;
        };
        for declaration in data.direct_methods.iter().chain(&data.virtual_methods) {
            if declaration.access_flags.contains(AccessFlags::NATIVE) {
                methods.insert(native_method(file, declaration)?)?;
            }
        }
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
    for artifact in apk.read_all_dex()? {
        methods.extend(native_methods(&artifact.file)?)?;
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
    use super::native_methods;
    use crate::descriptor::NativeType;
    use crate::method::InvocationKind;
    use ::dex::file::{
        AccessFlags, AnnotationDirectory, ClassData, ClassDefinition, DexFile, DexString,
        DexVersion, EncodedMethod, MethodId, PrototypeId, TypeId,
    };

    #[test]
    fn extracts_typed_native_dex_declarations() {
        let mut file = DexFile::new(DexVersion::V035);
        let name = file.push_string(DexString::new("read")).unwrap();
        let owner_descriptor = file.push_string(DexString::new("Lsample/Native;")).unwrap();
        let int_descriptor = file.push_string(DexString::new("I")).unwrap();
        let shorty = file.push_string(DexString::new("II")).unwrap();
        let owner = file
            .push_type(TypeId {
                descriptor: owner_descriptor,
            })
            .unwrap();
        let int_type = file
            .push_type(TypeId {
                descriptor: int_descriptor,
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
}

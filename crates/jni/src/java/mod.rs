//! Extraction of native declarations from JVM class files.

use ::java::classfile::{ClassFile, Constant, MethodAccessFlags, Utf8Constant};

use crate::Result;
use crate::binding::NativeMethods;
use crate::descriptor::MethodDescriptor;
use crate::method::{InvocationKind, NativeMethod};
use crate::text::JavaText;

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
    use super::native_methods;
    use crate::method::InvocationKind;
    use crate::symbol::SymbolStyle;
    use ::java::classfile::{ClassAccessFlags, ClassFile, JAVA_8_MAJOR_VERSION, MethodAccessFlags};

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
}

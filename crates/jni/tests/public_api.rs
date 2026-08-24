//! Public JNI API integration tests.

use jni::{InvocationKind, NativeMethod, NativeMethods, NativeType, SymbolStyle};

#[test]
fn root_api_builds_an_overload_aware_binding_plan() -> Result<(), jni::Error> {
    let methods = NativeMethods::from_methods([
        NativeMethod::new(
            "sample/Native",
            "convert",
            "([B)Ljava/lang/String;",
            InvocationKind::Static,
        )?,
        NativeMethod::new(
            "sample/Native",
            "convert",
            "(Ljava/lang/String;)[B",
            InvocationKind::Static,
        )?,
    ])?;

    let bindings = methods.bindings()?;
    assert!(
        bindings
            .iter()
            .all(|binding| binding.style() == SymbolStyle::Long)
    );
    assert_eq!(
        bindings[0].method().prototype().return_type(),
        NativeType::String
    );
    assert_eq!(
        bindings[1].method().prototype().return_type(),
        NativeType::ByteArray
    );
    assert_eq!(
        bindings[0].symbol().as_str(),
        "Java_sample_Native_convert___3B"
    );
    Ok(())
}

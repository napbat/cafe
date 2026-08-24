//! Proof that a consumer can reach every Cafe capability through one crate.

use cafe::{ModuleSource, Program, cfglib, dex, disassembler, java, jni, program};

#[test]
fn exposes_every_public_layer_through_cafe() -> Result<(), Box<dyn std::error::Error>> {
    let mut class = java::classfile::ClassFile::new(
        java::classfile::JAVA_8_MAJOR_VERSION,
        "sample/Native",
        Some("java/lang/Object"),
        java::classfile::ClassAccessFlags::PUBLIC,
    )?;
    class.add_method(
        java::classfile::MethodAccessFlags::PUBLIC
            | java::classfile::MethodAccessFlags::STATIC
            | java::classfile::MethodAccessFlags::NATIVE,
        "transform",
        "([B)Ljava/lang/String;",
    )?;

    let module = class.to_module()?;
    let program = Program::from_modules([module]);
    let native_methods = jni::java::native_methods(&class)?;
    let bindings = native_methods.bindings()?;
    let empty_dex = dex::DexFile::new(dex::DexVersion::V040);
    let mut apk = dex::apk::ApkFile::new();
    apk.put_dex(dex::apk::DexOrdinal::PRIMARY, &empty_dex)?;
    let mut visited_dex = false;
    apk.visit_dex(
        |_| true,
        |artifact| -> dex::Result<dex::apk::DexVisitControl> {
            assert_eq!(artifact.file.version(), dex::DexVersion::V040);
            visited_dex = true;
            Ok(dex::apk::DexVisitControl::Stop)
        },
    )?;
    let mut jar = java::jar::JarFile::new();
    jar.add_class(&class)?;
    let mut visited_class = false;
    jar.visit_classes(
        |entry| entry.name == "sample/Native.class",
        |entry, parsed| -> java::Result<java::jar::ClassVisitControl> {
            assert_eq!(entry.name, "sample/Native.class");
            assert_eq!(parsed.class_name()?, "sample/Native");
            visited_class = true;
            Ok(java::jar::ClassVisitControl::Stop)
        },
    )?;

    assert_eq!(program.module_count(), 1);
    assert_eq!(bindings.len(), 1);
    assert!(visited_class);
    assert!(visited_dex);
    assert_eq!(
        bindings[0].symbol().as_str(),
        "Java_sample_Native_transform"
    );
    assert_eq!(empty_dex.version(), dex::DexVersion::V040);
    assert_eq!(
        cafe::BinaryFormat::JavaClass,
        disassembler::BinaryFormat::JavaClass
    );

    let _: program::Program = program;
    let _ = std::any::type_name::<java::jar::JarFile>();
    let _ = std::any::type_name::<dex::apk::ApkFile>();
    let _ = std::any::type_name::<cfglib::BlockId>();
    Ok(())
}

//! Malformed class-file rejection and diagnostic-context coverage.

use std::panic::{AssertUnwindSafe, catch_unwind};

use java::bytecode::{Instruction, Opcode, Operand};
use java::classfile::{
    Attribute, ClassAccessFlags, ClassFile, CodeAttribute, Constant, FieldAccessFlags,
    IndexAttribute, JAVA_6_MAJOR_VERSION, KnownAttribute, MethodAccessFlags,
};
use java::jar::JarFile;

fn valid_class() -> ClassFile {
    let mut class = ClassFile::new(
        61,
        "sample/Example",
        Some("java/lang/Object"),
        ClassAccessFlags::PUBLIC | ClassAccessFlags::SUPER,
    )
    .unwrap();
    class
        .add_field(FieldAccessFlags::PRIVATE, "value", "I")
        .unwrap();
    let method = class
        .add_method(
            MethodAccessFlags::PUBLIC | MethodAccessFlags::STATIC,
            "run",
            "()V",
        )
        .unwrap();
    let code = CodeAttribute::new(
        &mut class.constant_pool,
        0,
        0,
        &[Instruction::new(0, Opcode::Return, Operand::None)],
    )
    .unwrap();
    class.methods[method].set_code(code);
    class
}

#[test]
fn preserves_the_legacy_vm_rule_for_interface_abstract_flags() {
    let class = ClassFile::new(
        JAVA_6_MAJOR_VERSION - 1,
        "legacy/package-info",
        Some("java/lang/Object"),
        ClassAccessFlags::INTERFACE,
    )
    .unwrap();
    let bytes = class.to_bytes().unwrap();
    let parsed = ClassFile::parse(&bytes).unwrap();
    assert_eq!(parsed.to_bytes().unwrap(), bytes);
    assert!(parsed.access_flags.contains(ClassAccessFlags::INTERFACE));
    assert!(!parsed.access_flags.contains(ClassAccessFlags::ABSTRACT));

    let mut modern = parsed;
    modern.major_version = JAVA_6_MAJOR_VERSION;
    assert!(
        modern
            .validate()
            .unwrap_err()
            .to_string()
            .contains("invalid interface access-flag combination")
    );
}

#[test]
fn every_truncated_prefix_is_rejected_without_panicking() {
    let bytes = valid_class().to_bytes().unwrap();
    for end in 0..bytes.len() {
        let outcome = catch_unwind(AssertUnwindSafe(|| ClassFile::parse(&bytes[..end])));
        assert!(outcome.is_ok(), "parser panicked at truncated length {end}");
        assert!(
            outcome.unwrap().is_err(),
            "parser accepted truncated length {end}"
        );
    }
}

#[test]
fn reports_exact_offsets_for_invalid_tags_and_bytecode_targets() {
    let mut bytes = valid_class().to_bytes().unwrap();
    bytes[10] = 0xff;
    let error = ClassFile::parse(&bytes).unwrap_err();
    assert!(error.to_string().contains("byte 10"));
    assert!(error.to_string().contains("constant-pool tag 255"));

    let mut class = valid_class();
    class.methods[0].code_mut().unwrap().code = vec![Opcode::Goto.byte(), 0, 1];
    let error = class.validate().unwrap_err();
    let message = error.to_string();
    assert!(message.contains("class `sample/Example`"));
    assert!(message.contains("method `run()V`"));
    assert!(message.contains("bytecode at offset 0"));
    assert!(message.contains("inside another instruction"));
}

#[test]
fn rejects_invalid_versions_references_descriptors_and_attributes() {
    let mut class = valid_class();
    class.major_version = 71;
    assert!(
        class
            .validate()
            .unwrap_err()
            .to_string()
            .contains("version 71")
    );

    let mut class = valid_class();
    let utf8 = class.constant_pool.intern_utf8("not-a-class").unwrap();
    class.this_class = utf8;
    assert!(
        class
            .validate()
            .unwrap_err()
            .to_string()
            .contains("expected Class")
    );

    let mut class = valid_class();
    let invalid_descriptor = class.constant_pool.intern_utf8("Q").unwrap();
    class.methods[0].descriptor_index = invalid_descriptor;
    let descriptor_error = class.validate().unwrap_err().to_string();
    assert!(descriptor_error.contains("method `runQ`"));
    assert!(descriptor_error.contains("invalid JVM descriptor"));

    let mut class = valid_class();
    let source_name = class.constant_pool.intern_utf8("SourceFile").unwrap();
    let source_value = class.constant_pool.intern_utf8("Example.java").unwrap();
    class.methods[0]
        .attributes
        .push(Attribute::Known(KnownAttribute::SourceFile(
            IndexAttribute {
                name_index: source_name,
                index: source_value,
            },
        )));
    let location_error = class.validate().unwrap_err().to_string();
    assert!(location_error.contains("method `run()V`"));
    assert!(location_error.contains("invalid at Method location"));
}

#[test]
fn rejects_bad_exception_and_bootstrap_references() {
    let mut class = valid_class();
    let handler = &mut class.methods[0].code_mut().unwrap().exception_table;
    handler.push(java::classfile::ExceptionHandler {
        start_pc: 0,
        end_pc: 0,
        handler_pc: 0,
        catch_type: 0,
    });
    let error = class.validate().unwrap_err().to_string();
    assert!(error.contains("method `run()V`"));
    assert!(error.contains("exception range start 0 is not before end 0"));

    let mut class = valid_class();
    let name_and_type_index = class
        .constant_pool
        .intern_name_and_type("value", "I")
        .unwrap();
    class
        .constant_pool
        .intern(Constant::Dynamic {
            bootstrap_method_attr_index: 0,
            name_and_type_index,
        })
        .unwrap();
    assert!(
        class
            .validate()
            .unwrap_err()
            .to_string()
            .contains("missing bootstrap method 0")
    );
}

#[test]
fn jar_failures_include_entry_class_method_and_byte_offset() {
    let mut bytes = valid_class().to_bytes().unwrap();
    let code_payload = [
        0,
        0,
        0,
        13, // attribute length
        0,
        0, // max_stack
        0,
        0, // max_locals
        0,
        0,
        0,
        1, // code length
        Opcode::Return.byte(),
        0,
        0, // exception count
        0,
        0, // nested attribute count
    ];
    let start = bytes
        .windows(code_payload.len())
        .position(|window| window == code_payload)
        .unwrap();
    bytes[start + 12] = 0xcb;

    let mut jar = JarFile::new();
    jar.add_file("sample/Example.class", bytes).unwrap();
    let message = jar.validate_all().unwrap_err().to_string();
    assert!(message.contains("sample/Example.class"));
    assert!(message.contains("class `sample/Example`"));
    assert!(message.contains("method `run()V`"));
    assert!(message.contains("bytecode at offset 0"));
}

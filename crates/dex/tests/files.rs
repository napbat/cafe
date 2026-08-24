//! Logical DEX file construction, canonical assembly, and integrity tests.

use dex::file::{
    AccessFlags, AnnotationDirectory, ClassData, ClassDefinition, CodeItem, DexFile, DexString,
    DexVersion, EncodedMethod, MethodId, PrototypeId, TypeId,
};
use dex::instruction::{Instruction, Opcode, Operands};

fn minimal_file(version: DexVersion) -> DexFile {
    let mut file = DexFile::new(version);
    let constructor_name = file.push_string(DexString::new("<init>")).unwrap();
    let example_descriptor = file.push_string(DexString::new("LExample;")).unwrap();
    let object_descriptor = file
        .push_string(DexString::new("Ljava/lang/Object;"))
        .unwrap();
    let void_descriptor = file.push_string(DexString::new("V")).unwrap();

    let example_type = file
        .push_type(TypeId {
            descriptor: example_descriptor,
        })
        .unwrap();
    let object_type = file
        .push_type(TypeId {
            descriptor: object_descriptor,
        })
        .unwrap();
    let void_type = file
        .push_type(TypeId {
            descriptor: void_descriptor,
        })
        .unwrap();
    let prototype = file
        .push_prototype(PrototypeId {
            shorty: void_descriptor,
            return_type: void_type,
            parameters: Vec::new(),
            parameters_offset: 0,
        })
        .unwrap();
    let method = file
        .push_method(MethodId {
            class: example_type,
            prototype,
            name: constructor_name,
        })
        .unwrap();
    let constructor_flags =
        AccessFlags::from_bits_retain(AccessFlags::PUBLIC.bits() | AccessFlags::CONSTRUCTOR.bits());
    file.push_class(ClassDefinition {
        class: example_type,
        access_flags: AccessFlags::PUBLIC,
        superclass: Some(object_type),
        interfaces: Vec::new(),
        source_file: None,
        annotations: AnnotationDirectory::default(),
        class_data: Some(ClassData {
            static_fields: Vec::new(),
            instance_fields: Vec::new(),
            direct_methods: vec![EncodedMethod {
                method,
                access_flags: constructor_flags,
                code: Some(CodeItem {
                    registers_size: 1,
                    ins_size: 1,
                    outs_size: 0,
                    instructions: vec![Instruction::operation(
                        0,
                        Opcode::ReturnVoid,
                        Operands::None,
                    )],
                    tries: Vec::new(),
                    debug_info: None,
                    data_offset: 0,
                }),
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

#[test]
fn assembles_and_reparses_empty_supported_files() {
    for version in DexVersion::ALL
        .iter()
        .copied()
        .filter(|version| *version != DexVersion::V041)
    {
        let bytes = DexFile::new(version).to_bytes().unwrap();
        let parsed = DexFile::parse(&bytes).unwrap();
        assert_eq!(parsed.version(), version);
        assert_eq!(parsed.to_bytes().unwrap(), bytes);
        assert!(parsed.is_pristine());
    }
}

#[test]
fn canonical_writer_preserves_complete_edited_models() {
    let file = minimal_file(DexVersion::V040);
    let bytes = file.to_bytes().unwrap();
    let parsed = DexFile::parse(&bytes).unwrap();

    assert_eq!(parsed.strings().len(), file.strings().len());
    assert_eq!(parsed.types(), file.types());
    assert_eq!(parsed.prototypes(), file.prototypes());
    assert_eq!(parsed.methods(), file.methods());
    assert_eq!(parsed.classes().len(), 1);
    assert_eq!(parsed.to_bytes().unwrap(), bytes);
}

#[test]
fn checked_edits_roll_back_on_invalid_table_order() {
    let mut file = minimal_file(DexVersion::V040);
    let before = file.to_bytes().unwrap();
    let error = file
        .try_edit(|file| {
            file.push_string(DexString::new("A"))?;
            Ok(())
        })
        .unwrap_err();

    assert!(error.to_string().contains("not strictly ordered"));
    assert_eq!(file.to_bytes().unwrap(), before);
}

#[test]
fn integrity_corruption_is_rejected_contextually() {
    let mut bytes = minimal_file(DexVersion::V040).to_bytes().unwrap();
    let last = bytes.last_mut().unwrap();
    *last ^= 1;

    let error = DexFile::parse(&bytes).unwrap_err();
    assert!(error.to_string().contains("SHA-1 signature"));
}

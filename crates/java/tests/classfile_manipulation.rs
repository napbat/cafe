//! Transactional JVM method-body manipulation coverage.

use java::bytecode::{Instruction, Opcode, Operand, decode, encode};
use java::classfile::{
    Annotation, Attribute, BytecodeOffsetMap, ClassAccessFlags, ClassFile, CodeAttribute,
    ExceptionHandler, FieldAccessFlags, KnownAttribute, KnownAttributeKind, LineNumber,
    LineNumberTableAttribute, LocalVariable, LocalVariableTableAttribute, MarkerAttribute,
    MethodAccessFlags, StackMapFrame, StackMapTableAttribute, TypeAnnotation, TypeAnnotationTarget,
    TypeAnnotationsAttribute, VerificationType,
};

fn original_code() -> CodeAttribute {
    CodeAttribute {
        name_index: 1,
        max_stack: 1,
        max_locals: 1,
        code: vec![
            Opcode::IConst0.byte(),
            Opcode::New.byte(),
            0,
            1,
            Opcode::Pop.byte(),
            Opcode::Return.byte(),
        ],
        exception_table: vec![ExceptionHandler {
            start_pc: 0,
            end_pc: 5,
            handler_pc: 5,
            catch_type: 0,
        }],
        attributes: vec![
            Attribute::Known(KnownAttribute::LineNumberTable(LineNumberTableAttribute {
                name_index: 2,
                lines: vec![LineNumber {
                    start_pc: 1,
                    line_number: 42,
                }],
            })),
            Attribute::Known(KnownAttribute::LocalVariableTable(
                LocalVariableTableAttribute {
                    name_index: 3,
                    variables: vec![LocalVariable {
                        start_pc: 0,
                        length: 6,
                        name_index: 4,
                        descriptor_index: 5,
                        slot: 0,
                    }],
                },
            )),
            Attribute::Known(KnownAttribute::StackMapTable(StackMapTableAttribute {
                name_index: 6,
                frames: vec![StackMapFrame::Full {
                    offset_delta: 5,
                    locals: vec![VerificationType::Uninitialized(1)],
                    stack: Vec::new(),
                }],
            })),
            Attribute::Known(KnownAttribute::RuntimeVisibleTypeAnnotations(
                TypeAnnotationsAttribute {
                    name_index: 7,
                    annotations: vec![TypeAnnotation {
                        target: TypeAnnotationTarget::New(1),
                        path: Vec::new(),
                        annotation: Annotation {
                            type_index: 8,
                            elements: Vec::new(),
                        },
                    }],
                },
            )),
        ],
    }
}

fn replacement_instructions() -> Vec<Instruction> {
    vec![
        Instruction::new(0, Opcode::SiPush, Operand::Short(0)),
        Instruction::new(3, Opcode::New, Operand::Constant(1)),
        Instruction::new(6, Opcode::Pop, Operand::None),
        Instruction::new(7, Opcode::Return, Operand::None),
    ]
}

#[test]
fn refuses_to_stale_offset_sensitive_metadata() {
    let mut code = original_code();
    let original = code.clone();
    let error = code
        .set_instructions(&replacement_instructions())
        .unwrap_err();
    assert!(error.to_string().contains("offset-sensitive metadata"));
    assert_eq!(code, original);
}

#[test]
fn remaps_every_modeled_code_offset_transactionally() {
    let mut code = original_code();
    let replacement = replacement_instructions();
    let old = decode(&code.code).unwrap();
    let new_length = encode(&replacement).unwrap().len();
    let offsets =
        BytecodeOffsetMap::from_instruction_pairs(&old, code.code.len(), &replacement, new_length)
            .unwrap();

    code.set_instructions_with_offset_map(&replacement, &offsets)
        .unwrap();
    assert_eq!(code.code.len(), 8);
    assert_eq!(code.exception_table[0].end_pc, 7);
    assert_eq!(code.exception_table[0].handler_pc, 7);

    let Attribute::Known(KnownAttribute::LineNumberTable(lines)) = &code.attributes[0] else {
        panic!("line-number table was not retained")
    };
    assert_eq!(lines.lines[0].start_pc, 3);

    let Attribute::Known(KnownAttribute::LocalVariableTable(locals)) = &code.attributes[1] else {
        panic!("local-variable table was not retained")
    };
    assert_eq!(locals.variables[0].length, 8);

    let Attribute::Known(KnownAttribute::StackMapTable(stack_maps)) = &code.attributes[2] else {
        panic!("stack-map table was not retained")
    };
    let StackMapFrame::Full {
        offset_delta,
        locals,
        ..
    } = &stack_maps.frames[0]
    else {
        panic!("full frame changed category")
    };
    assert_eq!(*offset_delta, 7);
    assert_eq!(locals, &[VerificationType::Uninitialized(3)]);

    let Attribute::Known(KnownAttribute::RuntimeVisibleTypeAnnotations(annotations)) =
        &code.attributes[3]
    else {
        panic!("type annotations were not retained")
    };
    assert_eq!(
        annotations.annotations[0].target,
        TypeAnnotationTarget::New(3)
    );
}

#[test]
fn explicitly_drops_all_code_metadata() {
    let mut code = original_code();
    code.set_instructions_dropping_metadata(&replacement_instructions())
        .unwrap();
    assert!(code.exception_table.is_empty());
    assert!(code.attributes.is_empty());
    assert_eq!(code.instructions().unwrap().len(), 4);
}

#[test]
fn builds_and_queries_an_editable_class_without_manual_indices() {
    let mut class = ClassFile::new(
        61,
        "sample/Example",
        Some("java/lang/Object"),
        ClassAccessFlags::PUBLIC | ClassAccessFlags::SUPER,
    )
    .unwrap();
    let first = class.constant_pool.intern_utf8("shared").unwrap();
    assert_eq!(class.constant_pool.intern_utf8("shared").unwrap(), first);

    class
        .add_field(FieldAccessFlags::PUBLIC, "value", "I")
        .unwrap();
    let method_index = class
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
    class.methods[method_index].set_code(code);

    let synthetic_name = class.constant_pool.intern_utf8("Synthetic").unwrap();
    class.set_attribute(Attribute::Known(KnownAttribute::Synthetic(
        MarkerAttribute {
            name_index: synthetic_name,
        },
    )));
    assert!(
        class
            .known_attribute(KnownAttributeKind::Synthetic)
            .is_some()
    );
    assert!(class.field("value", "I").unwrap().is_some());
    assert!(class.method("run", "()V").unwrap().is_some());

    let bytes = class.to_bytes().unwrap();
    let reparsed = ClassFile::parse(&bytes).unwrap();
    assert_eq!(reparsed.class_name().unwrap(), "sample/Example");
    assert_eq!(
        reparsed.methods[0]
            .code()
            .unwrap()
            .instructions()
            .unwrap()
            .len(),
        1
    );
}

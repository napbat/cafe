use crate::analysis::{
    ClassHierarchy, FrameValue, InstructionReference, LoadableConstant, analyze_code,
    analyze_code_with_hierarchy, resolve_instruction_reference,
};
use crate::bytecode::{CatchTarget, CodeBuilder, LocalKind, Opcode, Operand};
use crate::classfile::{CodeAttribute, Constant, ConstantPool, MethodAccessFlags};

#[test]
fn computes_exact_stack_and_local_requirements() {
    let mut builder = CodeBuilder::new();
    let _ = builder.emit_load(LocalKind::Integer, 0);
    let _ = builder.emit(Opcode::IConst1, Operand::None);
    let _ = builder.emit(Opcode::IAdd, Operand::None);
    let _ = builder.emit(Opcode::IReturn, Operand::None);
    let built = builder.finish().unwrap();
    let mut pool = ConstantPool::new();
    let code = CodeAttribute::from_built(&mut pool, 0, 0, &built).unwrap();

    let analysis = analyze_code(
        &pool,
        "Example",
        "increment",
        "(I)I",
        MethodAccessFlags::STATIC,
        &code,
    )
    .unwrap();
    assert_eq!(analysis.max_stack(), 2);
    assert_eq!(analysis.max_locals(), 1);
    assert_eq!(
        analysis.entry_frame(0).unwrap().locals(),
        &[FrameValue::Integer]
    );
}

#[test]
fn exception_handlers_receive_a_single_caught_reference() {
    let mut builder = CodeBuilder::new();
    let start = builder.new_label();
    let end = builder.new_label();
    let handler = builder.new_label();
    builder.bind(start).unwrap();
    let _ = builder.emit(Opcode::AConstNull, Operand::None);
    let _ = builder.emit(Opcode::AThrow, Operand::None);
    builder.bind(end).unwrap();
    builder.bind(handler).unwrap();
    let _ = builder.emit(Opcode::AReturn, Operand::None);
    builder
        .add_exception_handler(start, end, handler, CatchTarget::Any)
        .unwrap();
    let built = builder.finish().unwrap();
    let handler_offset = built.label_offset(handler).unwrap();
    let mut pool = ConstantPool::new();
    let mut code = CodeAttribute::from_built(&mut pool, 0, 0, &built).unwrap();
    let analysis = analyze_code(
        &pool,
        "Example",
        "caught",
        "()Ljava/lang/Throwable;",
        MethodAccessFlags::STATIC,
        &code,
    )
    .unwrap();
    assert_eq!(
        analysis.entry_frame(handler_offset).unwrap().stack(),
        &[FrameValue::Reference("java/lang/Throwable".to_owned())]
    );
    analysis.apply_to_code(&mut pool, &mut code).unwrap();
    assert_eq!(code.max_stack, 1);
    assert!(code.attributes.iter().any(|attribute| {
        matches!(
            attribute,
            crate::classfile::Attribute::Known(crate::classfile::KnownAttribute::StackMapTable(_))
        )
    }));
}

#[test]
fn constructor_invocation_initializes_every_allocation_alias() {
    let mut pool = ConstantPool::new();
    let class = pool.intern_class("java/lang/Object").unwrap();
    let constructor = pool
        .intern_method_ref("java/lang/Object", "<init>", "()V")
        .unwrap();
    let mut builder = CodeBuilder::new();
    let _ = builder.emit(Opcode::New, Operand::Constant(class));
    let _ = builder.emit(Opcode::Dup, Operand::None);
    let _ = builder.emit(Opcode::InvokeSpecial, Operand::Constant(constructor));
    let _ = builder.emit(Opcode::AReturn, Operand::None);
    let built = builder.finish().unwrap();
    let code = CodeAttribute::from_built(&mut pool, 0, 0, &built).unwrap();

    let analysis = analyze_code(
        &pool,
        "Example",
        "create",
        "()Ljava/lang/Object;",
        MethodAccessFlags::STATIC,
        &code,
    )
    .unwrap();
    assert_eq!(analysis.max_stack(), 2);
    assert_eq!(
        analysis.entry_frame(7).unwrap().stack(),
        &[FrameValue::Reference("java/lang/Object".to_owned())]
    );
}

#[test]
fn caller_hierarchy_checks_reference_assignment() {
    let mut pool = ConstantPool::new();
    let field = pool
        .intern_field_ref("sample/Owner", "value", "Lsample/Base;")
        .unwrap();
    let mut builder = CodeBuilder::new();
    let _ = builder.emit_load(LocalKind::Reference, 0);
    let _ = builder.emit(Opcode::PutStatic, Operand::Constant(field));
    let _ = builder.emit(Opcode::Return, Operand::None);
    let built = builder.finish().unwrap();
    let code = CodeAttribute::from_built(&mut pool, 0, 0, &built).unwrap();

    let mut hierarchy = ClassHierarchy::new();
    hierarchy.insert("sample/Base", Some("java/lang/Object"), [] as [&str; 0]);
    hierarchy.insert("sample/Sub", Some("sample/Base"), [] as [&str; 0]);
    let analysis = analyze_code_with_hierarchy(
        &pool,
        "sample/Owner",
        "store",
        "(Lsample/Sub;)V",
        MethodAccessFlags::STATIC,
        &code,
        &hierarchy,
    )
    .unwrap();
    assert_eq!(analysis.max_stack(), 1);

    let unrelated = ClassHierarchy::new();
    assert!(
        analyze_code_with_hierarchy(
            &pool,
            "sample/Owner",
            "store",
            "(Lsample/Sub;)V",
            MethodAccessFlags::STATIC,
            &code,
            &unrelated,
        )
        .is_err()
    );
}

#[test]
fn instruction_reference_resolution_preserves_exact_java_strings() {
    let mut pool = ConstantPool::new();
    let odd_units = vec![0xd800, u16::from(b'x')];
    let string_index = pool.intern_utf16(odd_units.clone()).unwrap();
    let string = pool.intern(Constant::String { string_index }).unwrap();
    let loaded = resolve_instruction_reference(
        &pool,
        &crate::bytecode::Instruction::new(7, Opcode::Ldc, Operand::Constant(string)),
    )
    .unwrap();
    let Some(InstructionReference::Constant(LoadableConstant::String(value))) = loaded else {
        panic!("expected a resolved Java string");
    };
    assert_eq!(value.utf16_units, odd_units);

    let owner = pool.intern_class("sample/Owner").unwrap();
    let name_index = pool.intern_utf16(vec![0xd801]).unwrap();
    let descriptor_index = pool.intern_utf8("I").unwrap();
    let name_and_type = pool
        .intern(Constant::NameAndType {
            name_index,
            descriptor_index,
        })
        .unwrap();
    let field = pool
        .intern(Constant::FieldRef {
            class_index: owner,
            name_and_type_index: name_and_type,
        })
        .unwrap();
    let resolved = resolve_instruction_reference(
        &pool,
        &crate::bytecode::Instruction::new(9, Opcode::GetStatic, Operand::Constant(field)),
    )
    .unwrap();
    let Some(InstructionReference::Field(field)) = resolved else {
        panic!("expected a resolved field");
    };
    assert_eq!(field.name.utf16_units, vec![0xd801]);
    assert_eq!(field.descriptor, "I");
}

#[test]
fn constructors_must_initialize_their_receiver_before_returning() {
    let mut builder = CodeBuilder::new();
    let _ = builder.emit(Opcode::Return, Operand::None);
    let built = builder.finish().unwrap();
    let mut pool = ConstantPool::new();
    let code = CodeAttribute::from_built(&mut pool, 0, 0, &built).unwrap();
    let error = analyze_code(
        &pool,
        "sample/Incomplete",
        "<init>",
        "()V",
        MethodAccessFlags::PUBLIC,
        &code,
    )
    .unwrap_err();
    assert!(error.to_string().contains("before initializing"));
}

#[test]
fn array_operations_check_the_component_category() {
    let mut builder = CodeBuilder::new();
    let _ = builder.emit_load(LocalKind::Reference, 0);
    let _ = builder.emit(Opcode::IConst0, Operand::None);
    let _ = builder.emit(Opcode::AALoad, Operand::None);
    let _ = builder.emit(Opcode::AReturn, Operand::None);
    let built = builder.finish().unwrap();
    let mut pool = ConstantPool::new();
    let code = CodeAttribute::from_built(&mut pool, 0, 0, &built).unwrap();
    assert!(
        analyze_code(
            &pool,
            "sample/Arrays",
            "bad",
            "([I)Ljava/lang/Object;",
            MethodAccessFlags::STATIC,
            &code,
        )
        .is_err()
    );
    let valid = analyze_code(
        &pool,
        "sample/Arrays",
        "first",
        "([Ljava/lang/String;)Ljava/lang/Object;",
        MethodAccessFlags::STATIC,
        &code,
    )
    .unwrap();
    assert_eq!(
        valid.entry_frame(3).unwrap().stack(),
        &[FrameValue::Reference("java/lang/String".to_owned())]
    );
}

#[test]
fn invokeinterface_count_is_recomputed_from_the_descriptor() {
    let mut pool = ConstantPool::new();
    let method = pool
        .intern_interface_method_ref("sample/Contract", "run", "()V")
        .unwrap();
    let mut builder = CodeBuilder::new();
    let _ = builder.emit_load(LocalKind::Reference, 0);
    let _ = builder.emit(
        Opcode::InvokeInterface,
        Operand::InvokeInterface {
            index: method,
            count: 2,
        },
    );
    let _ = builder.emit(Opcode::Return, Operand::None);
    let built = builder.finish().unwrap();
    let code = CodeAttribute::from_built(&mut pool, 0, 0, &built).unwrap();
    let error = analyze_code(
        &pool,
        "sample/Caller",
        "call",
        "(Lsample/Contract;)V",
        MethodAccessFlags::STATIC,
        &code,
    )
    .unwrap_err();
    assert!(error.to_string().contains("invokeinterface count"));
}

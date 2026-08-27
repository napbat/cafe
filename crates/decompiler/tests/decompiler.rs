//! End-to-end class-file to Java-source decompilation coverage.

use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use decompiler::decompile_class;
use java::bytecode::{CatchTarget, CodeBuilder, LocalKind, Opcode, Operand};
use java::classfile::{
    Attribute, ClassAccessFlags, ClassFile, CodeAttribute, FieldAccessFlags, InnerClass,
    InnerClassAccessFlags, InnerClassesAttribute, JAVA_8_MAJOR_VERSION, KnownAttribute,
    KnownAttributeKind, MethodAccessFlags,
};

const OWNER: &str = "sample/Arithmetic";

type TestResult<T> = Result<T, Box<dyn std::error::Error>>;

#[derive(Clone, Copy)]
struct FixtureReferences {
    object_constructor: u16,
    owner_class: u16,
    owner_constructor: u16,
    twice_method: u16,
    string_class: u16,
    counter_field: u16,
    flag_field: u16,
    seed_field: u16,
    always_true_method: u16,
}

fn fixture() -> TestResult<ClassFile> {
    let mut class = ClassFile::new(
        JAVA_8_MAJOR_VERSION,
        OWNER,
        Some("java/lang/Object"),
        ClassAccessFlags::PUBLIC | ClassAccessFlags::SUPER,
    )?;
    let references = fixture_references(&mut class)?;
    add_construction_methods(&mut class, &references)?;
    add_type_methods(&mut class, &references)?;
    add_field_and_boolean_methods(&mut class, &references)?;
    add_bridge_methods(&mut class)?;
    add_array_methods(&mut class)?;
    add_exception_and_switch_methods(&mut class)?;
    add_arithmetic_and_control_methods(&mut class)?;
    class.validate()?;
    Ok(class)
}

fn add_bridge_methods(class: &mut ClassFile) -> TestResult<()> {
    add_method(
        class,
        MethodAccessFlags::PUBLIC | MethodAccessFlags::STATIC,
        "covariant",
        "()Ljava/lang/String;",
        |code| {
            let _ = code.emit(Opcode::AConstNull, Operand::None);
            let _ = code.emit(Opcode::AReturn, Operand::None);
            Ok(())
        },
    )?;
    add_method(
        class,
        MethodAccessFlags::PUBLIC
            | MethodAccessFlags::STATIC
            | MethodAccessFlags::BRIDGE
            | MethodAccessFlags::SYNTHETIC,
        "covariant",
        "()Ljava/lang/Object;",
        |code| {
            let _ = code.emit(Opcode::AConstNull, Operand::None);
            let _ = code.emit(Opcode::AReturn, Operand::None);
            Ok(())
        },
    )?;
    Ok(())
}

fn fixture_references(class: &mut ClassFile) -> TestResult<FixtureReferences> {
    let object_constructor =
        class
            .constant_pool
            .intern_method_ref("java/lang/Object", "<init>", "()V")?;
    let owner_class = class.constant_pool.intern_class(OWNER)?;
    let owner_constructor = class
        .constant_pool
        .intern_method_ref(OWNER, "<init>", "()V")?;
    let twice_method = class
        .constant_pool
        .intern_method_ref(OWNER, "twice", "(I)I")?;
    let string_class = class.constant_pool.intern_class("java/lang/String")?;
    class.add_field(
        FieldAccessFlags::PUBLIC | FieldAccessFlags::STATIC,
        "counter",
        "I",
    )?;
    class.add_field(
        FieldAccessFlags::PUBLIC | FieldAccessFlags::STATIC,
        "flag",
        "Z",
    )?;
    class.add_field(
        FieldAccessFlags::PUBLIC | FieldAccessFlags::STATIC | FieldAccessFlags::FINAL,
        "seed",
        "I",
    )?;
    let counter_field = class
        .constant_pool
        .intern_field_ref(OWNER, "counter", "I")?;
    let flag_field = class.constant_pool.intern_field_ref(OWNER, "flag", "Z")?;
    let seed_field = class.constant_pool.intern_field_ref(OWNER, "seed", "I")?;
    let always_true_method = class
        .constant_pool
        .intern_method_ref(OWNER, "alwaysTrue", "()Z")?;

    Ok(FixtureReferences {
        object_constructor,
        owner_class,
        owner_constructor,
        twice_method,
        string_class,
        counter_field,
        flag_field,
        seed_field,
        always_true_method,
    })
}

fn add_construction_methods(
    class: &mut ClassFile,
    references: &FixtureReferences,
) -> TestResult<()> {
    add_method(class, MethodAccessFlags::PUBLIC, "<init>", "()V", |code| {
        let _ = code.emit_load(LocalKind::Reference, 0);
        let _ = code.emit(
            Opcode::InvokeSpecial,
            Operand::Constant(references.object_constructor),
        );
        let _ = code.emit(Opcode::Return, Operand::None);
        Ok(())
    })?;
    add_method(
        class,
        MethodAccessFlags::PUBLIC | MethodAccessFlags::STATIC,
        "make",
        "()Lsample/Arithmetic;",
        |code| {
            let _ = code.emit(Opcode::New, Operand::Constant(references.owner_class));
            let _ = code.emit(Opcode::Dup, Operand::None);
            let _ = code.emit(
                Opcode::InvokeSpecial,
                Operand::Constant(references.owner_constructor),
            );
            let _ = code.emit(Opcode::AReturn, Operand::None);
            Ok(())
        },
    )?;
    add_method(
        class,
        MethodAccessFlags::PUBLIC | MethodAccessFlags::STATIC,
        "callTwice",
        "(I)I",
        |code| {
            let _ = code.emit(Opcode::New, Operand::Constant(references.owner_class));
            let _ = code.emit(Opcode::Dup, Operand::None);
            let _ = code.emit(
                Opcode::InvokeSpecial,
                Operand::Constant(references.owner_constructor),
            );
            let _ = code.emit_load(LocalKind::Integer, 0);
            let _ = code.emit(
                Opcode::InvokeVirtual,
                Operand::Constant(references.twice_method),
            );
            let _ = code.emit(Opcode::IReturn, Operand::None);
            Ok(())
        },
    )?;
    add_method(class, MethodAccessFlags::PUBLIC, "twice", "(I)I", |code| {
        let _ = code.emit_load(LocalKind::Integer, 1);
        let _ = code.emit(Opcode::IConst2, Operand::None);
        let _ = code.emit(Opcode::IMul, Operand::None);
        let _ = code.emit(Opcode::IReturn, Operand::None);
        Ok(())
    })?;
    Ok(())
}

fn add_type_methods(class: &mut ClassFile, references: &FixtureReferences) -> TestResult<()> {
    add_method(
        class,
        MethodAccessFlags::PUBLIC | MethodAccessFlags::STATIC,
        "isString",
        "(Ljava/lang/Object;)Z",
        |code| {
            let _ = code.emit_load(LocalKind::Reference, 0);
            let _ = code.emit(
                Opcode::InstanceOf,
                Operand::Constant(references.string_class),
            );
            let _ = code.emit(Opcode::IReturn, Operand::None);
            Ok(())
        },
    )?;
    add_method(
        class,
        MethodAccessFlags::PUBLIC | MethodAccessFlags::STATIC,
        "asString",
        "(Ljava/lang/Object;)Ljava/lang/String;",
        |code| {
            let _ = code.emit_load(LocalKind::Reference, 0);
            let _ = code.emit(
                Opcode::CheckCast,
                Operand::Constant(references.string_class),
            );
            let _ = code.emit(Opcode::AReturn, Operand::None);
            Ok(())
        },
    )?;
    Ok(())
}

fn add_field_and_boolean_methods(
    class: &mut ClassFile,
    references: &FixtureReferences,
) -> TestResult<()> {
    add_method(
        class,
        MethodAccessFlags::STATIC,
        "<clinit>",
        "()V",
        |code| {
            let _ = code.emit(Opcode::IConst2, Operand::None);
            let _ = code.emit(Opcode::PutStatic, Operand::Constant(references.seed_field));
            let _ = code.emit(Opcode::Return, Operand::None);
            Ok(())
        },
    )?;
    add_method(
        class,
        MethodAccessFlags::PUBLIC | MethodAccessFlags::STATIC,
        "setCounter",
        "(I)V",
        |code| {
            let _ = code.emit_load(LocalKind::Integer, 0);
            let _ = code.emit(
                Opcode::PutStatic,
                Operand::Constant(references.counter_field),
            );
            let _ = code.emit(Opcode::Return, Operand::None);
            Ok(())
        },
    )?;
    add_method(
        class,
        MethodAccessFlags::PUBLIC | MethodAccessFlags::STATIC,
        "getCounter",
        "()I",
        |code| {
            let _ = code.emit(
                Opcode::GetStatic,
                Operand::Constant(references.counter_field),
            );
            let _ = code.emit(Opcode::IReturn, Operand::None);
            Ok(())
        },
    )?;
    add_method(
        class,
        MethodAccessFlags::PUBLIC | MethodAccessFlags::STATIC,
        "setFlag",
        "(Z)V",
        |code| {
            let _ = code.emit_load(LocalKind::Integer, 0);
            let _ = code.emit(Opcode::PutStatic, Operand::Constant(references.flag_field));
            let _ = code.emit(Opcode::Return, Operand::None);
            Ok(())
        },
    )?;
    add_method(
        class,
        MethodAccessFlags::PUBLIC | MethodAccessFlags::STATIC,
        "getFlag",
        "()Z",
        |code| {
            let _ = code.emit(Opcode::GetStatic, Operand::Constant(references.flag_field));
            let _ = code.emit(Opcode::IReturn, Operand::None);
            Ok(())
        },
    )?;
    add_method(
        class,
        MethodAccessFlags::PUBLIC | MethodAccessFlags::STATIC,
        "alwaysTrue",
        "()Z",
        |code| {
            let _ = code.emit(Opcode::IConst1, Operand::None);
            let _ = code.emit(Opcode::IReturn, Operand::None);
            Ok(())
        },
    )?;
    add_method(
        class,
        MethodAccessFlags::PUBLIC | MethodAccessFlags::STATIC,
        "callBoolean",
        "()Z",
        |code| {
            let _ = code.emit(
                Opcode::InvokeStatic,
                Operand::Constant(references.always_true_method),
            );
            let _ = code.emit(Opcode::IReturn, Operand::None);
            Ok(())
        },
    )?;
    Ok(())
}

fn add_array_methods(class: &mut ClassFile) -> TestResult<()> {
    add_method(
        class,
        MethodAccessFlags::PUBLIC | MethodAccessFlags::STATIC,
        "first",
        "([I)I",
        |code| {
            let _ = code.emit_load(LocalKind::Reference, 0);
            let _ = code.emit(Opcode::IConst0, Operand::None);
            let _ = code.emit(Opcode::IALoad, Operand::None);
            let _ = code.emit(Opcode::IReturn, Operand::None);
            Ok(())
        },
    )?;
    add_method(
        class,
        MethodAccessFlags::PUBLIC | MethodAccessFlags::STATIC,
        "booleanArray",
        "(Z)[Z",
        |code| {
            let _ = code.emit(Opcode::IConst1, Operand::None);
            let _ = code.emit(
                Opcode::NewArray,
                Operand::ArrayType(java::bytecode::ArrayType::Boolean),
            );
            let _ = code.emit_store(LocalKind::Reference, 1);
            let _ = code.emit_load(LocalKind::Reference, 1);
            let _ = code.emit(Opcode::IConst0, Operand::None);
            let _ = code.emit_load(LocalKind::Integer, 0);
            let _ = code.emit(Opcode::BAStore, Operand::None);
            let _ = code.emit_load(LocalKind::Reference, 1);
            let _ = code.emit(Opcode::AReturn, Operand::None);
            Ok(())
        },
    )?;
    add_method(
        class,
        MethodAccessFlags::PUBLIC | MethodAccessFlags::STATIC,
        "array",
        "(I)[I",
        |code| {
            let _ = code.emit_load(LocalKind::Integer, 0);
            let _ = code.emit(
                Opcode::NewArray,
                Operand::ArrayType(java::bytecode::ArrayType::Int),
            );
            let _ = code.emit(Opcode::AReturn, Operand::None);
            Ok(())
        },
    )?;
    Ok(())
}

fn add_exception_and_switch_methods(class: &mut ClassFile) -> TestResult<()> {
    add_method(
        class,
        MethodAccessFlags::PUBLIC | MethodAccessFlags::STATIC,
        "select",
        "(I)I",
        |code| {
            let one = code.new_label();
            let five = code.new_label();
            let fallback = code.new_label();
            let _ = code.emit_load(LocalKind::Integer, 0);
            let _ = code.emit_lookup_switch(fallback, [(1, one), (5, five)])?;
            code.bind(one)?;
            let _ = code.emit(Opcode::BiPush, Operand::Byte(10));
            let _ = code.emit(Opcode::IReturn, Operand::None);
            code.bind(five)?;
            let _ = code.emit(Opcode::BiPush, Operand::Byte(20));
            let _ = code.emit(Opcode::IReturn, Operand::None);
            code.bind(fallback)?;
            let _ = code.emit(Opcode::IConstM1, Operand::None);
            let _ = code.emit(Opcode::IReturn, Operand::None);
            Ok(())
        },
    )?;
    let string_length =
        class
            .constant_pool
            .intern_method_ref("java/lang/String", "length", "()I")?;
    let null_pointer = class
        .constant_pool
        .intern_class("java/lang/NullPointerException")?;
    add_method(
        class,
        MethodAccessFlags::PUBLIC | MethodAccessFlags::STATIC,
        "safeLength",
        "(Ljava/lang/String;)I",
        |code| {
            let start = code.new_label();
            let end = code.new_label();
            let handler = code.new_label();
            code.bind(start)?;
            let _ = code.emit_load(LocalKind::Reference, 0);
            let _ = code.emit(Opcode::InvokeVirtual, Operand::Constant(string_length));
            let _ = code.emit(Opcode::IReturn, Operand::None);
            code.bind(end)?;
            code.bind(handler)?;
            let _ = code.emit(Opcode::Pop, Operand::None);
            let _ = code.emit(Opcode::IConstM1, Operand::None);
            let _ = code.emit(Opcode::IReturn, Operand::None);
            code.add_exception_handler(start, end, handler, CatchTarget::Class(null_pointer))?;
            Ok(())
        },
    )?;
    Ok(())
}

fn add_arithmetic_and_control_methods(class: &mut ClassFile) -> TestResult<()> {
    add_method(
        class,
        MethodAccessFlags::PUBLIC | MethodAccessFlags::STATIC,
        "add",
        "(II)I",
        |code| {
            let _ = code.emit_load(LocalKind::Integer, 0);
            let _ = code.emit_load(LocalKind::Integer, 1);
            let _ = code.emit(Opcode::IAdd, Operand::None);
            let _ = code.emit(Opcode::IReturn, Operand::None);
            Ok(())
        },
    )?;
    add_method(
        class,
        MethodAccessFlags::PUBLIC | MethodAccessFlags::STATIC,
        "choose",
        "(I)I",
        |code| {
            let otherwise = code.new_label();
            let _ = code.emit_load(LocalKind::Integer, 0);
            let _ = code.emit_branch(Opcode::IfEq, otherwise)?;
            let _ = code.emit(Opcode::IConst1, Operand::None);
            let _ = code.emit(Opcode::IReturn, Operand::None);
            code.bind(otherwise)?;
            let _ = code.emit(Opcode::IConst2, Operand::None);
            let _ = code.emit(Opcode::IReturn, Operand::None);
            Ok(())
        },
    )?;
    add_method(
        class,
        MethodAccessFlags::PUBLIC | MethodAccessFlags::STATIC,
        "sum",
        "(I)I",
        |code| {
            let header = code.new_label();
            let exit = code.new_label();
            let _ = code.emit(Opcode::IConst0, Operand::None);
            let _ = code.emit_store(LocalKind::Integer, 1);
            let _ = code.emit(Opcode::IConst0, Operand::None);
            let _ = code.emit_store(LocalKind::Integer, 2);
            code.bind(header)?;
            let _ = code.emit_load(LocalKind::Integer, 2);
            let _ = code.emit_load(LocalKind::Integer, 0);
            let _ = code.emit_branch(Opcode::IfICmpGe, exit)?;
            let _ = code.emit_load(LocalKind::Integer, 1);
            let _ = code.emit_load(LocalKind::Integer, 2);
            let _ = code.emit(Opcode::IAdd, Operand::None);
            let _ = code.emit_store(LocalKind::Integer, 1);
            let _ = code.emit(Opcode::IInc, Operand::Increment { index: 2, value: 1 });
            let _ = code.emit_branch(Opcode::Goto, header)?;
            code.bind(exit)?;
            let _ = code.emit_load(LocalKind::Integer, 1);
            let _ = code.emit(Opcode::IReturn, Operand::None);
            Ok(())
        },
    )?;
    Ok(())
}

fn add_method(
    class: &mut ClassFile,
    flags: MethodAccessFlags,
    name: &str,
    descriptor: &str,
    emit: impl FnOnce(&mut CodeBuilder) -> java::Result<()>,
) -> Result<(), Box<dyn std::error::Error>> {
    let owner = class.class_name()?.to_owned();
    let position = class.add_method(flags, name, descriptor)?;
    let mut builder = CodeBuilder::new();
    emit(&mut builder)?;
    let built = builder.finish()?;
    let (code, _) = CodeAttribute::from_built_analyzed(
        &mut class.constant_pool,
        &owner,
        name,
        descriptor,
        flags,
        &built,
    )?;
    class.methods[position]
        .attributes
        .push(Attribute::Code(code));
    Ok(())
}

fn unsupported_initializer_fixture() -> Result<ClassFile, Box<dyn std::error::Error>> {
    let mut class = ClassFile::new(
        JAVA_8_MAJOR_VERSION,
        "sample/UnsupportedInit",
        Some("java/lang/Object"),
        ClassAccessFlags::PUBLIC | ClassAccessFlags::SUPER,
    )?;
    add_method(
        &mut class,
        MethodAccessFlags::STATIC,
        "<clinit>",
        "()V",
        |code| {
            let _ = code.emit(Opcode::AConstNull, Operand::None);
            let _ = code.emit(Opcode::MonitorEnter, Operand::None);
            let _ = code.emit(Opcode::Return, Operand::None);
            Ok(())
        },
    )?;
    class.validate()?;
    Ok(class)
}

fn interface_initializer_fixture() -> TestResult<ClassFile> {
    let mut class = ClassFile::new(
        JAVA_8_MAJOR_VERSION,
        "sample/InitializedInterface",
        Some("java/lang/Object"),
        ClassAccessFlags::PUBLIC | ClassAccessFlags::INTERFACE | ClassAccessFlags::ABSTRACT,
    )?;
    class.add_field(
        FieldAccessFlags::PUBLIC | FieldAccessFlags::STATIC | FieldAccessFlags::FINAL,
        "value",
        "I",
    )?;
    let field =
        class
            .constant_pool
            .intern_field_ref("sample/InitializedInterface", "value", "I")?;
    add_method(
        &mut class,
        MethodAccessFlags::STATIC,
        "<clinit>",
        "()V",
        |code| {
            let _ = code.emit(Opcode::IConst2, Operand::None);
            let _ = code.emit(Opcode::PutStatic, Operand::Constant(field));
            let _ = code.emit(Opcode::Return, Operand::None);
            Ok(())
        },
    )?;
    class.validate()?;
    Ok(class)
}

fn constructor_fallback_fixture() -> TestResult<ClassFile> {
    let mut class = ClassFile::new(
        JAVA_8_MAJOR_VERSION,
        "sample/Child",
        Some("java/lang/Object"),
        ClassAccessFlags::PUBLIC | ClassAccessFlags::SUPER,
    )?;
    class.add_field(
        FieldAccessFlags::PRIVATE | FieldAccessFlags::STATIC,
        "marker",
        "I",
    )?;
    let marker = class
        .constant_pool
        .intern_field_ref("sample/Child", "marker", "I")?;
    let object = class
        .constant_pool
        .intern_method_ref("java/lang/Object", "<init>", "()V")?;
    let sibling = class
        .constant_pool
        .intern_method_ref("sample/Child", "<init>", "(I)V")?;
    add_method(
        &mut class,
        MethodAccessFlags::PRIVATE,
        "<init>",
        "(I)V",
        |code| {
            let _ = code.emit_load(LocalKind::Reference, 0);
            let _ = code.emit(Opcode::InvokeSpecial, Operand::Constant(object));
            let _ = code.emit(Opcode::Return, Operand::None);
            Ok(())
        },
    )?;
    add_method(
        &mut class,
        MethodAccessFlags::PUBLIC,
        "<init>",
        "(Ljava/lang/String;)V",
        |code| {
            let _ = code.emit(Opcode::GetStatic, Operand::Constant(marker));
            let _ = code.emit(Opcode::Pop, Operand::None);
            let _ = code.emit_load(LocalKind::Reference, 0);
            let _ = code.emit(Opcode::IConst2, Operand::None);
            let _ = code.emit(Opcode::InvokeSpecial, Operand::Constant(sibling));
            let _ = code.emit(Opcode::Return, Operand::None);
            Ok(())
        },
    )?;
    class.validate()?;
    Ok(class)
}

#[test]
fn decompiles_verified_mlil_with_source_provenance() -> Result<(), Box<dyn std::error::Error>> {
    let output = decompile_class(&fixture()?)?;
    assert!(
        output.source.contains("package sample;"),
        "{}",
        output.source
    );
    assert!(
        output
            .source
            .contains("public static int add(int parameter0, int parameter1)"),
        "{}",
        output.source
    );
    assert!(output.source.contains(" + "), "{}", output.source);
    assert!(output.source.contains("if ("), "{}", output.source);
    // The counting loop recovers as a genuine `for` with its real
    // relational condition and compound update, and the lookup switch
    // renders structurally with its case keys and default arm.
    assert!(output.source.contains("for ("), "{}", output.source);
    assert!(output.source.contains(" < "), "{}", output.source);
    assert!(output.source.contains("++"), "{}", output.source);
    assert!(
        !output
            .source
            .contains("while (java.lang.Boolean.TRUE.booleanValue())"),
        "{}",
        output.source
    );
    assert!(output.source.contains("switch ("), "{}", output.source);
    assert!(output.source.contains("case 1: {"), "{}", output.source);
    assert!(output.source.contains("case 5: {"), "{}", output.source);
    assert!(output.source.contains("default: {"), "{}", output.source);
    assert!(output.source.contains("new int["), "{}", output.source);
    assert!(
        output.source.contains("return parameter0 * 2;"),
        "{}",
        output.source
    );
    assert!(!output.source.contains(" = this;"), "{}", output.source);
    assert!(
        output.source.contains("public Arithmetic() {\n    }"),
        "{}",
        output.source
    );
    assert!(output.source.contains("seed ="), "{}", output.source);
    assert!(
        output.source.contains("public static int seed;"),
        "{}",
        output.source
    );
    assert!(
        !output.source.contains("sample.Arithmetic.seed ="),
        "{}",
        output.source
    );
    // Exception dispatch is structured: one catch-all per region with
    // ordered `instanceof` dispatch, no state machine left in the fixture.
    assert!(output.source.contains("try {"), "{}", output.source);
    assert!(
        output
            .source
            .contains("} catch (java.lang.Throwable cafe_caught_"),
        "{}",
        output.source
    );
    assert!(
        output
            .source
            .contains("instanceof java.lang.NullPointerException"),
        "{}",
        output.source
    );
    assert!(
        !output.source.contains("cafe_dispatch"),
        "{}",
        output.source
    );
    assert_eq!(output.source.matches(" covariant()").count(), 1);
    assert!(output.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == decompiler::DiagnosticCode::DeclarationApproximation
            && diagnostic.message.contains("omits `final`")
    }));
    assert!(output.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == decompiler::DiagnosticCode::DeclarationApproximation
            && diagnostic.message.contains("bridge method is omitted")
    }));
    assert!(!output.source_map.is_empty());
    assert!(output.source_map.iter().all(|entry| {
        entry.generated.start < entry.generated.end && entry.generated.end <= output.source.len()
    }));
    Ok(())
}

#[test]
fn renders_inner_class_metadata_as_java_member_names() -> TestResult<()> {
    let mut class = ClassFile::new(
        JAVA_8_MAJOR_VERSION,
        "sample/Consumer",
        Some("java/lang/Object"),
        ClassAccessFlags::PUBLIC | ClassAccessFlags::SUPER,
    )?;
    class.add_field(FieldAccessFlags::PUBLIC, "value", "Lsample/Outer$Inner;")?;
    let name_index = class
        .constant_pool
        .intern_utf8(KnownAttributeKind::InnerClasses.name())?;
    let inner_class_info_index = class.constant_pool.intern_class("sample/Outer$Inner")?;
    let outer_class_info_index = class.constant_pool.intern_class("sample/Outer")?;
    let inner_name_index = class.constant_pool.intern_utf8("Inner")?;
    class
        .attributes
        .push(Attribute::Known(KnownAttribute::InnerClasses(
            InnerClassesAttribute {
                name_index,
                classes: vec![InnerClass {
                    inner_class_info_index,
                    outer_class_info_index,
                    inner_name_index,
                    access_flags: InnerClassAccessFlags::PUBLIC | InnerClassAccessFlags::STATIC,
                }],
            },
        )));
    let output = decompile_class(&class)?;
    assert!(
        output.source.contains("sample.Outer.Inner value;"),
        "{}",
        output.source
    );
    Ok(())
}

#[test]
fn diagnoses_interface_initialization_that_java_source_cannot_declare() -> TestResult<()> {
    let output = decompile_class(&interface_initializer_fixture()?)?;
    assert!(
        output.source.contains("public static final int value = 0;"),
        "{}",
        output.source
    );
    assert!(
        !output.source.contains("\n    static {"),
        "{}",
        output.source
    );
    assert!(output.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == decompiler::DiagnosticCode::UnsupportedSemantics
            && diagnostic
                .message
                .contains("interface class initialization")
    }));
    Ok(())
}

#[test]
fn constructor_stub_preserves_the_required_super_signature() -> TestResult<()> {
    let output = decompile_class(&constructor_fallback_fixture()?)?;
    assert!(output.source.contains("this(0);"), "{}", output.source);
    assert!(output.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == decompiler::DiagnosticCode::UnsupportedSemantics
            && diagnostic
                .method
                .as_ref()
                .is_some_and(|method| method.name == "<init>")
    }));
    Ok(())
}

#[test]
fn generated_source_compiles_and_executes_when_javac_is_available()
-> Result<(), Box<dyn std::error::Error>> {
    if Command::new("javac").arg("-version").output().is_err() {
        return Ok(());
    }
    let output = decompile_class(&fixture()?)?;
    let root = temporary_directory();
    let source_dir = root.join("sample");
    fs::create_dir_all(&source_dir)?;
    let source_file = source_dir.join("Arithmetic.java");
    fs::write(&source_file, &output.source)?;
    let compile = Command::new("javac")
        .arg("-d")
        .arg(&root)
        .arg(&source_file)
        .output()?;
    assert!(
        compile.status.success(),
        "javac failed:\n{}\nsource:\n{}",
        String::from_utf8_lossy(&compile.stderr),
        output.source
    );

    let harness = root.join("Harness.java");
    fs::write(
        &harness,
        "public final class Harness { public static void main(String[] args) { sample.Arithmetic.setCounter(8); sample.Arithmetic.setFlag(true); System.out.print(sample.Arithmetic.add(4, 5) + \":\" + sample.Arithmetic.choose(0) + \":\" + sample.Arithmetic.array(3).length + \":\" + new sample.Arithmetic().twice(6) + \":\" + sample.Arithmetic.safeLength(\"abc\") + \":\" + sample.Arithmetic.safeLength(null) + \":\" + sample.Arithmetic.select(5) + \":\" + sample.Arithmetic.select(7) + \":\" + sample.Arithmetic.sum(5) + \":\" + sample.Arithmetic.make().twice(3) + \":\" + sample.Arithmetic.callTwice(4) + \":\" + sample.Arithmetic.isString(\"x\") + \":\" + sample.Arithmetic.asString(\"x\") + \":\" + sample.Arithmetic.first(new int[] { 7 }) + \":\" + sample.Arithmetic.getCounter() + \":\" + sample.Arithmetic.getFlag() + \":\" + sample.Arithmetic.callBoolean() + \":\" + sample.Arithmetic.booleanArray(true)[0]); } }",
    )?;
    let compile = Command::new("javac")
        .arg("-cp")
        .arg(&root)
        .arg("-d")
        .arg(&root)
        .arg(&harness)
        .output()?;
    assert!(
        compile.status.success(),
        "{}",
        String::from_utf8_lossy(&compile.stderr)
    );
    let run = Command::new("java")
        .arg("-cp")
        .arg(&root)
        .arg("Harness")
        .output()?;
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    assert_eq!(
        String::from_utf8(run.stdout)?,
        "9:2:3:12:3:-1:20:-1:10:6:8:true:x:7:8:true:true:true"
    );
    fs::remove_dir_all(&root)?;
    Ok(())
}

#[test]
fn unsupported_static_initializer_uses_a_compilable_throwing_stub()
-> Result<(), Box<dyn std::error::Error>> {
    if Command::new("javac").arg("-version").output().is_err() {
        return Ok(());
    }
    let output = decompile_class(&unsupported_initializer_fixture()?)?;
    assert!(
        output.diagnostics.iter().any(|diagnostic| {
            diagnostic.code == decompiler::DiagnosticCode::UnsupportedSemantics
        }),
        "{:?}",
        output.diagnostics
    );
    assert!(
        output
            .source
            .contains("java.lang.Boolean.TRUE.booleanValue()"),
        "{}",
        output.source
    );
    let root = temporary_directory();
    let source_dir = root.join("sample");
    fs::create_dir_all(&source_dir)?;
    let source_file = source_dir.join("UnsupportedInit.java");
    fs::write(&source_file, &output.source)?;
    let compile = Command::new("javac")
        .arg("-d")
        .arg(&root)
        .arg(&source_file)
        .output()?;
    assert!(
        compile.status.success(),
        "javac failed:\n{}\nsource:\n{}",
        String::from_utf8_lossy(&compile.stderr),
        output.source
    );
    fs::remove_dir_all(&root)?;
    Ok(())
}

fn temporary_directory() -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock follows Unix epoch")
        .as_nanos();
    std::env::temp_dir().join(format!("cafe-decompiler-{}-{nonce}", std::process::id()))
}

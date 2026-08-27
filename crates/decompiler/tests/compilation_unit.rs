//! Member-class compilation-unit and generic-declaration coverage.

use decompiler::{
    DecompilerOptions, MethodExceptionCatalog, decompile_class,
    decompile_compilation_unit_with_environment,
};
use java::analysis::ClassHierarchy;
use java::bytecode::{CodeBuilder, LocalKind, Opcode, Operand};
use java::classfile::{
    Attribute, ClassAccessFlags, ClassFile, CodeAttribute, FieldAccessFlags, IndexAttribute,
    InnerClass, InnerClassAccessFlags, InnerClassesAttribute, JAVA_8_MAJOR_VERSION, KnownAttribute,
    KnownAttributeKind, MethodAccessFlags,
};

type TestResult<T> = Result<T, Box<dyn std::error::Error>>;

const OUTER: &str = "sample/ClosedSetHash";
const INNER: &str = "sample/ClosedSetHash$MinNodeProc";

#[test]
fn aggregates_named_member_classes_with_generic_declarations() -> TestResult<()> {
    let (outer, inner) = class_family()?;

    let hierarchy = ClassHierarchy::from_classes([&outer, &inner])?;
    let method_exceptions = MethodExceptionCatalog::from_classes([&outer, &inner])?;
    let output = decompile_compilation_unit_with_environment(
        &outer,
        &[&inner],
        Some(&hierarchy),
        &method_exceptions,
        &DecompilerOptions::default(),
    )?;
    assert_eq!(output.source.matches("package sample;").count(), 1);
    assert!(
        output.source.contains(
            "private static final class MinNodeProc implements gnu.trove.procedure.TObjectProcedure<sample.Node>"
        ),
        "{}",
        output.source
    );
    assert!(
        output
            .source
            .contains("java.util.Comparator<sample.Node> comp;"),
        "{}",
        output.source
    );
    assert!(
        output.source.contains("private MinNodeProc() {\n        }"),
        "{}",
        output.source
    );
    assert!(!output.source.contains("super();"), "{}", output.source);
    assert!(!output.source.contains("cafe_v"), "{}", output.source);

    let standalone = decompile_class(&inner)?;
    assert!(
        standalone
            .source
            .contains("class ClosedSetHash$MinNodeProc"),
        "{}",
        standalone.source
    );
    assert!(
        standalone
            .source
            .contains("private ClosedSetHash$MinNodeProc()"),
        "{}",
        standalone.source
    );
    Ok(())
}

fn class_family() -> TestResult<(ClassFile, ClassFile)> {
    let member_flags = InnerClassAccessFlags::PRIVATE
        | InnerClassAccessFlags::STATIC
        | InnerClassAccessFlags::FINAL;
    let mut outer = ClassFile::new(
        JAVA_8_MAJOR_VERSION,
        OUTER,
        Some("java/lang/Object"),
        ClassAccessFlags::PUBLIC | ClassAccessFlags::SUPER,
    )?;
    outer.add_field(
        FieldAccessFlags::PRIVATE | FieldAccessFlags::STATIC,
        "min",
        "Lsample/ClosedSetHash$MinNodeProc;",
    )?;
    add_inner_classes_metadata(&mut outer, member_flags)?;

    let mut inner = ClassFile::new(
        JAVA_8_MAJOR_VERSION,
        INNER,
        Some("java/lang/Object"),
        ClassAccessFlags::FINAL | ClassAccessFlags::SUPER,
    )?;
    inner.add_interface("gnu/trove/procedure/TObjectProcedure")?;
    let field = inner.add_field(FieldAccessFlags::PRIVATE, "comp", "Ljava/util/Comparator;")?;
    add_inner_classes_metadata(&mut inner, member_flags)?;
    add_signatures(&mut inner, field)?;
    add_default_constructor(&mut inner)?;
    outer.validate()?;
    inner.validate()?;
    Ok((outer, inner))
}

fn add_signatures(class: &mut ClassFile, field: usize) -> TestResult<()> {
    let name_index = class
        .constant_pool
        .intern_utf8(KnownAttributeKind::Signature.name())?;
    let class_signature = class
        .constant_pool
        .intern_utf8("Ljava/lang/Object;Lgnu/trove/procedure/TObjectProcedure<Lsample/Node;>;")?;
    class
        .attributes
        .push(Attribute::Known(KnownAttribute::Signature(
            IndexAttribute {
                name_index,
                index: class_signature,
            },
        )));
    let field_signature = class
        .constant_pool
        .intern_utf8("Ljava/util/Comparator<Lsample/Node;>;")?;
    class.fields[field]
        .attributes
        .push(Attribute::Known(KnownAttribute::Signature(
            IndexAttribute {
                name_index,
                index: field_signature,
            },
        )));
    Ok(())
}

fn add_inner_classes_metadata(
    class: &mut ClassFile,
    access_flags: InnerClassAccessFlags,
) -> TestResult<()> {
    let name_index = class
        .constant_pool
        .intern_utf8(KnownAttributeKind::InnerClasses.name())?;
    let inner_class_info_index = class.constant_pool.intern_class(INNER)?;
    let outer_class_info_index = class.constant_pool.intern_class(OUTER)?;
    let inner_name_index = class.constant_pool.intern_utf8("MinNodeProc")?;
    class
        .attributes
        .push(Attribute::Known(KnownAttribute::InnerClasses(
            InnerClassesAttribute {
                name_index,
                classes: vec![InnerClass {
                    inner_class_info_index,
                    outer_class_info_index,
                    inner_name_index,
                    access_flags,
                }],
            },
        )));
    Ok(())
}

fn add_default_constructor(class: &mut ClassFile) -> TestResult<()> {
    let constructor = class
        .constant_pool
        .intern_method_ref("java/lang/Object", "<init>", "()V")?;
    let position = class.add_method(MethodAccessFlags::PRIVATE, "<init>", "()V")?;
    let mut code = CodeBuilder::new();
    let _ = code.emit_load(LocalKind::Reference, 0);
    let _ = code.emit(Opcode::InvokeSpecial, Operand::Constant(constructor));
    let _ = code.emit(Opcode::Return, Operand::None);
    let built = code.finish()?;
    let (code, _) = CodeAttribute::from_built_analyzed(
        &mut class.constant_pool,
        INNER,
        "<init>",
        "()V",
        MethodAccessFlags::PRIVATE,
        &built,
    )?;
    class.methods[position]
        .attributes
        .push(Attribute::Code(code));
    Ok(())
}

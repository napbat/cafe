//! Proof that a consumer can reach every Cafe capability through one crate.

use cafe::{
    ModuleSource, Program, art, cfglib, classpath, dex, disassembler, java, jni, mlil, program,
};

#[test]
#[allow(clippy::too_many_lines)]
fn exposes_every_public_layer_through_cafe() -> Result<(), Box<dyn std::error::Error>> {
    exercise_analysis_entry_points()?;

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
    let emitted_classes = java::emit_module(&module)?;
    assert_eq!(emitted_classes.len(), 1);
    let hierarchy = classpath::ClasspathHierarchy::from_java_classes([&class])?;
    assert_eq!(hierarchy.len(), 1);
    let _ = hierarchy.jvm_view();
    let _ = hierarchy.dex_view();
    let program = Program::from_modules([module]);
    let native_methods = jni::java::native_methods(&class)?;
    let bindings = native_methods.bindings()?;
    let empty_dex = dex::DexFile::new(dex::DexVersion::V040);
    let emitted_dex = dex::emit_module(&empty_dex.to_module()?)?;
    assert_eq!(emitted_dex.version(), dex::DexVersion::V040);
    let vdex = art::VdexFile::from_standard_dex_files(std::slice::from_ref(&empty_dex), &[], &[])?;
    assert!(matches!(
        vdex.runtime_dex(0)?,
        art::RuntimeDex::Standard(file) if file.source_format() == empty_dex.source_format()
    ));
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
    let _ = std::any::type_name::<art::VdexFile>();
    let _ = std::any::type_name::<cfglib::BlockId>();
    let _ = std::any::type_name::<disassembler::ControlFlowEdge>();
    let _ = std::any::type_name::<disassembler::ControlFlowEdgeRole>();
    let _ = std::any::type_name::<disassembler::RecoveredExceptionModel>();
    let _ = std::any::type_name::<disassembler::RecoveredExceptionHandler>();
    let _ = std::any::type_name::<disassembler::HandlerExtentStatus>();
    let _ = std::any::type_name::<disassembler::RecoveredStructuredControlFlow>();
    let _ = std::any::type_name::<disassembler::StructuredRegionDecision>();
    let _ = std::any::type_name::<disassembler::RegisterResources>();
    let _ = std::any::type_name::<cfglib::HandlerTypes<String>>();
    let _ = std::any::type_name::<java::analysis::ClassHierarchy>();
    let _ = std::any::type_name::<java::llil::Body>();
    let _ = std::any::type_name::<java::mlil::LoweredBody>();
    let _ = std::any::type_name::<java::mlil::SourceJavaReferenceResolver>();
    let _ = std::any::type_name::<java::JavaEmitter>();
    let _ = std::any::type_name::<dex::analysis::RegisterAnalysis>();
    let _ = std::any::type_name::<dex::llil::Body>();
    let _ = std::any::type_name::<dex::mlil::LoweredBody>();
    let _ = std::any::type_name::<dex::mlil::SourceDexReferenceResolver>();
    let _ = std::any::type_name::<dex::mlil::TargetDexReferenceResolver>();
    let _ = std::any::type_name::<dex::DexEmitter>();
    let _ = std::any::type_name::<mlil::Function>();
    let _ = std::any::type_name::<mlil::ArrayType>();
    let _ = std::any::type_name::<mlil::EdgeMetadata>();
    let _ = std::any::type_name::<classpath::JvmHierarchyView<'static>>();
    let _ = std::any::type_name::<classpath::DexHierarchyView<'static>>();
    let _: fn(
        &mlil::Function,
        &mut java::classfile::ConstantPool,
    ) -> java::mlil::Result<java::mlil::LoweredBody> = java::mlil::lower_body;
    let _: fn(
        &mlil::Function,
        &mut java::classfile::ConstantPool,
    ) -> java::mlil::Result<java::mlil::LoweredBody> = java::mlil::lower_body_from_source;
    let _: fn(&dex::DexFile, &mlil::Function) -> dex::mlil::Result<dex::mlil::LoweredBody> =
        dex::mlil::lower_body;
    let _: fn(&dex::DexFile, &mlil::Function) -> dex::mlil::Result<dex::mlil::LoweredBody> =
        dex::mlil::lower_body_from_source;
    Ok(())
}

#[test]
fn resolves_equivalent_jvm_and_dex_definitions_without_identity_collisions()
-> Result<(), Box<dyn std::error::Error>> {
    const OWNER: &str = "sample/Equivalent";
    const DEX_OWNER: &str = "Lsample/Equivalent;";
    const METHOD: &str = "transform";
    const DESCRIPTOR: &str = "(I)Ljava/lang/String;";

    let mut class = java::classfile::ClassFile::new(
        java::classfile::JAVA_8_MAJOR_VERSION,
        OWNER,
        Some("java/lang/Object"),
        java::classfile::ClassAccessFlags::PUBLIC,
    )?;
    class.add_method(
        java::classfile::MethodAccessFlags::PUBLIC
            | java::classfile::MethodAccessFlags::STATIC
            | java::classfile::MethodAccessFlags::NATIVE,
        METHOD,
        DESCRIPTOR,
    )?;

    let mut builder = dex::file::DexBuilder::new(dex::DexVersion::V040);
    let owner_handle = builder.intern_type(DEX_OWNER)?;
    let method_handle =
        builder.intern_method_named(DEX_OWNER, METHOD, "Ljava/lang/String;", &["I"])?;
    let mut built = builder.build()?;
    let owner = built
        .indices
        .type_index(owner_handle)
        .expect("owner was interned");
    let method = built
        .indices
        .method(method_handle)
        .expect("method was interned");
    built.file.push_class(dex::file::ClassDefinition {
        class: owner,
        access_flags: dex::file::AccessFlags::PUBLIC,
        superclass: None,
        interfaces: Vec::new(),
        source_file: None,
        annotations: dex::file::AnnotationDirectory::default(),
        class_data: Some(dex::file::ClassData {
            static_fields: Vec::new(),
            instance_fields: Vec::new(),
            direct_methods: vec![dex::file::EncodedMethod {
                method,
                access_flags: dex::file::AccessFlags::from_bits_retain(
                    dex::file::AccessFlags::PUBLIC.bits()
                        | dex::file::AccessFlags::STATIC.bits()
                        | dex::file::AccessFlags::NATIVE.bits(),
                ),
                code: None,
            }],
            virtual_methods: Vec::new(),
            data_offset: 0,
        }),
        static_values: Vec::new(),
        definition_index: 0,
    })?;

    let program = Program::from_modules([class.to_module()?, built.file.to_module()?]);
    let java_owner = cafe::TypeId::new(cafe::BinaryFormat::JavaClass, OWNER);
    let dex_owner = cafe::TypeId::new(cafe::BinaryFormat::Dex, DEX_OWNER);
    let method = cafe::MethodId::new(METHOD, DESCRIPTOR);

    assert!(program.resolve_type(&java_owner).unique().is_some());
    assert!(program.resolve_type(&dex_owner).unique().is_some());
    assert!(
        program
            .resolve_method(&java_owner, &method)
            .unique()
            .is_some()
    );
    assert!(
        program
            .resolve_method(&dex_owner, &method)
            .unique()
            .is_some()
    );
    assert_ne!(java_owner, dex_owner);
    Ok(())
}

fn exercise_analysis_entry_points() -> Result<(), Box<dyn std::error::Error>> {
    let mut code = java::bytecode::CodeBuilder::new();
    let _ = code.emit(
        java::bytecode::Opcode::Return,
        java::bytecode::Operand::None,
    );
    let built_code = code.finish()?;
    assert_eq!(built_code.code(), [java::bytecode::Opcode::Return.byte()]);
    let native_jvm = java::bytecode::decode(built_code.code())?;
    let jvm_llil = java::llil::lift_instructions(&native_jvm)?;
    assert_eq!(java::llil::lower_instructions(&jvm_llil)?, native_jvm);
    let mut analysis_pool = java::classfile::ConstantPool::new();
    let (_analyzed_code, method_analysis) = java::classfile::CodeAttribute::from_built_analyzed(
        &mut analysis_pool,
        "sample/Generated",
        "run",
        "()V",
        java::classfile::MethodAccessFlags::STATIC,
        &built_code,
    )?;
    assert_eq!(method_analysis.max_stack(), 0);

    let dalvik_return = dex::instruction::Instruction::operation(
        0,
        dex::instruction::Opcode::ReturnVoid,
        dex::instruction::Operands::None,
    );
    assert!(
        dex::analysis::instruction_semantics(&dalvik_return)?
            .reads
            .is_empty()
    );
    let dex_llil = dex::llil::lift_instructions(std::slice::from_ref(&dalvik_return))?;
    assert_eq!(dex::llil::lower_instructions(&dex_llil)?, [dalvik_return]);

    let source_coordinate = disassembler::FunctionCoordinate::new(
        disassembler::BinaryFormat::Dex,
        disassembler::FunctionSymbol {
            owner: "Lsample/Generated;".to_owned(),
            name: "run".to_owned(),
            signature: "()V".to_owned(),
        },
        disassembler::AddressUnit::CodeUnit16,
    );
    let generated_coordinate = disassembler::FunctionCoordinate::new(
        disassembler::BinaryFormat::JavaClass,
        disassembler::FunctionSymbol {
            owner: "sample/Generated".to_owned(),
            name: "run".to_owned(),
            signature: "()V".to_owned(),
        },
        disassembler::AddressUnit::Byte,
    );
    let source_map = disassembler::SourceMap::new(source_coordinate.clone(), generated_coordinate);
    assert!(source_map.is_empty());
    let mut diagnostics = disassembler::Diagnostics::new();
    diagnostics.push(
        disassembler::Diagnostic::new(disassembler::DiagnosticLevel::Note, "fixture").at(
            disassembler::DiagnosticLocation::new(
                source_coordinate,
                disassembler::AddressRange::new(
                    disassembler::CodeAddress::new(0),
                    disassembler::CodeAddress::new(1),
                ),
            ),
        ),
    );
    assert!(!diagnostics.has_errors());

    Ok(())
}

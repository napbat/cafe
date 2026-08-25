//! Cross-ISA proof that Cafe exposes one verified semantic IL.

use cafe::{dex, disassembler, java, mlil};

fn jvm_constant_fixture()
-> Result<(java::classfile::ClassFile, usize, mlil::Function), Box<dyn std::error::Error>> {
    let mut class = java::classfile::ClassFile::new(
        java::classfile::JAVA_8_MAJOR_VERSION,
        "sample/CrossIsa",
        Some("java/lang/Object"),
        java::classfile::ClassAccessFlags::PUBLIC,
    )?;
    let flags =
        java::classfile::MethodAccessFlags::PUBLIC | java::classfile::MethodAccessFlags::STATIC;
    let method_position = class.add_method(flags, "value", "()I")?;
    let mut code = java::bytecode::CodeBuilder::new();
    let _ = code.emit(
        java::bytecode::Opcode::IConst1,
        java::bytecode::Operand::None,
    );
    let _ = code.emit(
        java::bytecode::Opcode::IReturn,
        java::bytecode::Operand::None,
    );
    let built = code.finish()?;
    let (code_attribute, _) = java::classfile::CodeAttribute::from_built_analyzed(
        &mut class.constant_pool,
        "sample/CrossIsa",
        "value",
        "()I",
        flags,
        &built,
    )?;
    class.methods[method_position]
        .attributes
        .push(java::classfile::Attribute::Code(code_attribute));
    let function =
        java::mlil::lift_method(&class, &class.methods[method_position])?.expect("method has code");
    Ok((class, method_position, function))
}

fn jvm_constant_function() -> Result<mlil::Function, Box<dyn std::error::Error>> {
    Ok(jvm_constant_fixture()?.2)
}

fn dex_constant_fixture()
-> Result<(dex::DexFile, dex::file::EncodedMethod, mlil::Function), Box<dyn std::error::Error>> {
    let mut file = dex::file::DexFile::new(dex::file::DexVersion::V040);
    let method_name = file.push_string(dex::file::DexString::new("value"))?;
    let owner_text = file.push_string(dex::file::DexString::new("Lsample/CrossIsa;"))?;
    let int_text = file.push_string(dex::file::DexString::new("I"))?;
    let owner = file.push_type(dex::file::TypeId {
        descriptor: owner_text,
    })?;
    let int = file.push_type(dex::file::TypeId {
        descriptor: int_text,
    })?;
    let prototype = file.push_prototype(dex::file::PrototypeId {
        shorty: int_text,
        return_type: int,
        parameters: Vec::new(),
        parameters_offset: 0,
    })?;
    let method = file.push_method(dex::file::MethodId {
        class: owner,
        prototype,
        name: method_name,
    })?;
    let declaration = dex::file::EncodedMethod {
        method,
        access_flags: dex::file::AccessFlags::STATIC,
        code: Some(dex::file::CodeItem {
            registers_size: 1,
            ins_size: 0,
            outs_size: 0,
            instructions: vec![
                dex::instruction::Instruction::operation(
                    0,
                    dex::instruction::Opcode::Const4,
                    dex::instruction::Operands::RegisterLiteral {
                        register: 0,
                        literal: 1,
                    },
                ),
                dex::instruction::Instruction::operation(
                    1,
                    dex::instruction::Opcode::Return,
                    dex::instruction::Operands::Register(0),
                ),
            ],
            tries: Vec::new(),
            debug_info: None,
            data_offset: 0,
        }),
    };
    let function = dex::mlil::lift_method(&file, &declaration)?.expect("method has code");
    Ok((file, declaration, function))
}

fn dex_constant_function() -> Result<mlil::Function, Box<dyn std::error::Error>> {
    Ok(dex_constant_fixture()?.2)
}

fn jvm_reference_identity() -> Result<mlil::Function, Box<dyn std::error::Error>> {
    let mut class = java::classfile::ClassFile::new(
        java::classfile::JAVA_8_MAJOR_VERSION,
        "sample/CrossIsa",
        Some("java/lang/Object"),
        java::classfile::ClassAccessFlags::PUBLIC,
    )?;
    let flags =
        java::classfile::MethodAccessFlags::PUBLIC | java::classfile::MethodAccessFlags::STATIC;
    let descriptor = "(Lsample/CrossIsa;)Lsample/CrossIsa;";
    let method_position = class.add_method(flags, "identity", descriptor)?;
    let mut code = java::bytecode::CodeBuilder::new();
    let _ = code.emit(
        java::bytecode::Opcode::ALoad0,
        java::bytecode::Operand::None,
    );
    let _ = code.emit(
        java::bytecode::Opcode::AReturn,
        java::bytecode::Operand::None,
    );
    let built = code.finish()?;
    let (code_attribute, _) = java::classfile::CodeAttribute::from_built_analyzed(
        &mut class.constant_pool,
        "sample/CrossIsa",
        "identity",
        descriptor,
        flags,
        &built,
    )?;
    class.methods[method_position]
        .attributes
        .push(java::classfile::Attribute::Code(code_attribute));
    Ok(java::mlil::lift_method(&class, &class.methods[method_position])?.expect("method has code"))
}

fn dex_reference_identity_fixture()
-> Result<(dex::DexFile, dex::file::EncodedMethod, mlil::Function), Box<dyn std::error::Error>> {
    let mut file = dex::file::DexFile::new(dex::file::DexVersion::V040);
    let method_name = file.push_string(dex::file::DexString::new("identity"))?;
    let owner_text = file.push_string(dex::file::DexString::new("Lsample/CrossIsa;"))?;
    let shorty_text = file.push_string(dex::file::DexString::new("LL"))?;
    let owner = file.push_type(dex::file::TypeId {
        descriptor: owner_text,
    })?;
    let prototype = file.push_prototype(dex::file::PrototypeId {
        shorty: shorty_text,
        return_type: owner,
        parameters: vec![owner],
        parameters_offset: 0,
    })?;
    let method = file.push_method(dex::file::MethodId {
        class: owner,
        prototype,
        name: method_name,
    })?;
    let declaration = dex::file::EncodedMethod {
        method,
        access_flags: dex::file::AccessFlags::STATIC,
        code: Some(dex::file::CodeItem {
            registers_size: 1,
            ins_size: 1,
            outs_size: 0,
            instructions: vec![dex::instruction::Instruction::operation(
                0,
                dex::instruction::Opcode::ReturnObject,
                dex::instruction::Operands::Register(0),
            )],
            tries: Vec::new(),
            debug_info: None,
            data_offset: 0,
        }),
    };
    let function = dex::mlil::lift_method(&file, &declaration)?.expect("method has code");
    Ok((file, declaration, function))
}

fn dex_reference_identity() -> Result<mlil::Function, Box<dyn std::error::Error>> {
    Ok(dex_reference_identity_fixture()?.2)
}

fn dex_null_function() -> Result<mlil::Function, Box<dyn std::error::Error>> {
    let mut file = dex::DexFile::new(dex::DexVersion::V040);
    let method_name = file.push_string(dex::file::DexString::new("nullable"))?;
    let owner_text = file.push_string(dex::file::DexString::new("Lsample/CrossIsa;"))?;
    let shorty_text = file.push_string(dex::file::DexString::new("L"))?;
    let owner = file.push_type(dex::file::TypeId {
        descriptor: owner_text,
    })?;
    let prototype = file.push_prototype(dex::file::PrototypeId {
        shorty: shorty_text,
        return_type: owner,
        parameters: Vec::new(),
        parameters_offset: 0,
    })?;
    let method = file.push_method(dex::file::MethodId {
        class: owner,
        prototype,
        name: method_name,
    })?;
    let declaration = dex::file::EncodedMethod {
        method,
        access_flags: dex::file::AccessFlags::STATIC,
        code: Some(dex::file::CodeItem {
            registers_size: 1,
            ins_size: 0,
            outs_size: 0,
            instructions: vec![
                dex::instruction::Instruction::operation(
                    0,
                    dex::instruction::Opcode::Const4,
                    dex::instruction::Operands::RegisterLiteral {
                        register: 0,
                        literal: 0,
                    },
                ),
                dex::instruction::Instruction::operation(
                    1,
                    dex::instruction::Opcode::ReturnObject,
                    dex::instruction::Operands::Register(0),
                ),
            ],
            tries: Vec::new(),
            debug_info: None,
            data_offset: 0,
        }),
    };
    Ok(dex::mlil::lift_method(&file, &declaration)?.expect("method has code"))
}

fn dex_fill_array_function() -> Result<mlil::Function, Box<dyn std::error::Error>> {
    let mut file = dex::DexFile::new(dex::DexVersion::V040);
    let method_name = file.push_string(dex::file::DexString::new("fill"))?;
    let owner_text = file.push_string(dex::file::DexString::new("Lsample/CrossIsa;"))?;
    let int_text = file.push_string(dex::file::DexString::new("I"))?;
    let array_text = file.push_string(dex::file::DexString::new("[I"))?;
    let void_text = file.push_string(dex::file::DexString::new("V"))?;
    let shorty_text = file.push_string(dex::file::DexString::new("VL"))?;
    let owner = file.push_type(dex::file::TypeId {
        descriptor: owner_text,
    })?;
    let _int = file.push_type(dex::file::TypeId {
        descriptor: int_text,
    })?;
    let array = file.push_type(dex::file::TypeId {
        descriptor: array_text,
    })?;
    let void = file.push_type(dex::file::TypeId {
        descriptor: void_text,
    })?;
    let prototype = file.push_prototype(dex::file::PrototypeId {
        shorty: shorty_text,
        return_type: void,
        parameters: vec![array],
        parameters_offset: 0,
    })?;
    let method = file.push_method(dex::file::MethodId {
        class: owner,
        prototype,
        name: method_name,
    })?;
    let declaration = dex::file::EncodedMethod {
        method,
        access_flags: dex::file::AccessFlags::STATIC,
        code: Some(dex::file::CodeItem {
            registers_size: 1,
            ins_size: 1,
            outs_size: 0,
            instructions: vec![
                dex::instruction::Instruction::operation(
                    0,
                    dex::instruction::Opcode::FillArrayData,
                    dex::instruction::Operands::RegisterBranch {
                        register: 0,
                        target: 4,
                    },
                ),
                dex::instruction::Instruction::operation(
                    3,
                    dex::instruction::Opcode::ReturnVoid,
                    dex::instruction::Operands::None,
                ),
                dex::instruction::Instruction::array_data(
                    4,
                    dex::instruction::ArrayDataPayload {
                        element_width: 4,
                        element_count: 2,
                        data: vec![1, 0, 0, 0, 2, 0, 0, 0],
                    },
                ),
            ],
            tries: Vec::new(),
            debug_info: None,
            data_offset: 0,
        }),
    };
    Ok(dex::mlil::lift_method(&file, &declaration)?.expect("method has code"))
}

fn jvm_new_array_fixture()
-> Result<(dex::DexFile, dex::file::EncodedMethod, mlil::Function), Box<dyn std::error::Error>> {
    let mut class = java::classfile::ClassFile::new(
        java::classfile::JAVA_8_MAJOR_VERSION,
        "sample/CrossIsa",
        Some("java/lang/Object"),
        java::classfile::ClassAccessFlags::PUBLIC,
    )?;
    let flags =
        java::classfile::MethodAccessFlags::PUBLIC | java::classfile::MethodAccessFlags::STATIC;
    let method_position = class.add_method(flags, "array", "(I)[I")?;
    let mut code = java::bytecode::CodeBuilder::new();
    let _ = code.emit(
        java::bytecode::Opcode::ILoad0,
        java::bytecode::Operand::None,
    );
    let _ = code.emit(
        java::bytecode::Opcode::NewArray,
        java::bytecode::Operand::ArrayType(java::bytecode::ArrayType::Int),
    );
    let _ = code.emit(
        java::bytecode::Opcode::AReturn,
        java::bytecode::Operand::None,
    );
    let built = code.finish()?;
    let (attribute, _) = java::classfile::CodeAttribute::from_built_analyzed(
        &mut class.constant_pool,
        "sample/CrossIsa",
        "array",
        "(I)[I",
        flags,
        &built,
    )?;
    class.methods[method_position]
        .attributes
        .push(java::classfile::Attribute::Code(attribute));
    let function =
        java::mlil::lift_method(&class, &class.methods[method_position])?.expect("method has code");

    let mut file = dex::DexFile::new(dex::DexVersion::V040);
    let method_name = file.push_string(dex::file::DexString::new("array"))?;
    let owner_text = file.push_string(dex::file::DexString::new("Lsample/CrossIsa;"))?;
    let int_text = file.push_string(dex::file::DexString::new("I"))?;
    let array_text = file.push_string(dex::file::DexString::new("[I"))?;
    let shorty_text = file.push_string(dex::file::DexString::new("LI"))?;
    let owner = file.push_type(dex::file::TypeId {
        descriptor: owner_text,
    })?;
    let int = file.push_type(dex::file::TypeId {
        descriptor: int_text,
    })?;
    let array = file.push_type(dex::file::TypeId {
        descriptor: array_text,
    })?;
    let prototype = file.push_prototype(dex::file::PrototypeId {
        shorty: shorty_text,
        return_type: array,
        parameters: vec![int],
        parameters_offset: 0,
    })?;
    let method = file.push_method(dex::file::MethodId {
        class: owner,
        prototype,
        name: method_name,
    })?;
    let declaration = dex::file::EncodedMethod {
        method,
        access_flags: dex::file::AccessFlags::STATIC,
        code: None,
    };
    Ok((file, declaration, function))
}

fn jvm_constructor_fixture()
-> Result<(dex::DexFile, dex::file::EncodedMethod, mlil::Function), Box<dyn std::error::Error>> {
    const OWNER: &str = "sample/CrossIsa";
    const DEX_OWNER: &str = "Lsample/CrossIsa;";
    let mut class = java::classfile::ClassFile::new(
        java::classfile::JAVA_8_MAJOR_VERSION,
        OWNER,
        Some("java/lang/Object"),
        java::classfile::ClassAccessFlags::PUBLIC,
    )?;
    let flags =
        java::classfile::MethodAccessFlags::PUBLIC | java::classfile::MethodAccessFlags::STATIC;
    let method_position = class.add_method(flags, "make", "()Lsample/CrossIsa;")?;
    let class_index = class.constant_pool.intern_class(OWNER)?;
    let constructor = class
        .constant_pool
        .intern_method_ref(OWNER, "<init>", "()V")?;
    let mut code = java::bytecode::CodeBuilder::new();
    let _ = code.emit(
        java::bytecode::Opcode::New,
        java::bytecode::Operand::Constant(class_index),
    );
    let _ = code.emit(java::bytecode::Opcode::Dup, java::bytecode::Operand::None);
    let _ = code.emit(
        java::bytecode::Opcode::InvokeSpecial,
        java::bytecode::Operand::Constant(constructor),
    );
    let _ = code.emit(
        java::bytecode::Opcode::AReturn,
        java::bytecode::Operand::None,
    );
    let built = code.finish()?;
    let (attribute, _) = java::classfile::CodeAttribute::from_built_analyzed(
        &mut class.constant_pool,
        OWNER,
        "make",
        "()Lsample/CrossIsa;",
        flags,
        &built,
    )?;
    class.methods[method_position]
        .attributes
        .push(java::classfile::Attribute::Code(attribute));
    let function =
        java::mlil::lift_method(&class, &class.methods[method_position])?.expect("method has code");

    let mut builder = dex::file::DexBuilder::new(dex::DexVersion::V040);
    let _ = builder.intern_type(DEX_OWNER)?;
    let _ = builder.intern_method_named(DEX_OWNER, "<init>", "V", &[])?;
    let make = builder.intern_method_named(DEX_OWNER, "make", DEX_OWNER, &[])?;
    let built = builder.build()?;
    let declaration = dex::file::EncodedMethod {
        method: built.indices.method(make).expect("make was interned"),
        access_flags: dex::file::AccessFlags::STATIC,
        code: None,
    };
    Ok((built.file, declaration, function))
}

fn jvm_polymorphic_fixture()
-> Result<(dex::DexFile, dex::file::EncodedMethod, mlil::Function), Box<dyn std::error::Error>> {
    const OWNER: &str = "sample/CrossIsa";
    const DEX_OWNER: &str = "Lsample/CrossIsa;";
    const METHOD_HANDLE: &str = "java/lang/invoke/MethodHandle";
    const DEX_METHOD_HANDLE: &str = "Ljava/lang/invoke/MethodHandle;";
    const DESCRIPTOR: &str = "(Ljava/lang/invoke/MethodHandle;I)I";
    let mut class = java::classfile::ClassFile::new(
        java::classfile::JAVA_8_MAJOR_VERSION,
        OWNER,
        Some("java/lang/Object"),
        java::classfile::ClassAccessFlags::PUBLIC,
    )?;
    let flags =
        java::classfile::MethodAccessFlags::PUBLIC | java::classfile::MethodAccessFlags::STATIC;
    let method_position = class.add_method(flags, "invoke", DESCRIPTOR)?;
    let target = class
        .constant_pool
        .intern_method_ref(METHOD_HANDLE, "invokeExact", "(I)I")?;
    let mut code = java::bytecode::CodeBuilder::new();
    let _ = code.emit(
        java::bytecode::Opcode::ALoad0,
        java::bytecode::Operand::None,
    );
    let _ = code.emit(
        java::bytecode::Opcode::ILoad1,
        java::bytecode::Operand::None,
    );
    let _ = code.emit(
        java::bytecode::Opcode::InvokeVirtual,
        java::bytecode::Operand::Constant(target),
    );
    let _ = code.emit(
        java::bytecode::Opcode::IReturn,
        java::bytecode::Operand::None,
    );
    let built = code.finish()?;
    let (attribute, _) = java::classfile::CodeAttribute::from_built_analyzed(
        &mut class.constant_pool,
        OWNER,
        "invoke",
        DESCRIPTOR,
        flags,
        &built,
    )?;
    class.methods[method_position]
        .attributes
        .push(java::classfile::Attribute::Code(attribute));
    let function =
        java::mlil::lift_method(&class, &class.methods[method_position])?.expect("method has code");

    let mut builder = dex::file::DexBuilder::new(dex::DexVersion::V040);
    let int = builder.intern_type("I")?;
    let _ = builder.intern_prototype(int, [int])?;
    let _ = builder.intern_method_named(
        DEX_METHOD_HANDLE,
        "invokeExact",
        "Ljava/lang/Object;",
        &["[Ljava/lang/Object;"],
    )?;
    let caller =
        builder.intern_method_named(DEX_OWNER, "invoke", "I", &[DEX_METHOD_HANDLE, "I"])?;
    let built = builder.build()?;
    let declaration = dex::file::EncodedMethod {
        method: built.indices.method(caller).expect("caller was interned"),
        access_flags: dex::file::AccessFlags::STATIC,
        code: None,
    };
    Ok((built.file, declaration, function))
}

fn dex_exception_fixture()
-> Result<(dex::DexFile, dex::file::EncodedMethod, mlil::Function), Box<dyn std::error::Error>> {
    let mut file = dex::DexFile::new(dex::DexVersion::V040);
    let method_name = file.push_string(dex::file::DexString::new("guarded"))?;
    let owner_text = file.push_string(dex::file::DexString::new("Lsample/CrossIsa;"))?;
    let void_text = file.push_string(dex::file::DexString::new("V"))?;
    let constant = file.push_string(dex::file::DexString::new("value"))?;
    let owner = file.push_type(dex::file::TypeId {
        descriptor: owner_text,
    })?;
    let void = file.push_type(dex::file::TypeId {
        descriptor: void_text,
    })?;
    let prototype = file.push_prototype(dex::file::PrototypeId {
        shorty: void_text,
        return_type: void,
        parameters: Vec::new(),
        parameters_offset: 0,
    })?;
    let method = file.push_method(dex::file::MethodId {
        class: owner,
        prototype,
        name: method_name,
    })?;
    let declaration = dex::file::EncodedMethod {
        method,
        access_flags: dex::file::AccessFlags::STATIC,
        code: Some(dex::file::CodeItem {
            registers_size: 1,
            ins_size: 0,
            outs_size: 0,
            instructions: vec![
                dex::instruction::Instruction::operation(
                    0,
                    dex::instruction::Opcode::ConstString,
                    dex::instruction::Operands::RegisterIndex {
                        register: 0,
                        index: constant.get(),
                    },
                ),
                dex::instruction::Instruction::operation(
                    2,
                    dex::instruction::Opcode::ReturnVoid,
                    dex::instruction::Operands::None,
                ),
                dex::instruction::Instruction::operation(
                    3,
                    dex::instruction::Opcode::MoveException,
                    dex::instruction::Operands::Register(0),
                ),
                dex::instruction::Instruction::operation(
                    4,
                    dex::instruction::Opcode::Throw,
                    dex::instruction::Operands::Register(0),
                ),
            ],
            tries: vec![dex::file::TryBlock {
                start_address: 0,
                instruction_count: 2,
                handlers: vec![dex::file::CatchHandler {
                    exception_type: None,
                    address: 3,
                }],
            }],
            debug_info: None,
            data_offset: 0,
        }),
    };
    let function = dex::mlil::lift_method(&file, &declaration)?.expect("exception method has code");
    Ok((file, declaration, function))
}

fn operations(function: &mlil::Function) -> Vec<mlil::Operation> {
    function
        .cfg()
        .blocks()
        .iter()
        .flat_map(disassembler::cfglib::BasicBlock::instructions)
        .map(|instruction| instruction.operation().clone())
        .collect()
}

#[test]
fn jvm_and_dex_lift_to_the_same_semantics() -> Result<(), Box<dyn std::error::Error>> {
    let jvm = jvm_constant_function()?;
    let dex = dex_constant_function()?;

    assert_eq!(jvm.source().format, disassembler::BinaryFormat::JavaClass);
    assert_eq!(dex.source().format, disassembler::BinaryFormat::Dex);
    assert_eq!(
        operations(&jvm),
        vec![
            mlil::Operation::Constant(mlil::Constant::Integer(1)),
            mlil::Operation::Return,
        ]
    );
    assert_eq!(operations(&jvm), operations(&dex));
    assert!(jvm.verify().is_ok());
    assert!(dex.verify().is_ok());
    jvm.ssa()?;
    dex.ssa()?;
    Ok(())
}

#[test]
fn cafe_exposes_verified_same_isa_mlil_round_trips() -> Result<(), Box<dyn std::error::Error>> {
    let (mut class, method_position, jvm) = jvm_constant_fixture()?;
    let jvm_lowered = java::mlil::lower_body(&jvm, &mut class.constant_pool)?;
    jvm_lowered.body.verify()?;
    assert!(!jvm_lowered.source_map.is_empty());
    let jvm_relifted = java::mlil::lift_body(
        &class.constant_pool,
        "sample/CrossIsa",
        "value",
        "()I",
        class.methods[method_position].access_flags,
        &jvm_lowered.body,
    )?;
    assert!(jvm_relifted.verify().is_ok());
    assert!(
        operations(&jvm_relifted).contains(&mlil::Operation::Constant(mlil::Constant::Integer(1)))
    );

    let (file, declaration, dex_function) = dex_constant_fixture()?;
    let dex_lowered = dex::mlil::lower_body(&file, &dex_function)?;
    dex_lowered.body.verify()?;
    assert!(!dex_lowered.source_map.is_empty());
    let dex_relifted = dex::mlil::lift_body(&file, &declaration, &dex_lowered.body)?;
    assert!(dex_relifted.verify().is_ok());
    assert!(
        operations(&dex_relifted).contains(&mlil::Operation::Constant(mlil::Constant::Integer(1)))
    );
    Ok(())
}

#[test]
fn mlil_retargets_between_jvm_and_dalvik_llil() -> Result<(), Box<dyn std::error::Error>> {
    let (mut class, method_position, jvm) = jvm_constant_fixture()?;
    let (file, declaration, dalvik) = dex_constant_fixture()?;

    let dalvik_lowered = dex::mlil::lower_body(&file, &jvm)?;
    dalvik_lowered.body.verify()?;
    assert_eq!(
        dalvik_lowered.source_map.source().format,
        disassembler::BinaryFormat::JavaClass
    );
    assert_eq!(
        dalvik_lowered.source_map.generated().format,
        disassembler::BinaryFormat::Dex
    );
    let dalvik_relifted = dex::mlil::lift_body(&file, &declaration, &dalvik_lowered.body)?;
    assert!(dalvik_relifted.verify().is_ok());
    assert!(
        operations(&dalvik_relifted)
            .contains(&mlil::Operation::Constant(mlil::Constant::Integer(1)))
    );

    let jvm_lowered = java::mlil::lower_body(&dalvik, &mut class.constant_pool)?;
    jvm_lowered.body.verify()?;
    assert_eq!(
        jvm_lowered.source_map.source().format,
        disassembler::BinaryFormat::Dex
    );
    assert_eq!(
        jvm_lowered.source_map.generated().format,
        disassembler::BinaryFormat::JavaClass
    );
    let jvm_relifted = java::mlil::lift_body(
        &class.constant_pool,
        "sample/CrossIsa",
        "value",
        "()I",
        class.methods[method_position].access_flags,
        &jvm_lowered.body,
    )?;
    assert!(jvm_relifted.verify().is_ok());
    assert!(
        operations(&jvm_relifted).contains(&mlil::Operation::Constant(mlil::Constant::Integer(1)))
    );
    Ok(())
}

#[test]
fn cross_isa_lowering_preserves_reference_parameters() -> Result<(), Box<dyn std::error::Error>> {
    let jvm = jvm_reference_identity()?;
    let (file, declaration, dalvik) = dex_reference_identity_fixture()?;

    let dalvik_lowered = dex::mlil::lower_body(&file, &jvm)?;
    dalvik_lowered.body.verify()?;
    let dalvik_relifted = dex::mlil::lift_body(&file, &declaration, &dalvik_lowered.body)?;
    let dalvik_return = dalvik_relifted
        .cfg()
        .blocks()
        .iter()
        .flat_map(disassembler::cfglib::BasicBlock::instructions)
        .find(|instruction| matches!(instruction.operation(), mlil::Operation::Return))
        .expect("retargeted Dalvik body returns");
    assert!(dalvik_return.use_types()[0].is_reference());

    let mut pool = java::classfile::ConstantPool::new();
    let jvm_lowered = java::mlil::lower_body(&dalvik, &mut pool)?;
    jvm_lowered.body.verify()?;
    let jvm_relifted = java::mlil::lift_body(
        &pool,
        "sample/CrossIsa",
        "identity",
        "(Lsample/CrossIsa;)Lsample/CrossIsa;",
        java::classfile::MethodAccessFlags::PUBLIC | java::classfile::MethodAccessFlags::STATIC,
        &jvm_lowered.body,
    )?;
    let jvm_return = jvm_relifted
        .cfg()
        .blocks()
        .iter()
        .flat_map(disassembler::cfglib::BasicBlock::instructions)
        .find(|instruction| matches!(instruction.operation(), mlil::Operation::Return))
        .expect("retargeted JVM body returns");
    assert!(jvm_return.use_types()[0].is_reference());
    Ok(())
}

#[test]
fn dalvik_zero_is_legalized_as_jvm_null_at_reference_uses() -> Result<(), Box<dyn std::error::Error>>
{
    let function = dex_null_function()?;
    let mut pool = java::classfile::ConstantPool::new();
    let lowered = java::mlil::lower_body(&function, &mut pool)?;

    lowered.body.verify()?;
    let relifted = java::mlil::lift_body(
        &pool,
        "sample/CrossIsa",
        "nullable",
        "()Lsample/CrossIsa;",
        java::classfile::MethodAccessFlags::PUBLIC | java::classfile::MethodAccessFlags::STATIC,
        &lowered.body,
    )?;
    assert!(operations(&relifted).contains(&mlil::Operation::Constant(mlil::Constant::Null)));
    let returned = relifted
        .cfg()
        .blocks()
        .iter()
        .flat_map(disassembler::cfglib::BasicBlock::instructions)
        .find(|instruction| matches!(instruction.operation(), mlil::Operation::Return))
        .expect("retargeted JVM body returns");
    assert!(returned.use_types()[0].is_reference());
    Ok(())
}

#[test]
fn cross_isa_lowering_legalizes_native_array_forms() -> Result<(), Box<dyn std::error::Error>> {
    let fill_array = dex_fill_array_function()?;
    assert!(operations(&fill_array).iter().any(|operation| {
        matches!(
            operation,
            mlil::Operation::InitializeArray { array_type, values }
                if array_type.descriptor() == "[I"
                    && values
                        == &[mlil::Constant::Integer(1), mlil::Constant::Integer(2)]
        )
    }));
    let mut pool = java::classfile::ConstantPool::new();
    let jvm_lowered = java::mlil::lower_body(&fill_array, &mut pool)?;
    jvm_lowered.body.verify()?;
    let jvm_relifted = java::mlil::lift_body(
        &pool,
        "sample/CrossIsa",
        "fill",
        "([I)V",
        java::classfile::MethodAccessFlags::PUBLIC | java::classfile::MethodAccessFlags::STATIC,
        &jvm_lowered.body,
    )?;
    assert_eq!(
        operations(&jvm_relifted)
            .iter()
            .filter(|operation| {
                matches!(
                    operation,
                    mlil::Operation::Array {
                        access: mlil::ArrayAccess::Put,
                        ..
                    }
                )
            })
            .count(),
        2
    );

    let (file, declaration, new_array) = jvm_new_array_fixture()?;
    let dalvik_lowered = dex::mlil::lower_body(&file, &new_array)?;
    dalvik_lowered.body.verify()?;
    let dalvik_relifted = dex::mlil::lift_body(&file, &declaration, &dalvik_lowered.body)?;
    assert!(operations(&dalvik_relifted).iter().any(|operation| {
        matches!(
            operation,
            mlil::Operation::Allocate(mlil::AllocationKind::Array { array_type, .. })
                if array_type.descriptor() == "[I"
        )
    }));
    Ok(())
}

#[test]
fn cross_isa_lowering_uses_semantic_direct_dispatch() -> Result<(), Box<dyn std::error::Error>> {
    let (file, declaration, jvm) = jvm_constructor_fixture()?;
    assert!(operations(&jvm).iter().any(|operation| {
        matches!(
            operation,
            mlil::Operation::Call {
                kind: mlil::CallKind::Direct,
                ..
            }
        )
    }));

    let dalvik_lowered = dex::mlil::lower_body(&file, &jvm)?;
    dalvik_lowered.body.verify()?;
    let dalvik_relifted = dex::mlil::lift_body(&file, &declaration, &dalvik_lowered.body)?;
    assert!(operations(&dalvik_relifted).iter().any(|operation| {
        matches!(
            operation,
            mlil::Operation::Call {
                kind: mlil::CallKind::Direct,
                ..
            }
        )
    }));

    let mut pool = java::classfile::ConstantPool::new();
    let jvm_lowered = java::mlil::lower_body(&dalvik_relifted, &mut pool)?;
    jvm_lowered.body.verify()?;
    let jvm_relifted = java::mlil::lift_body(
        &pool,
        "sample/CrossIsa",
        "make",
        "()Lsample/CrossIsa;",
        java::classfile::MethodAccessFlags::PUBLIC | java::classfile::MethodAccessFlags::STATIC,
        &jvm_lowered.body,
    )?;
    assert!(operations(&jvm_relifted).iter().any(|operation| {
        matches!(
            operation,
            mlil::Operation::Call {
                kind: mlil::CallKind::Direct,
                ..
            }
        )
    }));
    Ok(())
}

#[test]
fn cross_isa_lowering_preserves_signature_polymorphism() -> Result<(), Box<dyn std::error::Error>> {
    const DESCRIPTOR: &str = "(Ljava/lang/invoke/MethodHandle;I)I";
    let (file, declaration, jvm) = jvm_polymorphic_fixture()?;
    assert!(operations(&jvm).iter().any(|operation| {
        matches!(
            operation,
            mlil::Operation::Call {
                kind: mlil::CallKind::Polymorphic,
                descriptor: Some(descriptor),
                ..
            } if descriptor == "(I)I"
        )
    }));

    let dalvik = dex::mlil::lower_body(&file, &jvm)?;
    dalvik.body.verify()?;
    assert!(dalvik.body.instructions.iter().any(|instruction| {
        instruction.encoding.data.opcode() == Some(dex::instruction::Opcode::InvokePolymorphicRange)
    }));
    let relifted = dex::mlil::lift_body(&file, &declaration, &dalvik.body)?;
    assert!(operations(&relifted).iter().any(|operation| {
        matches!(
            operation,
            mlil::Operation::Call {
                kind: mlil::CallKind::Polymorphic,
                descriptor: Some(descriptor),
                ..
            } if descriptor == "(I)I"
        )
    }));

    let mut pool = java::classfile::ConstantPool::new();
    let jvm_lowered = java::mlil::lower_body(&relifted, &mut pool)?;
    jvm_lowered.body.verify()?;
    let jvm_relifted = java::mlil::lift_body(
        &pool,
        "sample/CrossIsa",
        "invoke",
        DESCRIPTOR,
        java::classfile::MethodAccessFlags::PUBLIC | java::classfile::MethodAccessFlags::STATIC,
        &jvm_lowered.body,
    )?;
    assert!(operations(&jvm_relifted).iter().any(|operation| {
        matches!(
            operation,
            mlil::Operation::Call {
                kind: mlil::CallKind::Polymorphic,
                descriptor: Some(descriptor),
                ..
            } if descriptor == "(I)I"
        )
    }));
    Ok(())
}

#[test]
fn cross_isa_lowering_preserves_exception_dispatch() -> Result<(), Box<dyn std::error::Error>> {
    let (file, declaration, dalvik) = dex_exception_fixture()?;
    let exception_edges = |function: &mlil::Function| {
        function
            .cfg()
            .edges()
            .filter(|edge| matches!(edge.payload().role, mlil::EdgeRole::Exception { .. }))
            .count()
    };
    assert_eq!(exception_edges(&dalvik), 1);

    let mut pool = java::classfile::ConstantPool::new();
    let jvm = java::mlil::lower_body(&dalvik, &mut pool)?;
    jvm.body.verify()?;
    let jvm_relifted = java::mlil::lift_body(
        &pool,
        "sample/CrossIsa",
        "guarded",
        "()V",
        java::classfile::MethodAccessFlags::PUBLIC | java::classfile::MethodAccessFlags::STATIC,
        &jvm.body,
    )?;
    assert_eq!(exception_edges(&jvm_relifted), 1);

    let lowered_back = dex::mlil::lower_body(&file, &jvm_relifted)?;
    lowered_back.body.verify()?;
    let relifted_back = dex::mlil::lift_body(&file, &declaration, &lowered_back.body)?;
    assert_eq!(exception_edges(&relifted_back), 1);
    Ok(())
}

#[test]
fn jvm_and_dex_exact_reference_types_share_descriptor_spelling()
-> Result<(), Box<dyn std::error::Error>> {
    let expected = mlil::ValueType::Reference(Some("Lsample/CrossIsa;".to_owned()));
    for function in [jvm_reference_identity()?, dex_reference_identity()?] {
        let returned = function
            .cfg()
            .blocks()
            .iter()
            .flat_map(disassembler::cfglib::BasicBlock::instructions)
            .find(|instruction| matches!(instruction.operation(), mlil::Operation::Return))
            .expect("identity method returns a value");
        assert_eq!(returned.use_types(), std::slice::from_ref(&expected));
        assert!(function.verify().is_ok());
        function.ssa()?;
    }
    Ok(())
}

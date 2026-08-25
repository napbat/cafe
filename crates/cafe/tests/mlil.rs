//! Cross-ISA proof that Cafe exposes one verified semantic IL.

use cafe::{dex, disassembler, java, mlil};

fn jvm_constant_function() -> Result<mlil::Function, Box<dyn std::error::Error>> {
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
    Ok(java::mlil::lift_method(&class, &class.methods[method_position])?.expect("method has code"))
}

fn dex_constant_function() -> Result<mlil::Function, Box<dyn std::error::Error>> {
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
    Ok(dex::mlil::lift_method(&file, &declaration)?.expect("method has code"))
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

fn dex_reference_identity() -> Result<mlil::Function, Box<dyn std::error::Error>> {
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
    Ok(dex::mlil::lift_method(&file, &declaration)?.expect("method has code"))
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

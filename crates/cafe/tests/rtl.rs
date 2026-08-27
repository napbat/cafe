//! Cross-ISA proof for the LLIL/RTL/MLIL bridge exposed by Cafe.

use cafe::cfglib::InstrInfo;
use cafe::{dex, disassembler, java, mlil};

fn jvm_fixture() -> Result<(java::classfile::ClassFile, usize), Box<dyn std::error::Error>> {
    let mut class = java::classfile::ClassFile::new(
        java::classfile::JAVA_8_MAJOR_VERSION,
        "sample/CrossIsa",
        Some("java/lang/Object"),
        java::classfile::ClassAccessFlags::PUBLIC,
    )?;
    let flags =
        java::classfile::MethodAccessFlags::PUBLIC | java::classfile::MethodAccessFlags::STATIC;
    let method = class.add_method(flags, "value", "()I")?;
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
    let (attribute, _) = java::classfile::CodeAttribute::from_built_analyzed(
        &mut class.constant_pool,
        "sample/CrossIsa",
        "value",
        "()I",
        flags,
        &built,
    )?;
    class.methods[method]
        .attributes
        .push(java::classfile::Attribute::Code(attribute));
    Ok((class, method))
}

fn dex_fixture() -> Result<(dex::DexFile, dex::file::EncodedMethod), Box<dyn std::error::Error>> {
    let mut file = dex::DexFile::new(dex::DexVersion::V040);
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
    Ok((
        file,
        dex::file::EncodedMethod {
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
        },
    ))
}

fn definitions<D: cafe::cfglib::ir::rtl::Dialect>(
    function: &cafe::cfglib::ir::rtl::Function<D>,
) -> impl Iterator<Item = &cafe::cfglib::ir::rtl::Lane<D>> {
    function
        .cfg()
        .blocks()
        .iter()
        .flat_map(disassembler::cfglib::BasicBlock::instructions)
        .flat_map(InstrInfo::defs)
}

#[test]
fn llil_rtl_mlil_retargets_in_both_directions() -> Result<(), Box<dyn std::error::Error>> {
    let (mut class, method) = jvm_fixture()?;
    let (file, declaration) = dex_fixture()?;

    let source_jvm =
        java::rtl::lift_method(&class, &class.methods[method])?.expect("JVM fixture has code");
    assert!(
        definitions(&source_jvm)
            .any(|(storage, _)| matches!(storage, java::rtl::JvmStorage::SourceStack(_)))
    );
    let canonical_jvm = java::rtl::raise_function(&source_jvm)?;
    let target_dex = dex::rtl::lower_function(&canonical_jvm)?.function;
    assert!(
        definitions(&target_dex)
            .any(|(storage, _)| matches!(storage, dex::rtl::DexStorage::GeneratedRegister(_)))
    );
    let dex_llil = dex::rtl::lower_body(&file, &target_dex)?;
    dex_llil.body.verify()?;
    let relifted_dex = dex::rtl::lift_body(&file, &declaration, &dex_llil.body)?;
    let canonical_dex = dex::rtl::raise_function(&relifted_dex)?;
    assert!(canonical_dex.verify().is_ok());
    assert!(
        canonical_dex
            .instructions()
            .any(|instruction| matches!(instruction.operation(), mlil::Operation::Constant(_)))
    );

    let source_dex = dex::rtl::lift_method(&file, &declaration)?.expect("Dalvik fixture has code");
    assert!(
        definitions(&source_dex)
            .any(|(storage, _)| matches!(storage, dex::rtl::DexStorage::SourceRegister(_)))
    );
    let canonical_dex = dex::rtl::raise_function(&source_dex)?;
    let target_jvm = java::rtl::lower_function(&canonical_dex)?.function;
    assert!(
        definitions(&target_jvm)
            .any(|(storage, _)| matches!(storage, java::rtl::JvmStorage::GeneratedLocal(_)))
    );
    let jvm_llil = java::rtl::lower_body(&target_jvm, &mut class.constant_pool)?;
    jvm_llil.body.verify()?;
    let relifted_jvm = java::rtl::lift_body(
        &class.constant_pool,
        "sample/CrossIsa",
        "value",
        "()I",
        class.methods[method].access_flags,
        &jvm_llil.body,
    )?;
    assert!(java::rtl::raise_function(&relifted_jvm)?.verify().is_ok());
    Ok(())
}

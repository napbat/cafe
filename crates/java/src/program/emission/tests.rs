use disassembler::{
    AddressUnit, CodeAddress, CodeSize, FunctionBody, Instruction as SharedInstruction,
    InstructionFlow, RawAccessFlags,
};
use program::{MethodDefinition, MethodId, Module, ModuleId, TypeDefinition, TypeId};

use super::*;
use crate::bytecode::Opcode;

fn instruction(address: u32, opcode: Opcode, flow: InstructionFlow) -> SharedInstruction {
    SharedInstruction::new(
        CodeAddress::from(address),
        CodeSize::new(1),
        u32::from(opcode.byte()),
        opcode.mnemonic(),
        Vec::new(),
        flow,
    )
}

#[test]
fn emits_valid_class_files_and_round_trips_executable_bodies() {
    let mut module = Module::new(ModuleId::new(BinaryFormat::JavaClass, "generated")).unwrap();
    let mut class = TypeDefinition::new(
        TypeId::new(BinaryFormat::JavaClass, "sample/Generated"),
        RawAccessFlags::new(u32::from(
            (ClassAccessFlags::PUBLIC | ClassAccessFlags::SUPER).bits(),
        )),
    )
    .unwrap();
    class.set_superclass(Some(TypeId::new(
        BinaryFormat::JavaClass,
        "java/lang/Object",
    )));
    class
        .insert_method(
            MethodDefinition::new(
                MethodId::new("value", "()I"),
                RawAccessFlags::new(u32::from(
                    (MethodAccessFlags::PUBLIC | MethodAccessFlags::STATIC).bits(),
                )),
                Some(FunctionBody::new(
                    AddressUnit::Byte,
                    vec![
                        instruction(0, Opcode::IConst1, InstructionFlow::FallThrough),
                        instruction(1, Opcode::IReturn, InstructionFlow::Return),
                    ],
                    Vec::new(),
                )),
            )
            .unwrap(),
        )
        .unwrap();
    module.insert_type(class).unwrap();

    let classes = emit_module(&module).unwrap();
    assert_eq!(classes.len(), 1);
    classes[0].validate().unwrap();
    let reparsed = ClassFile::parse(&classes[0].to_bytes().unwrap()).unwrap();
    assert_eq!(reparsed.class_name().unwrap(), "sample/Generated");
    assert_eq!(
        reparsed
            .method("value", "()I")
            .unwrap()
            .unwrap()
            .code()
            .unwrap()
            .instructions()
            .unwrap()
            .iter()
            .map(|instruction| instruction.opcode)
            .collect::<Vec<_>>(),
        vec![Opcode::IConst1, Opcode::IReturn]
    );
}

#[test]
fn rejects_a_dex_module_without_partial_native_output() {
    let module = Module::new(ModuleId::new(BinaryFormat::Dex, "classes.dex")).unwrap();
    assert!(matches!(
        emit_module(&module),
        Err(JavaEmissionError::WrongFormat { .. })
    ));
}

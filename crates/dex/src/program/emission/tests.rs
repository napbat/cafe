use disassembler::{
    AddressUnit, BinaryFormat, CodeAddress, CodeSize, ExactText, FunctionBody, Immediate,
    Instruction as SharedInstruction, InstructionFlow, Operand, RawAccessFlags, Reference,
    ReferenceKind, ReferenceSymbol, RegisterResources,
};
use program::{MethodDefinition, MethodId, Module, ModuleId, TypeDefinition, TypeId};

use super::*;

fn instruction(
    address: u32,
    size: u32,
    opcode: Opcode,
    operands: Vec<Operand>,
    flow: InstructionFlow,
) -> SharedInstruction {
    SharedInstruction::new(
        CodeAddress::from(address),
        CodeSize::new(size),
        u32::from(opcode.byte()),
        opcode.mnemonic(),
        operands,
        flow,
    )
}

fn module_with_body(body: FunctionBody, descriptor: &str) -> Module {
    let mut module = Module::new(ModuleId::new(BinaryFormat::Dex, "generated.dex")).unwrap();
    let mut class = TypeDefinition::new(
        TypeId::new(BinaryFormat::Dex, "Lsample/Generated;"),
        RawAccessFlags::new(AccessFlags::PUBLIC.bits()),
    )
    .unwrap();
    class.set_superclass(Some(TypeId::new(BinaryFormat::Dex, "Ljava/lang/Object;")));
    class
        .insert_method(
            MethodDefinition::new(
                MethodId::new("value", descriptor),
                RawAccessFlags::new(AccessFlags::PUBLIC.bits() | AccessFlags::STATIC.bits()),
                Some(body),
            )
            .unwrap(),
        )
        .unwrap();
    module.insert_type(class).unwrap();
    module
}

#[test]
fn emits_and_reparses_an_executable_dex_file() {
    let body = FunctionBody::new(
        AddressUnit::CodeUnit16,
        vec![
            instruction(
                0,
                2,
                Opcode::Const16,
                vec![
                    Operand::Register(0),
                    Operand::Immediate(Immediate::Signed(42)),
                ],
                InstructionFlow::FallThrough,
            ),
            instruction(
                2,
                1,
                Opcode::Return,
                vec![Operand::Register(0)],
                InstructionFlow::Return,
            ),
        ],
        Vec::new(),
    )
    .with_register_resources(RegisterResources::new(1, 0, 0));
    let file = emit_module(&module_with_body(body, "()I")).unwrap();
    let reparsed = DexFile::parse(&file.to_bytes().unwrap()).unwrap();
    let class = reparsed.classes().first().unwrap();
    let code = &class.class_data.as_ref().unwrap().direct_methods[0]
        .code
        .as_ref()
        .unwrap();
    assert_eq!(
        (code.registers_size, code.ins_size, code.outs_size),
        (1, 0, 0)
    );
    assert_eq!(
        code.instructions
            .iter()
            .filter_map(|instruction| instruction.data().opcode())
            .collect::<Vec<_>>(),
        vec![Opcode::Const16, Opcode::Return]
    );
}

#[test]
fn rebuilds_exact_utf16_string_references_without_source_indices() {
    let units = vec![u16::from(b'a'), 0xd800, u16::from(b'b')];
    let reference = Reference::resolved(ReferenceKind::String, u32::MAX, "stale").with_symbol(
        ReferenceSymbol::String(ExactText::from_utf16(units.clone())),
    );
    let body = FunctionBody::new(
        AddressUnit::CodeUnit16,
        vec![
            instruction(
                0,
                2,
                Opcode::ConstString,
                vec![Operand::Register(0), Operand::Reference(reference)],
                InstructionFlow::FallThrough,
            ),
            instruction(
                2,
                1,
                Opcode::ReturnObject,
                vec![Operand::Register(0)],
                InstructionFlow::Return,
            ),
        ],
        Vec::new(),
    )
    .with_register_resources(RegisterResources::new(1, 0, 0));
    let file = emit_module(&module_with_body(body, "()Ljava/lang/String;")).unwrap();
    let code = file.classes()[0]
        .class_data
        .as_ref()
        .unwrap()
        .direct_methods[0]
        .code
        .as_ref()
        .unwrap();
    let index = match code.instructions[0].data() {
        InstructionData::Operation {
            operands: Operands::RegisterIndex { index, .. },
            ..
        } => *index,
        other => panic!("unexpected instruction: {other:?}"),
    };
    assert_eq!(
        file.resolve_string(crate::file::StringIndex::new(index))
            .unwrap()
            .utf16_units,
        units
    );
}

#[test]
fn rejects_a_jvm_module_without_partial_native_output() {
    let module = Module::new(ModuleId::new(BinaryFormat::JavaClass, "Generated.class")).unwrap();
    assert!(matches!(
        emit_module(&module),
        Err(DexEmissionError::WrongFormat { .. })
    ));
}

//! Focused Dalvik LLIL-to-MLIL adapter coverage.

use ::mlil::{EdgeRole, EntityId, Operation, SourceStorage};
use disassembler::CodeAddress;

use super::{lift_body, lift_method, lower_body};
use crate::file::{
    AccessFlags, CatchHandler, CodeItem, DexFile, DexString, DexVersion, EncodedMethod, FieldId,
    MethodId, PrototypeId, TryBlock, TypeId,
};
use crate::instruction::{Instruction, Opcode, Operands, PackedSwitchPayload};

fn arithmetic_fixture() -> (DexFile, EncodedMethod) {
    let mut file = DexFile::new(DexVersion::V040);
    let method_name = file.push_string(DexString::new("work")).unwrap();
    let owner_text = file.push_string(DexString::new("LExample;")).unwrap();
    let int_text = file.push_string(DexString::new("I")).unwrap();
    let owner = file
        .push_type(TypeId {
            descriptor: owner_text,
        })
        .unwrap();
    let int = file
        .push_type(TypeId {
            descriptor: int_text,
        })
        .unwrap();
    let prototype = file
        .push_prototype(PrototypeId {
            shorty: int_text,
            return_type: int,
            parameters: vec![int],
            parameters_offset: 0,
        })
        .unwrap();
    let method = file
        .push_method(MethodId {
            class: owner,
            prototype,
            name: method_name,
        })
        .unwrap();
    let declaration = EncodedMethod {
        method,
        access_flags: AccessFlags::STATIC,
        code: Some(CodeItem {
            registers_size: 2,
            ins_size: 1,
            outs_size: 0,
            instructions: vec![
                Instruction::operation(
                    0,
                    Opcode::Move,
                    Operands::Registers {
                        first: 0,
                        second: 1,
                    },
                ),
                Instruction::operation(
                    1,
                    Opcode::AddInt2Addr,
                    Operands::Registers {
                        first: 0,
                        second: 1,
                    },
                ),
                Instruction::operation(2, Opcode::Return, Operands::Register(0)),
            ],
            tries: Vec::new(),
            debug_info: None,
            data_offset: 0,
        }),
    };
    (file, declaration)
}

#[test]
fn lifts_register_arithmetic_into_generic_variables_and_ssa() {
    let (file, declaration) = arithmetic_fixture();
    let function = lift_method(&file, &declaration).unwrap().unwrap();

    assert!(function.verify().is_ok());
    assert!(function.ssa().is_ok());
    assert!(
        function
            .cfg()
            .blocks()
            .iter()
            .flat_map(disassembler::cfglib::BasicBlock::instructions)
            .any(|instruction| {
                matches!(
                    instruction.operation(),
                    Operation::Binary(::mlil::BinaryOperator::Add)
                )
            })
    );
    assert!(function.variables().iter().any(|variable| {
        variable
            .native
            .is_some_and(|native| matches!(native.storage, SourceStorage::DexRegister(1)))
    }));
}

#[test]
fn lowers_mlil_to_verified_llil_and_relifts_semantics() {
    let (file, declaration) = arithmetic_fixture();
    let function = lift_method(&file, &declaration).unwrap().unwrap();
    let lowered = lower_body(&file, &function).unwrap();

    lowered.body.verify().unwrap();
    assert!(!lowered.source_map.is_empty());
    let relifted = lift_body(&file, &declaration, &lowered.body).unwrap();
    assert!(relifted.verify().is_ok());
    assert!(relifted.cfg().blocks().iter().any(|block| {
        block.instructions().iter().any(|instruction| {
            matches!(
                instruction.operation(),
                Operation::Binary(::mlil::BinaryOperator::Add)
            )
        })
    }));
}

#[test]
fn throwing_register_definitions_commit_after_exception_dispatch() {
    let mut file = DexFile::new(DexVersion::V040);
    let method_name = file.push_string(DexString::new("guarded")).unwrap();
    let owner_text = file.push_string(DexString::new("LExample;")).unwrap();
    let void_text = file.push_string(DexString::new("V")).unwrap();
    let constant = file.push_string(DexString::new("value")).unwrap();
    let owner = file
        .push_type(TypeId {
            descriptor: owner_text,
        })
        .unwrap();
    let void = file
        .push_type(TypeId {
            descriptor: void_text,
        })
        .unwrap();
    let prototype = file
        .push_prototype(PrototypeId {
            shorty: void_text,
            return_type: void,
            parameters: Vec::new(),
            parameters_offset: 0,
        })
        .unwrap();
    let method = file
        .push_method(MethodId {
            class: owner,
            prototype,
            name: method_name,
        })
        .unwrap();
    let declaration = EncodedMethod {
        method,
        access_flags: AccessFlags::STATIC,
        code: Some(CodeItem {
            registers_size: 1,
            ins_size: 0,
            outs_size: 0,
            instructions: vec![
                Instruction::operation(
                    0,
                    Opcode::ConstString,
                    Operands::RegisterIndex {
                        register: 0,
                        index: constant.get(),
                    },
                ),
                Instruction::operation(2, Opcode::ReturnVoid, Operands::None),
                Instruction::operation(3, Opcode::MoveException, Operands::Register(0)),
                Instruction::operation(4, Opcode::Throw, Operands::Register(0)),
            ],
            tries: vec![TryBlock {
                start_address: 0,
                instruction_count: 2,
                handlers: vec![CatchHandler {
                    exception_type: None,
                    address: 3,
                }],
            }],
            debug_info: None,
            data_offset: 0,
        }),
    };
    let function = lift_method(&file, &declaration).unwrap().unwrap();

    assert!(function.verify().is_ok());
    let roles: Vec<_> = function
        .cfg()
        .edges()
        .map(|edge| &edge.payload().role)
        .collect();
    assert_eq!(
        roles
            .iter()
            .filter(|role| matches!(role, EdgeRole::Exception { .. }))
            .count(),
        1
    );
    assert!(roles.contains(&&EdgeRole::Commit));
    assert!(
        function
            .cfg()
            .blocks()
            .iter()
            .flat_map(disassembler::cfglib::BasicBlock::instructions)
            .any(|instruction| matches!(instruction.operation(), Operation::CaughtException(_)))
    );
    function.ssa().unwrap();

    let lowered = lower_body(&file, &function).unwrap();
    lowered.body.verify().unwrap();
    let relifted = lift_body(&file, &declaration, &lowered.body).unwrap();
    assert!(
        relifted
            .cfg()
            .edges()
            .any(|edge| { matches!(edge.payload().role, EdgeRole::Exception { .. }) })
    );
}

#[test]
fn switch_payload_is_fused_into_switch_provenance() {
    let mut file = DexFile::new(DexVersion::V040);
    let method_name = file.push_string(DexString::new("choose")).unwrap();
    let owner_text = file.push_string(DexString::new("LExample;")).unwrap();
    let void_text = file.push_string(DexString::new("V")).unwrap();
    let int_text = file.push_string(DexString::new("I")).unwrap();
    let shorty_text = file.push_string(DexString::new("VI")).unwrap();
    let owner = file
        .push_type(TypeId {
            descriptor: owner_text,
        })
        .unwrap();
    let void = file
        .push_type(TypeId {
            descriptor: void_text,
        })
        .unwrap();
    let int = file
        .push_type(TypeId {
            descriptor: int_text,
        })
        .unwrap();
    let prototype = file
        .push_prototype(PrototypeId {
            shorty: shorty_text,
            return_type: void,
            parameters: vec![int],
            parameters_offset: 0,
        })
        .unwrap();
    let method = file
        .push_method(MethodId {
            class: owner,
            prototype,
            name: method_name,
        })
        .unwrap();
    let declaration = EncodedMethod {
        method,
        access_flags: AccessFlags::STATIC,
        code: Some(CodeItem {
            registers_size: 1,
            ins_size: 1,
            outs_size: 0,
            instructions: vec![
                Instruction::operation(
                    0,
                    Opcode::PackedSwitch,
                    Operands::RegisterBranch {
                        register: 0,
                        target: 4,
                    },
                ),
                Instruction::operation(3, Opcode::ReturnVoid, Operands::None),
                Instruction::packed_switch(
                    4,
                    PackedSwitchPayload {
                        first_key: 7,
                        targets: vec![3],
                    },
                ),
            ],
            tries: Vec::new(),
            debug_info: None,
            data_offset: 0,
        }),
    };
    let function = lift_method(&file, &declaration).unwrap().unwrap();

    assert!(function.verify().is_ok());
    let payload_mappings: Vec<_> = function
        .provenance()
        .mappings_from(CodeAddress::from(4u32))
        .collect();
    assert_eq!(payload_mappings.len(), 1);
    assert!(matches!(
        payload_mappings[0].entity,
        EntityId::Instruction(_)
    ));
    assert!(
        function
            .cfg()
            .edges()
            .any(|edge| { matches!(edge.payload().role, EdgeRole::SwitchCase(7)) })
    );

    let lowered = lower_body(&file, &function).unwrap();
    lowered.body.verify().unwrap();
    let relifted = lift_body(&file, &declaration, &lowered.body).unwrap();
    assert!(
        relifted
            .cfg()
            .edges()
            .any(|edge| { matches!(edge.payload().role, EdgeRole::SwitchCase(7)) })
    );
}

fn memory_fixture() -> (DexFile, EncodedMethod) {
    let mut file = DexFile::new(DexVersion::V040);
    let method_name = file.push_string(DexString::new("memory")).unwrap();
    let field_name = file.push_string(DexString::new("value")).unwrap();
    let owner_text = file.push_string(DexString::new("LExample;")).unwrap();
    let array_text = file.push_string(DexString::new("[I")).unwrap();
    let int_text = file.push_string(DexString::new("I")).unwrap();
    let shorty_text = file.push_string(DexString::new("ILLII")).unwrap();
    let owner = file
        .push_type(TypeId {
            descriptor: owner_text,
        })
        .unwrap();
    let array = file
        .push_type(TypeId {
            descriptor: array_text,
        })
        .unwrap();
    let int = file
        .push_type(TypeId {
            descriptor: int_text,
        })
        .unwrap();
    let prototype = file
        .push_prototype(PrototypeId {
            shorty: shorty_text,
            return_type: int,
            parameters: vec![owner, array, int, int],
            parameters_offset: 0,
        })
        .unwrap();
    let method = file
        .push_method(MethodId {
            class: owner,
            prototype,
            name: method_name,
        })
        .unwrap();
    let field = file
        .push_field(FieldId {
            class: owner,
            field_type: int,
            name: field_name,
        })
        .unwrap();
    let declaration = EncodedMethod {
        method,
        access_flags: AccessFlags::STATIC,
        code: Some(CodeItem {
            registers_size: 5,
            ins_size: 4,
            outs_size: 0,
            instructions: vec![
                Instruction::operation(
                    0,
                    Opcode::Iput,
                    Operands::RegistersIndex {
                        first: 4,
                        second: 1,
                        index: field.get(),
                    },
                ),
                Instruction::operation(
                    2,
                    Opcode::Aput,
                    Operands::ThreeRegisters {
                        first: 4,
                        second: 2,
                        third: 3,
                    },
                ),
                Instruction::operation(
                    4,
                    Opcode::Iget,
                    Operands::RegistersIndex {
                        first: 0,
                        second: 1,
                        index: field.get(),
                    },
                ),
                Instruction::operation(
                    6,
                    Opcode::Aget,
                    Operands::ThreeRegisters {
                        first: 0,
                        second: 2,
                        third: 3,
                    },
                ),
                Instruction::operation(8, Opcode::Return, Operands::Register(0)),
            ],
            tries: Vec::new(),
            debug_info: None,
            data_offset: 0,
        }),
    };
    (file, declaration)
}

#[test]
fn canonicalizes_array_and_field_operand_order() {
    let (file, declaration) = memory_fixture();
    let function = lift_method(&file, &declaration).unwrap().unwrap();

    assert!(function.verify().is_ok());
    for instruction in function
        .cfg()
        .blocks()
        .iter()
        .flat_map(disassembler::cfglib::BasicBlock::instructions)
    {
        let expected = match instruction.operation() {
            Operation::Field {
                access: ::mlil::FieldAccess::PutInstance,
                ..
            } => Some(vec![1, 4]),
            Operation::Array {
                access: ::mlil::ArrayAccess::Put,
                ..
            } => Some(vec![2, 3, 4]),
            _ => None,
        };
        if let Some(expected) = expected {
            let actual = instruction
                .uses()
                .iter()
                .map(|&variable| {
                    let native = function.variable(variable).unwrap().native.unwrap();
                    let SourceStorage::DexRegister(register) = native.storage else {
                        panic!("expected a DEX register operand");
                    };
                    register
                })
                .collect::<Vec<_>>();
            assert_eq!(actual, expected);
        }
    }
    function.ssa().unwrap();

    let lowered = lower_body(&file, &function).unwrap();
    lowered.body.verify().unwrap();
    let relifted = lift_body(&file, &declaration, &lowered.body).unwrap();
    assert!(relifted.cfg().blocks().iter().any(|block| {
        block
            .instructions()
            .iter()
            .any(|instruction| matches!(instruction.operation(), Operation::Array { .. }))
    }));
}

#[test]
#[allow(clippy::too_many_lines)]
fn preserves_invoke_result_as_explicit_state() {
    let mut file = DexFile::new(DexVersion::V040);
    let method_name = file.push_string(DexString::new("recur")).unwrap();
    let owner_text = file.push_string(DexString::new("LExample;")).unwrap();
    let long_text = file.push_string(DexString::new("J")).unwrap();
    let shorty_text = file.push_string(DexString::new("JJ")).unwrap();
    let owner = file
        .push_type(TypeId {
            descriptor: owner_text,
        })
        .unwrap();
    let long = file
        .push_type(TypeId {
            descriptor: long_text,
        })
        .unwrap();
    let prototype = file
        .push_prototype(PrototypeId {
            shorty: shorty_text,
            return_type: long,
            parameters: vec![long],
            parameters_offset: 0,
        })
        .unwrap();
    let method = file
        .push_method(MethodId {
            class: owner,
            prototype,
            name: method_name,
        })
        .unwrap();
    let declaration = EncodedMethod {
        method,
        access_flags: AccessFlags::STATIC,
        code: Some(CodeItem {
            registers_size: 4,
            ins_size: 2,
            outs_size: 2,
            instructions: vec![
                Instruction::operation(
                    0,
                    Opcode::InvokeStatic,
                    Operands::RegisterListIndex {
                        registers: vec![2, 3],
                        index: method.get(),
                        secondary_index: None,
                    },
                ),
                Instruction::operation(3, Opcode::MoveResultWide, Operands::Register(0)),
                Instruction::operation(4, Opcode::ReturnWide, Operands::Register(0)),
            ],
            tries: Vec::new(),
            debug_info: None,
            data_offset: 0,
        }),
    };
    let function = lift_method(&file, &declaration).unwrap().unwrap();

    assert!(function.verify().is_ok());
    let instructions = function
        .cfg()
        .blocks()
        .iter()
        .flat_map(disassembler::cfglib::BasicBlock::instructions)
        .collect::<Vec<_>>();
    let call = instructions
        .iter()
        .find(|instruction| matches!(instruction.operation(), Operation::Call { .. }))
        .unwrap();
    assert_eq!(call.uses().len(), 1);
    assert_eq!(call.use_types(), [::mlil::ValueType::Long]);
    assert_eq!(call.defs().len(), 1);
    assert!(matches!(
        function
            .variable(call.defs()[0])
            .unwrap()
            .native
            .unwrap()
            .storage,
        SourceStorage::DexResult
    ));
    let move_result = instructions
        .iter()
        .find(|instruction| {
            matches!(instruction.operation(), Operation::Copy) && instruction.uses() == call.defs()
        })
        .unwrap();
    assert!(matches!(
        function
            .variable(move_result.defs()[0])
            .unwrap()
            .native
            .unwrap()
            .storage,
        SourceStorage::DexRegister(0)
    ));
    function.ssa().unwrap();

    let lowered = lower_body(&file, &function).unwrap();
    lowered.body.verify().unwrap();
    let relifted = lift_body(&file, &declaration, &lowered.body).unwrap();
    assert!(relifted.cfg().blocks().iter().any(|block| {
        block
            .instructions()
            .iter()
            .any(|instruction| matches!(instruction.operation(), Operation::Call { .. }))
    }));
}

#[test]
fn preserves_exact_reference_type_through_move_result_object() {
    let mut file = DexFile::new(DexVersion::V040);
    let method_name = file.push_string(DexString::new("recurObject")).unwrap();
    let owner_text = file.push_string(DexString::new("LExample;")).unwrap();
    let shorty_text = file.push_string(DexString::new("L")).unwrap();
    let owner = file
        .push_type(TypeId {
            descriptor: owner_text,
        })
        .unwrap();
    let prototype = file
        .push_prototype(PrototypeId {
            shorty: shorty_text,
            return_type: owner,
            parameters: Vec::new(),
            parameters_offset: 0,
        })
        .unwrap();
    let method = file
        .push_method(MethodId {
            class: owner,
            prototype,
            name: method_name,
        })
        .unwrap();
    let declaration = EncodedMethod {
        method,
        access_flags: AccessFlags::STATIC,
        code: Some(CodeItem {
            registers_size: 1,
            ins_size: 0,
            outs_size: 0,
            instructions: vec![
                Instruction::operation(
                    0,
                    Opcode::InvokeStatic,
                    Operands::RegisterListIndex {
                        registers: Vec::new(),
                        index: method.get(),
                        secondary_index: None,
                    },
                ),
                Instruction::operation(3, Opcode::MoveResultObject, Operands::Register(0)),
                Instruction::operation(4, Opcode::ReturnObject, Operands::Register(0)),
            ],
            tries: Vec::new(),
            debug_info: None,
            data_offset: 0,
        }),
    };
    let function = lift_method(&file, &declaration).unwrap().unwrap();

    assert!(function.verify().is_ok());
    let instructions = function
        .cfg()
        .blocks()
        .iter()
        .flat_map(disassembler::cfglib::BasicBlock::instructions)
        .collect::<Vec<_>>();
    let call = instructions
        .iter()
        .find(|instruction| matches!(instruction.operation(), Operation::Call { .. }))
        .unwrap();
    let move_result = instructions
        .iter()
        .find(|instruction| {
            matches!(instruction.operation(), Operation::Copy) && instruction.uses() == call.defs()
        })
        .unwrap();
    assert_eq!(
        call.def_types(),
        [::mlil::ValueType::Reference(Some("LExample;".to_owned()))]
    );
    assert_eq!(
        move_result.def_types(),
        [::mlil::ValueType::Reference(Some("LExample;".to_owned()))]
    );
    function.ssa().unwrap();
}

#[test]
fn preserves_dalvik_zero_as_numeric_or_null() {
    let mut file = DexFile::new(DexVersion::V040);
    let method_name = file.push_string(DexString::new("nullValue")).unwrap();
    let owner_text = file.push_string(DexString::new("LExample;")).unwrap();
    let shorty_text = file.push_string(DexString::new("L")).unwrap();
    let owner = file
        .push_type(TypeId {
            descriptor: owner_text,
        })
        .unwrap();
    let prototype = file
        .push_prototype(PrototypeId {
            shorty: shorty_text,
            return_type: owner,
            parameters: Vec::new(),
            parameters_offset: 0,
        })
        .unwrap();
    let method = file
        .push_method(MethodId {
            class: owner,
            prototype,
            name: method_name,
        })
        .unwrap();
    let declaration = EncodedMethod {
        method,
        access_flags: AccessFlags::STATIC,
        code: Some(CodeItem {
            registers_size: 1,
            ins_size: 0,
            outs_size: 0,
            instructions: vec![
                Instruction::operation(
                    0,
                    Opcode::Const4,
                    Operands::RegisterLiteral {
                        register: 0,
                        literal: 0,
                    },
                ),
                Instruction::operation(1, Opcode::ReturnObject, Operands::Register(0)),
            ],
            tries: Vec::new(),
            debug_info: None,
            data_offset: 0,
        }),
    };
    let function = lift_method(&file, &declaration).unwrap().unwrap();

    assert!(function.verify().is_ok());
    let instructions = function
        .cfg()
        .blocks()
        .iter()
        .flat_map(disassembler::cfglib::BasicBlock::instructions)
        .collect::<Vec<_>>();
    let constant = instructions
        .iter()
        .find(|instruction| matches!(instruction.operation(), Operation::Constant(_)))
        .unwrap();
    let returned = instructions
        .iter()
        .find(|instruction| matches!(instruction.operation(), Operation::Return))
        .unwrap();
    assert_eq!(constant.def_types(), [::mlil::ValueType::Zero]);
    assert_eq!(returned.use_types(), [::mlil::ValueType::Zero]);
    assert!(
        ::mlil::ValueType::Reference(Some("LExample;".to_owned()))
            .accepts(&::mlil::ValueType::Zero)
    );
    function.ssa().unwrap();
}

#[test]
#[allow(clippy::too_many_lines)]
fn treats_two_zero_values_as_integer_operands_for_ordered_branches() {
    let mut file = DexFile::new(DexVersion::V040);
    let method_name = file.push_string(DexString::new("zeroOrder")).unwrap();
    let owner_text = file.push_string(DexString::new("LExample;")).unwrap();
    let void_text = file.push_string(DexString::new("V")).unwrap();
    let owner = file
        .push_type(TypeId {
            descriptor: owner_text,
        })
        .unwrap();
    let void = file
        .push_type(TypeId {
            descriptor: void_text,
        })
        .unwrap();
    let prototype = file
        .push_prototype(PrototypeId {
            shorty: void_text,
            return_type: void,
            parameters: Vec::new(),
            parameters_offset: 0,
        })
        .unwrap();
    let method = file
        .push_method(MethodId {
            class: owner,
            prototype,
            name: method_name,
        })
        .unwrap();
    let zero = |offset, register| {
        Instruction::operation(
            offset,
            Opcode::Const4,
            Operands::RegisterLiteral {
                register,
                literal: 0,
            },
        )
    };
    let declaration = EncodedMethod {
        method,
        access_flags: AccessFlags::STATIC,
        code: Some(CodeItem {
            registers_size: 2,
            ins_size: 0,
            outs_size: 0,
            instructions: vec![
                zero(0, 0),
                zero(1, 1),
                Instruction::operation(
                    2,
                    Opcode::IfLt,
                    Operands::RegistersBranch {
                        first: 0,
                        second: 1,
                        target: 4,
                    },
                ),
                Instruction::operation(4, Opcode::ReturnVoid, Operands::None),
            ],
            tries: Vec::new(),
            debug_info: None,
            data_offset: 0,
        }),
    };
    let function = lift_method(&file, &declaration).unwrap().unwrap();

    assert!(function.verify().is_ok());
    let branch = function
        .cfg()
        .blocks()
        .iter()
        .flat_map(disassembler::cfglib::BasicBlock::instructions)
        .find(|instruction| matches!(instruction.operation(), Operation::Branch(_)))
        .unwrap();
    assert!(matches!(
        branch.operation(),
        Operation::Branch(::mlil::BranchPredicate {
            operands: ::mlil::BranchOperandKind::IntegerPair,
            ..
        })
    ));
    assert_eq!(
        branch.use_types(),
        [::mlil::ValueType::Zero, ::mlil::ValueType::Zero]
    );
    function.ssa().unwrap();

    let lowered = lower_body(&file, &function).unwrap();
    lowered.body.verify().unwrap();
    let relifted = lift_body(&file, &declaration, &lowered.body).unwrap();
    let relifted_branch = relifted
        .cfg()
        .blocks()
        .iter()
        .flat_map(disassembler::cfglib::BasicBlock::instructions)
        .find(|instruction| matches!(instruction.operation(), Operation::Branch(_)))
        .unwrap();
    assert!(matches!(
        relifted_branch.operation(),
        Operation::Branch(::mlil::BranchPredicate {
            operands: ::mlil::BranchOperandKind::IntegerPair,
            ..
        })
    ));
}

#[test]
fn types_unreachable_invocation_words_from_the_descriptor() {
    let mut file = DexFile::new(DexVersion::V040);
    let method_name = file.push_string(DexString::new("unreachableCall")).unwrap();
    let owner_text = file.push_string(DexString::new("LExample;")).unwrap();
    let void_text = file.push_string(DexString::new("V")).unwrap();
    let long_text = file.push_string(DexString::new("J")).unwrap();
    let shorty_text = file.push_string(DexString::new("VJ")).unwrap();
    let owner = file
        .push_type(TypeId {
            descriptor: owner_text,
        })
        .unwrap();
    let void = file
        .push_type(TypeId {
            descriptor: void_text,
        })
        .unwrap();
    let long = file
        .push_type(TypeId {
            descriptor: long_text,
        })
        .unwrap();
    let prototype = file
        .push_prototype(PrototypeId {
            shorty: shorty_text,
            return_type: void,
            parameters: vec![long],
            parameters_offset: 0,
        })
        .unwrap();
    let method = file
        .push_method(MethodId {
            class: owner,
            prototype,
            name: method_name,
        })
        .unwrap();
    let declaration = EncodedMethod {
        method,
        access_flags: AccessFlags::PUBLIC,
        code: Some(CodeItem {
            registers_size: 3,
            ins_size: 3,
            outs_size: 3,
            instructions: vec![
                Instruction::operation(0, Opcode::Goto, Operands::Branch { target: 5 }),
                Instruction::operation(
                    1,
                    Opcode::InvokeVirtual,
                    Operands::RegisterListIndex {
                        registers: vec![0, 1, 2],
                        index: method.get(),
                        secondary_index: None,
                    },
                ),
                Instruction::operation(4, Opcode::ReturnVoid, Operands::None),
                Instruction::operation(5, Opcode::ReturnVoid, Operands::None),
            ],
            tries: Vec::new(),
            debug_info: None,
            data_offset: 0,
        }),
    };
    let function = lift_method(&file, &declaration).unwrap().unwrap();

    assert!(function.verify().is_ok());
    let call = function
        .cfg()
        .blocks()
        .iter()
        .flat_map(disassembler::cfglib::BasicBlock::instructions)
        .find(|instruction| matches!(instruction.operation(), Operation::Call { .. }))
        .unwrap();
    assert_eq!(
        call.use_types(),
        [::mlil::ValueType::Reference(None), ::mlil::ValueType::Long]
    );
    assert_eq!(call.uses().len(), 2);
    function.ssa().unwrap();
}

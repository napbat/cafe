//! Dalvik LLIL/RTL/MLIL integration coverage.

use mlil::{BinaryOperator, EdgeRole, Operation};

use super::{lift_body, lift_method, lower_body, raise_function};
use crate::file::{
    AccessFlags, CatchHandler, CodeItem, DexFile, DexString, DexVersion, EncodedMethod, MethodId,
    PrototypeId, TryBlock, TypeId,
};
use crate::instruction::{Instruction, Opcode, Operands};
use crate::llil;

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
fn dalvik_llil_rtl_mlil_and_back_preserve_semantics() {
    let (file, declaration) = arithmetic_fixture();
    let body = llil::Body::from_code(declaration.code.as_ref().unwrap()).unwrap();
    let rtl = lift_body(&file, &declaration, &body).unwrap();
    assert_eq!(rtl.signature().parameters.len(), 1);
    assert_eq!(rtl.signature().returns.len(), 1);

    let semantic = raise_function(&rtl).unwrap();
    assert!(semantic.verify().is_ok());
    assert!(semantic.instructions().any(|instruction| {
        matches!(
            instruction.operation(),
            Operation::Binary(BinaryOperator::Add)
        )
    }));

    let lowered = lower_body(&file, &rtl).unwrap();
    lowered.body.verify().unwrap();
    let relifted = lift_body(&file, &declaration, &lowered.body).unwrap();
    assert!(raise_function(&relifted).unwrap().verify().is_ok());
}

#[test]
fn dalvik_rtl_preserves_exact_exception_edges_and_regions() {
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

    let rtl = lift_method(&file, &declaration).unwrap().unwrap();
    let exceptional: Vec<_> = rtl
        .cfg()
        .edges()
        .filter(|edge| matches!(edge.payload().role, EdgeRole::Exception { .. }))
        .collect();
    assert_eq!(exceptional.len(), 1);
    assert!(exceptional[0].payload().throw_site.is_some());
    assert_eq!(rtl.cfg().regions().len(), 1);
    assert!(matches!(
        rtl.cfg().regions()[0].handlers[0].body,
        cfglib::HandlerBody::Unknown
    ));

    let semantic = raise_function(&rtl).unwrap();
    assert!(semantic.verify().is_ok());
    assert!(
        semantic
            .cfg()
            .edges()
            .any(|edge| edge.payload().role == EdgeRole::Commit)
    );
    assert!(
        semantic.instructions().any(|instruction| {
            matches!(instruction.operation(), Operation::CaughtException(_))
        })
    );
}

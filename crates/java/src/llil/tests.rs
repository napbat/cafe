//! JVM LLIL tests.

use crate::bytecode::{self, Instruction as NativeInstruction, Opcode, Operand};
use crate::classfile::{Attribute, CodeAttribute, ExceptionHandler, RawAttribute};

use super::{Body, Instruction, Operation, lift_instructions, lower_instructions};

#[test]
fn every_effective_jvm_opcode_has_llil_semantics_and_reverses() {
    for &opcode in Opcode::ALL {
        if opcode == Opcode::Wide {
            continue;
        }
        let native = NativeInstruction::new(0, opcode, sample_operand(opcode));
        let llil = Instruction::from_native(&native)
            .unwrap_or_else(|error| panic!("{}: {error}", opcode.mnemonic()));
        assert_eq!(llil.to_native().unwrap(), native, "{}", opcode.mnemonic());
    }
}

#[test]
fn encoding_aliases_share_one_normalized_operation() {
    let shorthand =
        Instruction::from_native(&NativeInstruction::new(0, Opcode::ILoad0, Operand::None))
            .unwrap();
    let explicit =
        Instruction::from_native(&NativeInstruction::new(0, Opcode::ILoad, Operand::Local(0)))
            .unwrap();

    assert_eq!(shorthand.operation, explicit.operation);
    assert_ne!(shorthand.encoding, explicit.encoding);

    let dense = Instruction::from_native(&NativeInstruction::new(
        0,
        Opcode::TableSwitch,
        Operand::TableSwitch {
            default: 30,
            low: 3,
            targets: vec![10, 20],
        },
    ))
    .unwrap();
    let sparse = Instruction::from_native(&NativeInstruction::new(
        0,
        Opcode::LookupSwitch,
        Operand::LookupSwitch {
            default: 30,
            pairs: vec![(3, 10), (4, 20)],
        },
    ))
    .unwrap();
    assert_eq!(dense.operation, sparse.operation);
    assert_ne!(dense.encoding, sparse.encoding);

    let interface_one = Instruction::from_native(&NativeInstruction::new(
        0,
        Opcode::InvokeInterface,
        Operand::InvokeInterface { index: 1, count: 1 },
    ))
    .unwrap();
    let interface_two = Instruction::from_native(&NativeInstruction::new(
        0,
        Opcode::InvokeInterface,
        Operand::InvokeInterface { index: 1, count: 2 },
    ))
    .unwrap();
    assert_eq!(interface_one.operation, interface_two.operation);
    assert_ne!(interface_one.encoding, interface_two.encoding);
}

#[test]
fn decoded_stream_round_trips_byte_for_byte() {
    let bytes = vec![
        Opcode::IConst1.byte(),
        Opcode::IStore0.byte(),
        Opcode::IInc.byte(),
        0,
        1,
        Opcode::ILoad0.byte(),
        Opcode::IfEq.byte(),
        0,
        5,
        Opcode::IConst2.byte(),
        Opcode::Pop.byte(),
        Opcode::Return.byte(),
    ];
    let native = bytecode::decode(&bytes).unwrap();
    let llil = lift_instructions(&native).unwrap();
    let lowered = lower_instructions(&llil).unwrap();

    assert_eq!(lowered, native);
    assert_eq!(bytecode::encode(&lowered).unwrap(), bytes);
}

#[test]
fn wide_encoding_round_trips_exactly() {
    let native = NativeInstruction::new_wide(
        0,
        Opcode::IInc,
        Operand::Increment {
            index: 300,
            value: 1_000,
        },
    );
    let bytes = bytecode::encode(std::slice::from_ref(&native)).unwrap();
    let decoded = bytecode::decode(&bytes).unwrap();
    let llil = lift_instructions(&decoded).unwrap();

    assert_eq!(
        bytecode::encode(&lower_instructions(&llil).unwrap()).unwrap(),
        bytes
    );
    assert!(llil[0].encoding.wide);
}

#[test]
fn code_attribute_round_trip_preserves_handlers_and_nested_attributes() {
    let code = CodeAttribute {
        name_index: 7,
        max_stack: 1,
        max_locals: 1,
        code: vec![
            Opcode::AConstNull.byte(),
            Opcode::AThrow.byte(),
            Opcode::AStore0.byte(),
            Opcode::Return.byte(),
        ],
        exception_table: vec![ExceptionHandler {
            start_pc: 0,
            end_pc: 2,
            handler_pc: 2,
            catch_type: 0,
        }],
        attributes: vec![Attribute::Raw(RawAttribute {
            name_index: 8,
            name: "VendorDebug".to_owned(),
            info: vec![1, 2, 3],
        })],
    };

    let llil = Body::from_code(&code).unwrap();
    assert_eq!(llil.to_code().unwrap(), code);
}

#[test]
fn stale_native_encoding_is_rejected() {
    let mut llil =
        Instruction::from_native(&NativeInstruction::new(0, Opcode::IConst0, Operand::None))
            .unwrap();
    llil.operation = Operation::Nop;

    assert!(llil.to_native().is_err());
}

fn sample_operand(opcode: Opcode) -> Operand {
    use Opcode as O;

    match opcode {
        O::BiPush => Operand::Byte(1),
        O::SiPush => Operand::Short(1),
        O::Ldc
        | O::LdcW
        | O::Ldc2W
        | O::GetStatic
        | O::PutStatic
        | O::GetField
        | O::PutField
        | O::InvokeVirtual
        | O::InvokeSpecial
        | O::InvokeStatic
        | O::New
        | O::ANewArray
        | O::CheckCast
        | O::InstanceOf => Operand::Constant(1),
        O::ILoad
        | O::LLoad
        | O::FLoad
        | O::DLoad
        | O::ALoad
        | O::IStore
        | O::LStore
        | O::FStore
        | O::DStore
        | O::AStore
        | O::Ret => Operand::Local(1),
        O::IInc => Operand::Increment { index: 1, value: 1 },
        O::IfEq
        | O::IfNe
        | O::IfLt
        | O::IfGe
        | O::IfGt
        | O::IfLe
        | O::IfICmpEq
        | O::IfICmpNe
        | O::IfICmpLt
        | O::IfICmpGe
        | O::IfICmpGt
        | O::IfICmpLe
        | O::IfACmpEq
        | O::IfACmpNe
        | O::Goto
        | O::Jsr
        | O::IfNull
        | O::IfNonNull
        | O::GotoW
        | O::JsrW => Operand::Branch(0),
        O::TableSwitch => Operand::TableSwitch {
            default: 0,
            low: 1,
            targets: vec![0],
        },
        O::LookupSwitch => Operand::LookupSwitch {
            default: 0,
            pairs: vec![(1, 0)],
        },
        O::InvokeInterface => Operand::InvokeInterface { index: 1, count: 1 },
        O::InvokeDynamic => Operand::InvokeDynamic(1),
        O::NewArray => Operand::ArrayType(crate::bytecode::ArrayType::Int),
        O::MultiANewArray => Operand::MultiArray {
            index: 1,
            dimensions: 1,
        },
        O::Wide => unreachable!("wide is an encoding prefix, not an effective opcode"),
        _ => Operand::None,
    }
}

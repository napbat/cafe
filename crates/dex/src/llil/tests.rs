//! Dalvik LLIL tests.

use crate::file::{CatchHandler, CodeItem, DebugEvent, DebugInfo, TryBlock};
use crate::instruction::{
    self, ArrayDataPayload, Instruction as NativeInstruction, InstructionFormat, Opcode, Operands,
    PackedSwitchPayload, SparseSwitchPayload,
};

use super::{
    Body, Instruction, InstructionKind, OperationKind, lift_instructions, lower_instructions,
};

#[test]
fn every_standard_opcode_has_llil_semantics_and_reverses() {
    for &opcode in Opcode::ALL {
        let native = NativeInstruction::operation(0, opcode, sample_operands(opcode));
        let llil = Instruction::from_native(&native)
            .unwrap_or_else(|error| panic!("{}: {error}", opcode.mnemonic()));
        assert_eq!(llil.to_native().unwrap(), native, "{}", opcode.mnemonic());
    }
}

#[test]
fn encoding_aliases_share_normalized_operations() {
    let compact = Instruction::from_native(&NativeInstruction::operation(
        0,
        Opcode::Move,
        Operands::Registers {
            first: 1,
            second: 2,
        },
    ))
    .unwrap();
    let extended = Instruction::from_native(&NativeInstruction::operation(
        0,
        Opcode::MoveFrom16,
        Operands::Registers {
            first: 1,
            second: 2,
        },
    ))
    .unwrap();

    assert_eq!(compact.kind, extended.kind);
    assert_ne!(compact.encoding, extended.encoding);

    let list = Instruction::from_native(&NativeInstruction::operation(
        0,
        Opcode::InvokeStatic,
        Operands::RegisterListIndex {
            registers: vec![1, 2],
            index: 3,
            secondary_index: None,
        },
    ))
    .unwrap();
    let range = Instruction::from_native(&NativeInstruction::operation(
        0,
        Opcode::InvokeStaticRange,
        Operands::RegisterRangeIndex {
            start: 1,
            count: 2,
            index: 3,
            secondary_index: None,
        },
    ))
    .unwrap();

    assert_eq!(list.kind, range.kind);
    assert_ne!(list.encoding, range.encoding);
}

#[test]
fn operation_and_payload_streams_round_trip_code_unit_for_code_unit() {
    let streams = [
        packed_switch_stream(),
        sparse_switch_stream(),
        array_data_stream(),
    ];

    for native in streams {
        let encoded = instruction::encode(&native).unwrap();
        let llil = lift_instructions(&native).unwrap();
        let lowered = lower_instructions(&llil).unwrap();

        assert_eq!(lowered, native);
        assert_eq!(instruction::encode(&lowered).unwrap(), encoded);
    }
}

#[test]
fn code_item_round_trip_preserves_handlers_debug_info_and_frame_sizes() {
    let code = CodeItem {
        registers_size: 1,
        ins_size: 0,
        outs_size: 1,
        instructions: vec![
            NativeInstruction::operation(0, Opcode::Nop, Operands::None),
            NativeInstruction::operation(1, Opcode::MoveException, Operands::Register(0)),
            NativeInstruction::operation(2, Opcode::Throw, Operands::Register(0)),
        ],
        tries: vec![TryBlock {
            start_address: 0,
            instruction_count: 1,
            handlers: vec![CatchHandler {
                exception_type: None,
                address: 1,
            }],
        }],
        debug_info: Some(DebugInfo {
            line_start: 7,
            parameter_names: Vec::new(),
            events: vec![
                DebugEvent::Position {
                    address_delta: 1,
                    line_delta: 2,
                },
                DebugEvent::EndSequence,
            ],
            data_offset: 41,
        }),
        data_offset: 23,
    };

    let llil = Body::from_code(&code).unwrap();
    assert_eq!(llil.to_code().unwrap(), code);
}

#[test]
fn stale_native_encoding_is_rejected() {
    let mut llil = Instruction::from_native(&NativeInstruction::operation(
        0,
        Opcode::Const4,
        Operands::RegisterLiteral {
            register: 0,
            literal: 1,
        },
    ))
    .unwrap();
    let InstructionKind::Operation(operation) = &mut llil.kind else {
        panic!("constant must lift as an operation");
    };
    operation.kind = OperationKind::Nop;

    assert!(llil.to_native().is_err());
}

fn packed_switch_stream() -> Vec<NativeInstruction> {
    vec![
        NativeInstruction::operation(
            0,
            Opcode::PackedSwitch,
            Operands::RegisterBranch {
                register: 0,
                target: 4,
            },
        ),
        NativeInstruction::operation(3, Opcode::ReturnVoid, Operands::None),
        NativeInstruction::packed_switch(
            4,
            PackedSwitchPayload {
                first_key: 7,
                targets: vec![3],
            },
        ),
    ]
}

fn sparse_switch_stream() -> Vec<NativeInstruction> {
    vec![
        NativeInstruction::operation(
            0,
            Opcode::SparseSwitch,
            Operands::RegisterBranch {
                register: 0,
                target: 4,
            },
        ),
        NativeInstruction::operation(3, Opcode::ReturnVoid, Operands::None),
        NativeInstruction::sparse_switch(
            4,
            SparseSwitchPayload {
                keys: vec![-7, 19],
                targets: vec![3, 3],
            },
        ),
    ]
}

fn array_data_stream() -> Vec<NativeInstruction> {
    vec![
        NativeInstruction::operation(
            0,
            Opcode::FillArrayData,
            Operands::RegisterBranch {
                register: 0,
                target: 4,
            },
        ),
        NativeInstruction::operation(3, Opcode::ReturnVoid, Operands::None),
        NativeInstruction::array_data(
            4,
            ArrayDataPayload {
                element_width: 1,
                element_count: 3,
                data: vec![1, 2, 3],
            },
        ),
    ]
}

fn sample_operands(opcode: Opcode) -> Operands {
    match opcode.format() {
        InstructionFormat::F10x => Operands::None,
        InstructionFormat::F12x | InstructionFormat::F22x | InstructionFormat::F32x => {
            Operands::Registers {
                first: 1,
                second: 2,
            }
        }
        InstructionFormat::F11n
        | InstructionFormat::F21s
        | InstructionFormat::F21h
        | InstructionFormat::F31i
        | InstructionFormat::F51l => Operands::RegisterLiteral {
            register: 1,
            literal: 0,
        },
        InstructionFormat::F11x => Operands::Register(1),
        InstructionFormat::F10t | InstructionFormat::F20t | InstructionFormat::F30t => {
            Operands::Branch { target: 0 }
        }
        InstructionFormat::F21t | InstructionFormat::F31t => Operands::RegisterBranch {
            register: 1,
            target: 0,
        },
        InstructionFormat::F21c | InstructionFormat::F31c => Operands::RegisterIndex {
            register: 1,
            index: 0,
        },
        InstructionFormat::F23x => Operands::ThreeRegisters {
            first: 1,
            second: 2,
            third: 3,
        },
        InstructionFormat::F22t => Operands::RegistersBranch {
            first: 1,
            second: 2,
            target: 0,
        },
        InstructionFormat::F22s | InstructionFormat::F22b => Operands::RegistersLiteral {
            first: 1,
            second: 2,
            literal: 0,
        },
        InstructionFormat::F22c => Operands::RegistersIndex {
            first: 1,
            second: 2,
            index: 0,
        },
        InstructionFormat::F35c => Operands::RegisterListIndex {
            registers: vec![1, 2],
            index: 0,
            secondary_index: None,
        },
        InstructionFormat::F3rc => Operands::RegisterRangeIndex {
            start: 1,
            count: 2,
            index: 0,
            secondary_index: None,
        },
        InstructionFormat::F45cc => Operands::RegisterListIndex {
            registers: vec![1, 2],
            index: 0,
            secondary_index: Some(0),
        },
        InstructionFormat::F4rcc => Operands::RegisterRangeIndex {
            start: 1,
            count: 2,
            index: 0,
            secondary_index: Some(0),
        },
    }
}

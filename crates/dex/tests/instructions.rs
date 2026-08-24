//! Exhaustive standard-opcode and payload round-trip tests.

use std::collections::BTreeSet;

use dex::instruction::{
    ArrayDataPayload, Instruction, InstructionFormat, Opcode, Operands, PackedSwitchPayload,
    SparseSwitchPayload, decode, encode,
};

#[test]
fn every_standard_opcode_round_trips() {
    for &opcode in Opcode::ALL {
        let instructions = example(opcode);
        let encoded = encode(&instructions)
            .unwrap_or_else(|error| panic!("{} failed to encode: {error}", opcode.mnemonic()));
        let decoded = decode(&encoded)
            .unwrap_or_else(|error| panic!("{} failed to decode: {error}", opcode.mnemonic()));
        assert_eq!(decoded, instructions, "{} changed", opcode.mnemonic());
    }
}

#[test]
fn opcode_table_is_unique_and_ordered() {
    let mut bytes = BTreeSet::new();
    let mut previous = None;
    for &opcode in Opcode::ALL {
        assert_eq!(Opcode::from_byte(opcode.byte()), Some(opcode));
        assert!(bytes.insert(opcode.byte()), "duplicate opcode byte");
        if let Some(previous) = previous {
            assert!(previous < opcode.byte(), "opcode table is not ordered");
        }
        previous = Some(opcode.byte());
        assert!(!opcode.mnemonic().is_empty());
    }
    for byte in u8::MIN..=u8::MAX {
        assert_eq!(Opcode::from_byte(byte).is_some(), bytes.contains(&byte));
    }
}

#[test]
fn payloads_and_wide_values_use_exact_little_endian_layouts() {
    let packed = example(Opcode::PackedSwitch);
    assert_eq!(
        encode(&packed).unwrap(),
        vec![
            0x002b, 0x0004, 0x0000, 0x0000, 0x0100, 0x0001, 0xfffd, 0xffff, 0x0000, 0x0000
        ]
    );

    let wide = vec![Instruction::operation(
        0,
        Opcode::ConstWide,
        Operands::RegisterLiteral {
            register: 0xab,
            literal: i64::from_le_bytes([1, 2, 3, 4, 5, 6, 7, 128]),
        },
    )];
    assert_eq!(
        encode(&wide).unwrap(),
        vec![0xab18, 0x0201, 0x0403, 0x0605, 0x8007]
    );
    assert_eq!(decode(&encode(&wide).unwrap()).unwrap(), wide);
}

#[test]
fn every_truncated_prefix_is_rejected_without_panicking() {
    for opcode in [
        Opcode::ConstWide,
        Opcode::InvokePolymorphic,
        Opcode::PackedSwitch,
        Opcode::SparseSwitch,
        Opcode::FillArrayData,
    ] {
        let encoded = encode(&example(opcode)).unwrap();
        for end in 1..encoded.len() {
            assert!(
                decode(&encoded[..end]).is_err(),
                "{} accepted prefix {end}/{}",
                opcode.mnemonic(),
                encoded.len()
            );
        }
    }
}

#[test]
fn malformed_encodings_are_contextual_errors() {
    assert!(
        decode(&[0x003e])
            .unwrap_err()
            .to_string()
            .contains("undefined opcode")
    );
    assert!(
        decode(&[0x010e])
            .unwrap_err()
            .to_string()
            .contains("reserved byte")
    );
    assert!(
        decode(&[0x0029, 0x0001])
            .unwrap_err()
            .to_string()
            .contains("not an instruction boundary")
    );
    assert!(
        decode(&[0x0000, 0x0100, 0x0000, 0, 0])
            .unwrap_err()
            .to_string()
            .contains("not aligned")
    );
    assert!(
        decode(&[0x0026, 0x0004, 0, 0, 0x0300, 1, 1, 0, 0xff01])
            .unwrap_err()
            .to_string()
            .contains("padding")
    );
}

#[test]
fn unrepresentable_edits_are_rejected() {
    let bad_register = vec![Instruction::operation(
        0,
        Opcode::Move,
        Operands::Registers {
            first: 16,
            second: 0,
        },
    )];
    assert!(encode(&bad_register).is_err());

    let bad_layout = vec![Instruction::operation(
        1,
        Opcode::ReturnVoid,
        Operands::None,
    )];
    assert!(encode(&bad_layout).is_err());

    let bad_sparse = vec![
        Instruction::operation(
            0,
            Opcode::SparseSwitch,
            Operands::RegisterBranch {
                register: 0,
                target: 4,
            },
        ),
        Instruction::operation(3, Opcode::Nop, Operands::None),
        Instruction::sparse_switch(
            4,
            SparseSwitchPayload {
                keys: vec![2, 1],
                targets: vec![0, 0],
            },
        ),
    ];
    assert!(encode(&bad_sparse).is_err());
}

fn example(opcode: Opcode) -> Vec<Instruction> {
    match opcode {
        Opcode::PackedSwitch => vec![
            Instruction::operation(
                0,
                opcode,
                Operands::RegisterBranch {
                    register: 0,
                    target: 4,
                },
            ),
            Instruction::operation(3, Opcode::Nop, Operands::None),
            Instruction::packed_switch(
                4,
                PackedSwitchPayload {
                    first_key: -3,
                    targets: vec![0],
                },
            ),
        ],
        Opcode::SparseSwitch => vec![
            Instruction::operation(
                0,
                opcode,
                Operands::RegisterBranch {
                    register: 1,
                    target: 4,
                },
            ),
            Instruction::operation(3, Opcode::Nop, Operands::None),
            Instruction::sparse_switch(
                4,
                SparseSwitchPayload {
                    keys: vec![-10, 20],
                    targets: vec![0, 3],
                },
            ),
        ],
        Opcode::FillArrayData => vec![
            Instruction::operation(
                0,
                opcode,
                Operands::RegisterBranch {
                    register: 2,
                    target: 4,
                },
            ),
            Instruction::operation(3, Opcode::Nop, Operands::None),
            Instruction::array_data(
                4,
                ArrayDataPayload {
                    element_width: 1,
                    element_count: 3,
                    data: vec![1, 2, 3],
                },
            ),
        ],
        _ => vec![Instruction::operation(0, opcode, operands(opcode))],
    }
}

#[allow(clippy::too_many_lines)]
fn operands(opcode: Opcode) -> Operands {
    match opcode.format() {
        InstructionFormat::F10x => Operands::None,
        InstructionFormat::F12x => Operands::Registers {
            first: 1,
            second: 15,
        },
        InstructionFormat::F11n => Operands::RegisterLiteral {
            register: 15,
            literal: -3,
        },
        InstructionFormat::F11x => Operands::Register(250),
        InstructionFormat::F10t | InstructionFormat::F20t | InstructionFormat::F30t => {
            Operands::Branch { target: 0 }
        }
        InstructionFormat::F22x => Operands::Registers {
            first: 250,
            second: 60_000,
        },
        InstructionFormat::F21t => Operands::RegisterBranch {
            register: 250,
            target: 0,
        },
        InstructionFormat::F21s => Operands::RegisterLiteral {
            register: 250,
            literal: -12_345,
        },
        InstructionFormat::F21h => Operands::RegisterLiteral {
            register: 250,
            literal: if opcode == Opcode::ConstWideHigh16 {
                -2_i64 << 48
            } else {
                -2_i64 << 16
            },
        },
        InstructionFormat::F21c => Operands::RegisterIndex {
            register: 250,
            index: 60_000,
        },
        InstructionFormat::F23x => Operands::ThreeRegisters {
            first: 250,
            second: 249,
            third: 248,
        },
        InstructionFormat::F22t => Operands::RegistersBranch {
            first: 15,
            second: 14,
            target: 0,
        },
        InstructionFormat::F22s => Operands::RegistersLiteral {
            first: 15,
            second: 14,
            literal: -12_345,
        },
        InstructionFormat::F22c => Operands::RegistersIndex {
            first: 15,
            second: 14,
            index: 60_000,
        },
        InstructionFormat::F22b => Operands::RegistersLiteral {
            first: 250,
            second: 249,
            literal: -100,
        },
        InstructionFormat::F32x => Operands::Registers {
            first: 50_000,
            second: 60_000,
        },
        InstructionFormat::F31i => Operands::RegisterLiteral {
            register: 250,
            literal: -123_456_789,
        },
        InstructionFormat::F31t => unreachable!("payload opcodes handled separately"),
        InstructionFormat::F31c => Operands::RegisterIndex {
            register: 250,
            index: 0xfedc_ba98,
        },
        InstructionFormat::F35c => Operands::RegisterListIndex {
            registers: vec![1, 2, 3, 4, 15],
            index: 60_000,
            secondary_index: None,
        },
        InstructionFormat::F3rc => Operands::RegisterRangeIndex {
            start: 50_000,
            count: 12,
            index: 60_000,
            secondary_index: None,
        },
        InstructionFormat::F45cc => Operands::RegisterListIndex {
            registers: vec![1, 2, 3, 4, 15],
            index: 60_000,
            secondary_index: Some(60_001),
        },
        InstructionFormat::F4rcc => Operands::RegisterRangeIndex {
            start: 50_000,
            count: 12,
            index: 60_000,
            secondary_index: Some(60_001),
        },
        InstructionFormat::F51l => Operands::RegisterLiteral {
            register: 250,
            literal: i64::MIN + 123,
        },
    }
}

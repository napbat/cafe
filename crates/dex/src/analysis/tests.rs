use crate::file::{
    AccessFlags, CatchHandler, CodeItem, DexFile, DexString, DexVersion, EncodedMethod, MethodId,
    PrototypeId, TryBlock, TypeId,
};
use crate::instruction::{Instruction, InstructionFormat, Opcode, Operands, PackedSwitchPayload};

use super::{
    FlowEdgeKind, InstructionReference, PayloadKind, ProducedValue, ReferenceHierarchy,
    ReferenceType, RegisterType, ValueKind, analyze_body, analyze_method_registers,
    analyze_method_registers_with_hierarchy, control_flow, instruction_semantics,
    resolve_instruction_references,
};

#[test]
fn every_standard_opcode_has_semantics() {
    for &opcode in Opcode::ALL {
        let instruction = Instruction::operation(0, opcode, operands(opcode));
        let facts = instruction_semantics(&instruction)
            .unwrap_or_else(|error| panic!("{}: {error}", opcode.mnemonic()));
        assert!(facts.executable, "{}", opcode.mnemonic());
    }
}

#[test]
fn semantics_retain_typed_uses_defs_and_implicit_results() {
    let arithmetic = Instruction::operation(
        0,
        Opcode::AddLong,
        Operands::ThreeRegisters {
            first: 0,
            second: 2,
            third: 4,
        },
    );
    let facts = instruction_semantics(&arithmetic).unwrap();
    assert_eq!(facts.writes[0].kind, ValueKind::Long);
    assert_eq!(facts.reads[0].kind, ValueKind::Long);
    assert_eq!(facts.reads[1].register, 4);

    let invoke = Instruction::operation(
        0,
        Opcode::InvokeStatic,
        Operands::RegisterListIndex {
            registers: vec![1, 2],
            index: 0,
            secondary_index: None,
        },
    );
    let facts = instruction_semantics(&invoke).unwrap();
    assert_eq!(facts.produced, Some(ProducedValue::Prototype));
    assert!(facts.may_throw);
    assert_eq!(facts.reads.len(), 2);
}

#[test]
fn body_analysis_links_results_payloads_and_handlers() {
    let result_code = code(
        2,
        vec![
            Instruction::operation(
                0,
                Opcode::FilledNewArray,
                Operands::RegisterListIndex {
                    registers: vec![0, 1],
                    index: 0,
                    secondary_index: None,
                },
            ),
            Instruction::operation(3, Opcode::MoveResultObject, Operands::Register(0)),
            Instruction::operation(4, Opcode::ReturnVoid, Operands::None),
        ],
    );
    let result = analyze_body(&result_code).unwrap();
    assert_eq!(result.instruction(0).unwrap().result_consumer, Some(3));
    assert_eq!(result.instruction(3).unwrap().result_producer, Some(0));

    let switch_code = code(
        1,
        vec![
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
    );
    let switch = analyze_body(&switch_code).unwrap();
    let link = switch.instruction(0).unwrap().payload.unwrap();
    assert_eq!(link.kind, PayloadKind::PackedSwitch);
    assert_eq!(switch.payload_users(4), &[0]);

    let mut handler_code = code(
        1,
        vec![
            Instruction::operation(0, Opcode::Nop, Operands::None),
            Instruction::operation(1, Opcode::MoveException, Operands::Register(0)),
            Instruction::operation(2, Opcode::Throw, Operands::Register(0)),
        ],
    );
    handler_code.tries.push(TryBlock {
        start_address: 0,
        instruction_count: 1,
        handlers: vec![CatchHandler {
            exception_type: None,
            address: 1,
        }],
    });
    let handler = analyze_body(&handler_code).unwrap();
    assert_eq!(handler.instruction(1).unwrap().handler_types, vec![None]);
}

#[test]
fn body_analysis_rejects_dangling_results_registers_and_handlers() {
    let dangling = code(
        1,
        vec![
            Instruction::operation(0, Opcode::MoveResult, Operands::Register(0)),
            Instruction::operation(1, Opcode::ReturnVoid, Operands::None),
        ],
    );
    assert!(analyze_body(&dangling).is_err());

    let outside_frame = code(
        1,
        vec![
            Instruction::operation(
                0,
                Opcode::Move,
                Operands::Registers {
                    first: 0,
                    second: 1,
                },
            ),
            Instruction::operation(1, Opcode::ReturnVoid, Operands::None),
        ],
    );
    assert!(analyze_body(&outside_frame).is_err());

    let misplaced_exception = code(
        1,
        vec![
            Instruction::operation(0, Opcode::MoveException, Operands::Register(0)),
            Instruction::operation(1, Opcode::Throw, Operands::Register(0)),
        ],
    );
    assert!(analyze_body(&misplaced_exception).is_err());
}

#[test]
fn instruction_references_are_owned_and_preserve_exact_utf16() {
    let mut file = DexFile::new(DexVersion::V040);
    let method_name = file.push_string(DexString::new("run")).unwrap();
    let owner_text = file.push_string(DexString::new("LExample;")).unwrap();
    let int_text = file.push_string(DexString::new("I")).unwrap();
    let void_text = file.push_string(DexString::new("V")).unwrap();
    let odd_text = file
        .push_string(DexString::from_utf16(vec![0xd800]))
        .unwrap();
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
    let void = file
        .push_type(TypeId {
            descriptor: void_text,
        })
        .unwrap();
    let prototype = file
        .push_prototype(PrototypeId {
            shorty: void_text,
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

    let invoke = Instruction::operation(
        0,
        Opcode::InvokeStatic,
        Operands::RegisterListIndex {
            registers: vec![0],
            index: method.get(),
            secondary_index: None,
        },
    );
    let references = resolve_instruction_references(&file, &invoke).unwrap();
    let Some(InstructionReference::Method(symbol)) = references.primary else {
        panic!("expected a method symbol");
    };
    assert_eq!(symbol.owner, "LExample;");
    assert_eq!(
        symbol.name.utf16_units,
        "run".encode_utf16().collect::<Vec<_>>()
    );
    assert_eq!(symbol.descriptor, "(I)V");

    let string = Instruction::operation(
        0,
        Opcode::ConstString,
        Operands::RegisterIndex {
            register: 0,
            index: odd_text.get(),
        },
    );
    let references = resolve_instruction_references(&file, &string).unwrap();
    let Some(InstructionReference::String(symbol)) = references.primary else {
        panic!("expected a string symbol");
    };
    assert_eq!(symbol.utf16_units, vec![0xd800]);
}

#[test]
fn control_flow_distinguishes_normal_switch_and_exception_edges() {
    let mut exceptional = code(
        1,
        vec![
            Instruction::operation(
                0,
                Opcode::ConstString,
                Operands::RegisterIndex {
                    register: 0,
                    index: 0,
                },
            ),
            Instruction::operation(2, Opcode::ReturnVoid, Operands::None),
            Instruction::operation(3, Opcode::MoveException, Operands::Register(0)),
            Instruction::operation(4, Opcode::Throw, Operands::Register(0)),
        ],
    );
    exceptional.tries.push(TryBlock {
        start_address: 0,
        instruction_count: 2,
        handlers: vec![CatchHandler {
            exception_type: None,
            address: 3,
        }],
    });
    let flow = control_flow(&exceptional).unwrap();
    assert!(flow.edges().contains(&super::FlowEdge {
        source: 0,
        target: 2,
        kind: FlowEdgeKind::FallThrough,
    }));
    assert!(flow.edges().contains(&super::FlowEdge {
        source: 0,
        target: 3,
        kind: FlowEdgeKind::Exception(None),
    }));
    assert!(
        !flow
            .successors(2)
            .any(|edge| matches!(edge.kind, FlowEdgeKind::Exception(_)))
    );

    let switch_code = code(
        1,
        vec![
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
    );
    let flow = control_flow(&switch_code).unwrap();
    assert!(flow.edges().contains(&super::FlowEdge {
        source: 0,
        target: 3,
        kind: FlowEdgeKind::SwitchCase(7),
    }));
}

#[test]
fn register_analysis_types_parameters_moves_and_arithmetic() {
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

    let analysis = analyze_method_registers(&file, &declaration)
        .unwrap()
        .unwrap();
    assert_eq!(
        analysis.entry_frame(0).unwrap().register(1),
        Some(&super::RegisterType::Integer)
    );
    assert_eq!(
        analysis.exit_frame(0).unwrap().register(0),
        Some(&super::RegisterType::Integer)
    );
}

#[test]
fn register_analysis_uses_pre_instruction_frames_on_exception_edges() {
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

    let analysis = analyze_method_registers(&file, &declaration)
        .unwrap()
        .unwrap();
    assert_eq!(
        analysis.entry_frame(3).unwrap().register(0),
        Some(&super::RegisterType::Unknown)
    );
    assert!(matches!(
        analysis.exit_frame(3).unwrap().register(0),
        Some(super::RegisterType::Reference(_))
    ));
}

struct FixtureHierarchy;

impl ReferenceHierarchy for FixtureHierarchy {
    fn is_assignable(&self, source: &str, target: &str) -> bool {
        source == target
            || target == "Ljava/lang/Object;"
            || (source == "Lsample/Sub;" && target == "Lsample/Base;")
    }

    fn common_supertype(&self, left: &str, right: &str) -> Option<String> {
        if self.is_assignable(left, right) {
            Some(right.to_owned())
        } else if self.is_assignable(right, left) {
            Some(left.to_owned())
        } else {
            Some("Ljava/lang/Object;".to_owned())
        }
    }
}

#[test]
fn register_analysis_refines_array_components_and_uses_caller_hierarchy() {
    let mut file = DexFile::new(DexVersion::V040);
    let method_name = file.push_string(DexString::new("first")).unwrap();
    let owner_text = file.push_string(DexString::new("Lsample/Owner;")).unwrap();
    let base_text = file.push_string(DexString::new("Lsample/Base;")).unwrap();
    let array_text = file.push_string(DexString::new("[Lsample/Sub;")).unwrap();
    let shorty = file.push_string(DexString::new("LL")).unwrap();
    let owner = file
        .push_type(TypeId {
            descriptor: owner_text,
        })
        .unwrap();
    let base = file
        .push_type(TypeId {
            descriptor: base_text,
        })
        .unwrap();
    let array = file
        .push_type(TypeId {
            descriptor: array_text,
        })
        .unwrap();
    let prototype = file
        .push_prototype(PrototypeId {
            shorty,
            return_type: base,
            parameters: vec![array],
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
            registers_size: 3,
            ins_size: 1,
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
                Instruction::operation(
                    1,
                    Opcode::AgetObject,
                    Operands::ThreeRegisters {
                        first: 1,
                        second: 2,
                        third: 0,
                    },
                ),
                Instruction::operation(3, Opcode::ReturnObject, Operands::Register(1)),
            ],
            tries: Vec::new(),
            debug_info: None,
            data_offset: 0,
        }),
    };

    assert!(analyze_method_registers(&file, &declaration).is_err());
    let analysis = analyze_method_registers_with_hierarchy(&file, &declaration, &FixtureHierarchy)
        .unwrap()
        .unwrap();
    assert_eq!(
        analysis.exit_frame(1).unwrap().register(1),
        Some(&RegisterType::Reference(ReferenceType::Descriptor(
            "Lsample/Sub;".to_owned()
        )))
    );
}

fn code(registers_size: u16, instructions: Vec<Instruction>) -> CodeItem {
    CodeItem {
        registers_size,
        ins_size: 0,
        outs_size: 0,
        instructions,
        tries: Vec::new(),
        debug_info: None,
        data_offset: 0,
    }
}

#[allow(clippy::too_many_lines)]
fn operands(opcode: Opcode) -> Operands {
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

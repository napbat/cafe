//! Integration coverage for the public disassembly and graph API.

use disassembler::cfglib::{DominatorTree, verify};
use disassembler::{
    AddressUnit, CodeAddress, CodeSize, FunctionBody, Instruction, InstructionFlow,
};

const INSTRUCTION_SIZE: CodeSize = CodeSize::new(1);

fn instruction(address: u32, flow: InstructionFlow) -> Instruction {
    Instruction::new(
        CodeAddress::from(address),
        INSTRUCTION_SIZE,
        0,
        "fixture",
        Vec::new(),
        flow,
    )
}

#[test]
fn downstream_consumers_can_use_cfglib_algorithms_on_shared_ir() {
    let body = FunctionBody::new(
        AddressUnit::Byte,
        vec![
            instruction(
                0,
                InstructionFlow::ConditionalBranch {
                    target: CodeAddress::from(2_u32),
                },
            ),
            instruction(1, InstructionFlow::Return),
            instruction(2, InstructionFlow::Return),
        ],
        Vec::new(),
    );

    let graph = body.control_flow_graph().unwrap();
    assert!(verify(graph.cfg()).is_ok());

    let target = graph
        .block_for_instruction(CodeAddress::from(2_u32))
        .unwrap();
    let dominators = DominatorTree::compute(graph.cfg());
    assert!(dominators.dominates(graph.cfg().entry(), target));
    assert!(graph.to_dot().contains("green4"));
}

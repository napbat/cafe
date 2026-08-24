//! Basic-block discovery and cfglib edge construction.

use std::collections::{BTreeMap, BTreeSet};

use cfglib::{BlockId, Cfg, EdgeKind, verify};

use super::{ControlFlowGraph, GraphError};
use crate::{CodeAddress, FunctionBody, InstructionFlow};

/// Builds a verified cfglib control-flow graph from a function body.
///
/// Leaders are introduced at the entry, direct branch targets, instructions
/// following terminators, and exception-range boundaries. Exception handlers
/// produce `ExceptionUnwind` edges from every protected block.
///
/// # Errors
///
/// Returns an error if instruction ranges overlap, a target is not an
/// instruction boundary, exception metadata is invalid, or cfglib reports a
/// structural invariant violation.
pub fn build_control_flow_graph(body: &FunctionBody) -> Result<ControlFlowGraph, GraphError> {
    let code_end = validate_instructions(body)?;
    let instruction_addresses = body
        .instructions
        .iter()
        .map(|instruction| instruction.address)
        .collect::<BTreeSet<_>>();
    let mut leaders = collect_flow_leaders(body, &instruction_addresses)?;
    validate_exception_handlers(body, code_end, &instruction_addresses, &mut leaders)?;

    let (mut cfg, instruction_blocks) = populate_blocks(body, &leaders);
    add_normal_edges(&mut cfg, &instruction_blocks);
    add_exception_edges(body, &mut cfg, &instruction_blocks);

    let verification = verify(&cfg);
    if !verification.is_ok() {
        let details = verification
            .errors
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("; ");
        return Err(GraphError::InvalidGraph { details });
    }

    Ok(ControlFlowGraph::new(cfg, instruction_blocks))
}

fn validate_instructions(body: &FunctionBody) -> Result<CodeAddress, GraphError> {
    let mut previous_end = None;
    for instruction in &body.instructions {
        if instruction.size.is_zero() {
            return Err(GraphError::ZeroInstructionSize {
                address: instruction.address,
            });
        }
        if previous_end.is_some_and(|end| instruction.address < end) {
            return Err(GraphError::OverlappingInstruction {
                address: instruction.address,
                previous_end: previous_end.expect("checked as present"),
            });
        }
        previous_end = Some(
            instruction
                .checked_end()
                .ok_or(GraphError::AddressOverflow {
                    address: instruction.address,
                })?,
        );
    }
    Ok(previous_end.unwrap_or(CodeAddress::ZERO))
}

fn collect_flow_leaders(
    body: &FunctionBody,
    instruction_addresses: &BTreeSet<CodeAddress>,
) -> Result<BTreeSet<CodeAddress>, GraphError> {
    let mut leaders = BTreeSet::new();
    if let Some(first) = body.instructions.first() {
        leaders.insert(first.address);
    }

    for (position, instruction) in body.instructions.iter().enumerate() {
        match &instruction.flow {
            InstructionFlow::FallThrough
            | InstructionFlow::Return
            | InstructionFlow::Throw
            | InstructionFlow::IndirectBranch => {}
            InstructionFlow::ConditionalBranch { target }
            | InstructionFlow::UnconditionalBranch { target }
            | InstructionFlow::SubroutineCall { target } => {
                validate_branch_target(instruction.address, *target, instruction_addresses)?;
                leaders.insert(*target);
            }
            InstructionFlow::Switch { default, cases } => {
                validate_branch_target(instruction.address, *default, instruction_addresses)?;
                leaders.insert(*default);
                for case in cases {
                    validate_branch_target(
                        instruction.address,
                        case.target,
                        instruction_addresses,
                    )?;
                    leaders.insert(case.target);
                }
            }
        }

        if instruction.flow.ends_basic_block()
            && let Some(next) = body.instructions.get(position + 1)
        {
            leaders.insert(next.address);
        }
    }
    Ok(leaders)
}

fn validate_branch_target(
    source: CodeAddress,
    target: CodeAddress,
    instruction_addresses: &BTreeSet<CodeAddress>,
) -> Result<(), GraphError> {
    if instruction_addresses.contains(&target) {
        Ok(())
    } else {
        Err(GraphError::MissingBranchTarget {
            source_address: source,
            target,
        })
    }
}

fn validate_exception_handlers(
    body: &FunctionBody,
    code_end: CodeAddress,
    instruction_addresses: &BTreeSet<CodeAddress>,
    leaders: &mut BTreeSet<CodeAddress>,
) -> Result<(), GraphError> {
    for handler in &body.exception_handlers {
        if handler.protected.is_empty() {
            return Err(GraphError::InvalidExceptionRange {
                start: handler.protected.start,
                end: handler.protected.end,
            });
        }
        if !instruction_addresses.contains(&handler.protected.start) {
            return Err(GraphError::MissingExceptionStart {
                address: handler.protected.start,
            });
        }
        if handler.protected.end != code_end
            && !instruction_addresses.contains(&handler.protected.end)
        {
            return Err(GraphError::MissingExceptionEnd {
                address: handler.protected.end,
            });
        }
        if !instruction_addresses.contains(&handler.handler) {
            return Err(GraphError::MissingExceptionHandler {
                address: handler.handler,
            });
        }
        leaders.insert(handler.protected.start);
        if handler.protected.end != code_end {
            leaders.insert(handler.protected.end);
        }
        leaders.insert(handler.handler);
    }
    Ok(())
}

fn populate_blocks(
    body: &FunctionBody,
    leaders: &BTreeSet<CodeAddress>,
) -> (Cfg<crate::Instruction>, BTreeMap<CodeAddress, BlockId>) {
    let mut cfg = Cfg::new();
    let mut current = cfg.entry();
    let mut instruction_blocks = BTreeMap::new();

    for (position, instruction) in body.instructions.iter().enumerate() {
        if position != 0 && leaders.contains(&instruction.address) {
            current = cfg.new_block();
        }
        if cfg.block(current).is_empty() {
            cfg.block_mut(current)
                .set_label(format!("address_{}", instruction.address));
        }
        cfg.block_mut(current).push(instruction.clone());
        instruction_blocks.insert(instruction.address, current);
    }
    (cfg, instruction_blocks)
}

fn add_normal_edges(
    cfg: &mut Cfg<crate::Instruction>,
    instruction_blocks: &BTreeMap<CodeAddress, BlockId>,
) {
    let blocks = cfg
        .blocks()
        .iter()
        .filter(|block| !block.is_empty())
        .map(cfglib::BasicBlock::id)
        .collect::<Vec<_>>();

    for (position, &block) in blocks.iter().enumerate() {
        let flow = cfg
            .block(block)
            .instructions()
            .last()
            .expect("filtered to non-empty blocks")
            .flow
            .clone();
        let next = blocks.get(position + 1).copied();
        match flow {
            InstructionFlow::FallThrough => {
                add_optional_edge(cfg, block, next, EdgeKind::Fallthrough);
            }
            InstructionFlow::ConditionalBranch { target } => {
                add_target_edge(
                    cfg,
                    block,
                    target,
                    EdgeKind::ConditionalTrue,
                    instruction_blocks,
                );
                add_optional_edge(cfg, block, next, EdgeKind::ConditionalFalse);
            }
            InstructionFlow::UnconditionalBranch { target } => {
                add_target_edge(cfg, block, target, EdgeKind::Jump, instruction_blocks);
            }
            InstructionFlow::Switch { default, cases } => {
                add_target_edge(
                    cfg,
                    block,
                    default,
                    EdgeKind::SwitchCase,
                    instruction_blocks,
                );
                for case in cases {
                    add_target_edge(
                        cfg,
                        block,
                        case.target,
                        EdgeKind::SwitchCase,
                        instruction_blocks,
                    );
                }
            }
            InstructionFlow::SubroutineCall { target } => {
                add_target_edge(cfg, block, target, EdgeKind::Call, instruction_blocks);
                add_optional_edge(cfg, block, next, EdgeKind::CallReturn);
            }
            InstructionFlow::Return | InstructionFlow::Throw | InstructionFlow::IndirectBranch => {}
        }
    }
}

fn add_optional_edge(
    cfg: &mut Cfg<crate::Instruction>,
    source: BlockId,
    target: Option<BlockId>,
    kind: EdgeKind,
) {
    if let Some(target) = target {
        cfg.add_edge(source, target, kind);
    }
}

fn add_target_edge(
    cfg: &mut Cfg<crate::Instruction>,
    source: BlockId,
    target: CodeAddress,
    kind: EdgeKind,
    instruction_blocks: &BTreeMap<CodeAddress, BlockId>,
) {
    cfg.add_edge(source, instruction_blocks[&target], kind);
}

fn add_exception_edges(
    body: &FunctionBody,
    cfg: &mut Cfg<crate::Instruction>,
    instruction_blocks: &BTreeMap<CodeAddress, BlockId>,
) {
    let block_starts = cfg
        .blocks()
        .iter()
        .filter_map(|block| {
            block
                .instructions()
                .first()
                .map(|instruction| (block.id(), instruction.address))
        })
        .collect::<Vec<_>>();

    for handler in &body.exception_handlers {
        let target = instruction_blocks[&handler.handler];
        for &(source, address) in &block_starts {
            if handler.protected.contains(address) {
                cfg.add_edge(source, target, EdgeKind::ExceptionUnwind);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use cfglib::EdgeKind;

    use super::build_control_flow_graph;
    use crate::{
        AddressRange, AddressUnit, CatchType, CodeAddress, CodeSize, ExceptionHandler,
        FunctionBody, Instruction, InstructionFlow,
    };

    const ONE_UNIT: CodeSize = CodeSize::new(1);

    fn instruction(address: u32, flow: InstructionFlow) -> Instruction {
        Instruction::new(
            CodeAddress::from(address),
            ONE_UNIT,
            0,
            "test",
            Vec::new(),
            flow,
        )
    }

    #[test]
    fn builds_a_diamond_with_typed_edges_and_dot_output() {
        let body = FunctionBody::new(
            AddressUnit::Byte,
            vec![
                instruction(
                    0,
                    InstructionFlow::ConditionalBranch {
                        target: CodeAddress::from(3_u32),
                    },
                ),
                instruction(1, InstructionFlow::FallThrough),
                instruction(
                    2,
                    InstructionFlow::UnconditionalBranch {
                        target: CodeAddress::from(4_u32),
                    },
                ),
                instruction(3, InstructionFlow::FallThrough),
                instruction(4, InstructionFlow::Return),
            ],
            Vec::new(),
        );

        let graph = build_control_flow_graph(&body).unwrap();
        assert_eq!(graph.cfg().num_blocks(), 4);
        assert_eq!(graph.cfg().num_edges(), 4);
        assert!(
            graph
                .cfg()
                .edges()
                .any(|edge| edge.kind() == EdgeKind::ConditionalTrue)
        );
        assert!(graph.to_dot().contains("digraph cfg"));
    }

    #[test]
    fn adds_exception_unwind_edges() {
        let body = FunctionBody::new(
            AddressUnit::Byte,
            vec![
                instruction(0, InstructionFlow::FallThrough),
                instruction(1, InstructionFlow::Return),
                instruction(2, InstructionFlow::Return),
            ],
            vec![ExceptionHandler {
                protected: AddressRange::new(CodeAddress::from(0_u32), CodeAddress::from(2_u32)),
                handler: CodeAddress::from(2_u32),
                catch: CatchType::Any,
            }],
        );

        let graph = build_control_flow_graph(&body).unwrap();
        assert!(
            graph
                .cfg()
                .edges()
                .any(|edge| edge.kind() == EdgeKind::ExceptionUnwind)
        );
    }

    #[test]
    fn rejects_targets_that_are_not_instruction_boundaries() {
        let body = FunctionBody::new(
            AddressUnit::CodeUnit16,
            vec![instruction(
                0,
                InstructionFlow::UnconditionalBranch {
                    target: CodeAddress::from(9_u32),
                },
            )],
            Vec::new(),
        );

        assert!(build_control_flow_graph(&body).is_err());
    }
}

//! Basic-block discovery and cfglib edge construction.

use std::collections::{BTreeMap, BTreeSet};

use cfglib::{BlockId, Cfg, EdgeKind, verify_with};

use super::validate::ControlFlowValidator;
use super::{
    ControlFlowEdge, ControlFlowEdgeRole, ControlFlowGraph, ExceptionHandlerIndex, GraphError,
};
use crate::{CodeAddress, FunctionBody, Instruction, InstructionFlow};

type SharedCfg = Cfg<Instruction, ControlFlowEdge>;

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

    let verification = verify_with(&cfg, &ControlFlowValidator::new(body));
    if !verification.is_ok() {
        let details = verification
            .structural
            .errors
            .iter()
            .map(ToString::to_string)
            .chain(verification.semantic_errors.iter().map(ToString::to_string))
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
        for instruction in &body.instructions {
            if handler.protected.contains(instruction.address) {
                leaders.insert(instruction.address);
            }
        }
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
) -> (SharedCfg, BTreeMap<CodeAddress, BlockId>) {
    let mut cfg = SharedCfg::new_with_edge_payload();
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

fn add_normal_edges(cfg: &mut SharedCfg, instruction_blocks: &BTreeMap<CodeAddress, BlockId>) {
    let blocks = cfg
        .blocks()
        .iter()
        .filter(|block| !block.is_empty())
        .map(cfglib::BasicBlock::id)
        .collect::<Vec<_>>();

    for (position, &block) in blocks.iter().enumerate() {
        let terminator = cfg
            .block(block)
            .instructions()
            .last()
            .expect("filtered to non-empty blocks");
        let source_address = terminator.address;
        let flow = terminator.flow.clone();
        let next = blocks.get(position + 1).copied();
        match flow {
            InstructionFlow::FallThrough => {
                add_optional_edge(
                    cfg,
                    block,
                    next,
                    EdgeKind::Fallthrough,
                    ControlFlowEdgeRole::Sequential,
                );
            }
            InstructionFlow::ConditionalBranch { target } => {
                add_target_edge(
                    cfg,
                    block,
                    target,
                    EdgeKind::ConditionalTrue,
                    ControlFlowEdgeRole::ConditionalTaken,
                    instruction_blocks,
                );
                add_optional_edge(
                    cfg,
                    block,
                    next,
                    EdgeKind::ConditionalFalse,
                    ControlFlowEdgeRole::ConditionalFallThrough,
                );
            }
            InstructionFlow::UnconditionalBranch { target } => {
                add_target_edge(
                    cfg,
                    block,
                    target,
                    EdgeKind::Jump,
                    ControlFlowEdgeRole::DirectBranch,
                    instruction_blocks,
                );
            }
            InstructionFlow::Switch { default, cases } => {
                add_target_edge(
                    cfg,
                    block,
                    default,
                    EdgeKind::SwitchCase,
                    ControlFlowEdgeRole::SwitchDefault,
                    instruction_blocks,
                );
                for case in cases {
                    add_target_edge(
                        cfg,
                        block,
                        case.target,
                        EdgeKind::SwitchCase,
                        ControlFlowEdgeRole::SwitchCase { key: case.key },
                        instruction_blocks,
                    );
                }
            }
            InstructionFlow::SubroutineCall { target } => {
                add_target_edge(
                    cfg,
                    block,
                    target,
                    EdgeKind::Call,
                    ControlFlowEdgeRole::SubroutineCall,
                    instruction_blocks,
                );
                add_optional_edge(
                    cfg,
                    block,
                    next,
                    EdgeKind::CallReturn,
                    ControlFlowEdgeRole::SubroutineContinuation {
                        call_site: source_address,
                    },
                );
            }
            InstructionFlow::Return | InstructionFlow::Throw | InstructionFlow::IndirectBranch => {}
        }
    }
}

fn add_optional_edge(
    cfg: &mut SharedCfg,
    source: BlockId,
    target: Option<BlockId>,
    kind: EdgeKind,
    role: ControlFlowEdgeRole,
) {
    if let Some(target) = target {
        add_flow_edge(cfg, source, target, kind, role);
    }
}

fn add_target_edge(
    cfg: &mut SharedCfg,
    source: BlockId,
    target: CodeAddress,
    kind: EdgeKind,
    role: ControlFlowEdgeRole,
    instruction_blocks: &BTreeMap<CodeAddress, BlockId>,
) {
    add_flow_edge(cfg, source, instruction_blocks[&target], kind, role);
}

fn add_flow_edge(
    cfg: &mut SharedCfg,
    source: BlockId,
    target: BlockId,
    kind: EdgeKind,
    role: ControlFlowEdgeRole,
) {
    let source_address = cfg
        .block(source)
        .instructions()
        .last()
        .expect("normal edges leave non-empty blocks")
        .address;
    let target_address = cfg
        .block(target)
        .instructions()
        .first()
        .expect("normal edges enter non-empty blocks")
        .address;
    cfg.add_edge_with_payload(
        source,
        target,
        kind,
        ControlFlowEdge::new(source_address, target_address, role),
    );
}

fn add_exception_edges(
    body: &FunctionBody,
    cfg: &mut SharedCfg,
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

    for (index, handler) in body.exception_handlers.iter().enumerate() {
        let target = instruction_blocks[&handler.handler];
        for &(source, address) in &block_starts {
            if handler.protected.contains(address) {
                cfg.add_edge_with_payload(
                    source,
                    target,
                    EdgeKind::ExceptionUnwind,
                    ControlFlowEdge::new(
                        address,
                        handler.handler,
                        ControlFlowEdgeRole::Exception {
                            handler: ExceptionHandlerIndex::from_index(index),
                            protected: handler.protected,
                            catch: handler.catch.clone(),
                        },
                    ),
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use cfglib::{
        BlockId, Cfg, Direction, DominatorTree, Edge, EdgeKind, EdgeProblem, EdgeRef,
        solve_edge_problem, verify_edge_view,
    };

    use super::build_control_flow_graph;
    use crate::{
        AddressRange, AddressUnit, CatchType, CodeAddress, CodeSize, ControlFlowEdge,
        ControlFlowEdgeRole, ExceptionHandler, ExceptionHandlerIndex, FunctionBody, Instruction,
        InstructionFlow, SwitchCase,
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
        let exceptional = graph
            .cfg()
            .edges()
            .filter(|edge| edge.kind() == EdgeKind::ExceptionUnwind)
            .collect::<Vec<_>>();
        assert_eq!(exceptional.len(), 2);
        assert_eq!(exceptional[0].payload().source(), CodeAddress::from(0_u32));
        assert_eq!(exceptional[1].payload().source(), CodeAddress::from(1_u32));
        for edge in &exceptional {
            assert_eq!(graph.cfg().block(edge.source()).instructions().len(), 1);
            assert!(matches!(
                edge.payload().role(),
                ControlFlowEdgeRole::Exception { handler, catch, .. }
                    if *handler == ExceptionHandlerIndex::from_index(0)
                        && catch == &CatchType::Any
            ));
        }

        let handler = graph
            .block_for_instruction(CodeAddress::from(2_u32))
            .unwrap();
        assert!(DominatorTree::compute(graph.cfg()).is_reachable(handler));
        assert!(verify_edge_view(&graph.normal_view()).is_ok());
        assert!(!DominatorTree::compute(&graph.normal_view()).is_reachable(handler));
    }

    #[test]
    fn preserves_parallel_switch_arms_and_subroutine_call_sites() {
        let switch_body = FunctionBody::new(
            AddressUnit::Byte,
            vec![
                instruction(
                    0,
                    InstructionFlow::Switch {
                        default: CodeAddress::from(2_u32),
                        cases: vec![SwitchCase {
                            key: 7,
                            target: CodeAddress::from(2_u32),
                        }],
                    },
                ),
                instruction(1, InstructionFlow::Return),
                instruction(2, InstructionFlow::Return),
            ],
            Vec::new(),
        );
        let switch_graph = build_control_flow_graph(&switch_body).unwrap();
        let entry_edges = switch_graph
            .cfg()
            .successor_edges(switch_graph.cfg().entry());
        assert_eq!(entry_edges.len(), 2);
        assert_ne!(entry_edges[0], entry_edges[1]);
        assert_eq!(
            switch_graph.cfg().edge(entry_edges[0]).target(),
            switch_graph.cfg().edge(entry_edges[1]).target()
        );
        assert_eq!(
            switch_graph.cfg().edge(entry_edges[0]).payload().role(),
            &ControlFlowEdgeRole::SwitchDefault
        );
        assert_eq!(
            switch_graph.cfg().edge(entry_edges[1]).payload().role(),
            &ControlFlowEdgeRole::SwitchCase { key: 7 }
        );

        let subroutine_body = FunctionBody::new(
            AddressUnit::Byte,
            vec![
                instruction(
                    0,
                    InstructionFlow::SubroutineCall {
                        target: CodeAddress::from(2_u32),
                    },
                ),
                instruction(1, InstructionFlow::Return),
                instruction(2, InstructionFlow::Return),
            ],
            Vec::new(),
        );
        let subroutine_graph = build_control_flow_graph(&subroutine_body).unwrap();
        let continuation = subroutine_graph
            .cfg()
            .edges()
            .find(|edge| {
                matches!(
                    edge.payload().role(),
                    ControlFlowEdgeRole::SubroutineContinuation { .. }
                )
            })
            .unwrap();
        assert_eq!(
            continuation.payload().role(),
            &ControlFlowEdgeRole::SubroutineContinuation {
                call_site: CodeAddress::from(0_u32),
            }
        );
    }

    struct PrePostState {
        entry: BlockId,
    }

    impl EdgeProblem<Cfg<Instruction, ControlFlowEdge>> for PrePostState {
        type Fact = u8;

        fn direction(&self) -> Direction {
            Direction::Forward
        }

        fn bottom(&self, _graph: &Cfg<Instruction, ControlFlowEdge>) -> Self::Fact {
            0
        }

        fn boundary(
            &self,
            _graph: &Cfg<Instruction, ControlFlowEdge>,
            node: BlockId,
        ) -> Option<Self::Fact> {
            (node == self.entry).then_some(1)
        }

        fn meet(&self, left: &Self::Fact, right: &Self::Fact) -> Self::Fact {
            left | right
        }

        fn transfer_node(
            &self,
            _graph: &Cfg<Instruction, ControlFlowEdge>,
            node: BlockId,
            input: &Self::Fact,
        ) -> Self::Fact {
            if node == self.entry {
                input | 2
            } else {
                *input
            }
        }

        fn transfer_edge(
            &self,
            _graph: &Cfg<Instruction, ControlFlowEdge>,
            edge: EdgeRef<'_, BlockId, cfglib::EdgeId, Edge<ControlFlowEdge>>,
            node_input: &Self::Fact,
            node_output: &Self::Fact,
        ) -> Self::Fact {
            if edge.data().payload().is_exceptional() {
                *node_input
            } else {
                *node_output
            }
        }
    }

    #[test]
    fn edge_dataflow_uses_pre_state_for_exception_and_post_state_for_normal_flow() {
        let body = FunctionBody::new(
            AddressUnit::Byte,
            vec![
                instruction(0, InstructionFlow::FallThrough),
                instruction(1, InstructionFlow::Return),
                instruction(2, InstructionFlow::Return),
            ],
            vec![ExceptionHandler {
                protected: AddressRange::new(CodeAddress::from(0_u32), CodeAddress::from(1_u32)),
                handler: CodeAddress::from(2_u32),
                catch: CatchType::Any,
            }],
        );
        let graph = build_control_flow_graph(&body).unwrap();
        let entry = graph.cfg().entry();
        let facts = solve_edge_problem(graph.cfg(), &PrePostState { entry }).unwrap();
        let mut normal_fact = None;
        let mut exception_fact = None;
        for &edge in graph.cfg().successor_edges(entry) {
            if graph.cfg().edge(edge).payload().is_exceptional() {
                exception_fact = facts.fact_on(edge).copied();
            } else {
                normal_fact = facts.fact_on(edge).copied();
            }
        }
        assert_eq!(facts.fact_in(entry), &1);
        assert_eq!(facts.fact_out(entry), &3);
        assert_eq!(normal_fact, Some(3));
        assert_eq!(exception_fact, Some(1));
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

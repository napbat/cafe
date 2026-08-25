//! Basic-block discovery and cfglib edge construction.

use std::collections::{BTreeMap, BTreeSet};

use cfglib::{
    BlockId, Cfg, EdgeKind, Handler, HandlerBody, HandlerKind, HandlerRef, Region, RegionId,
    verify_with,
};

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
/// produce `ExceptionUnwind` edges from every protected block and ordered
/// region metadata with explicitly unknown handler-body extents.
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
    let handler_refs = add_exception_regions(body, &mut cfg, &instruction_blocks);

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

    Ok(ControlFlowGraph::new(
        cfg,
        instruction_blocks,
        body.exception_handlers.clone(),
        handler_refs,
    ))
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
    let mut cfg = SharedCfg::with_edge_payload();
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

#[derive(Debug)]
struct ExceptionRegionDraft {
    protected: crate::AddressRange,
    handler_indices: Vec<usize>,
}

fn add_exception_regions(
    body: &FunctionBody,
    cfg: &mut SharedCfg,
    instruction_blocks: &BTreeMap<CodeAddress, BlockId>,
) -> Vec<HandlerRef> {
    let mut drafts = Vec::<ExceptionRegionDraft>::new();
    for (index, handler) in body.exception_handlers.iter().enumerate() {
        if let Some(draft) = drafts
            .iter_mut()
            .find(|draft| draft.protected == handler.protected)
        {
            draft.handler_indices.push(index);
        } else {
            drafts.push(ExceptionRegionDraft {
                protected: handler.protected,
                handler_indices: vec![index],
            });
        }
    }

    // cfglib resolves the innermost protecting region by reverse insertion
    // order, so enclosing ranges must be registered before nested ranges.
    // Stable sorting preserves native table order for disjoint peers.
    drafts.sort_by(|left, right| {
        exception_range_span(right.protected).cmp(&exception_range_span(left.protected))
    });

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

    let mut handler_refs = vec![None; body.exception_handlers.len()];
    for (index, draft) in drafts.iter().enumerate() {
        let protected_blocks = block_starts
            .iter()
            .filter_map(|&(block, address)| draft.protected.contains(address).then_some(block))
            .collect();
        let handlers = draft
            .handler_indices
            .iter()
            .map(|&handler_index| {
                let handler = &body.exception_handlers[handler_index];
                Handler {
                    entry: instruction_blocks[&handler.handler],
                    body: HandlerBody::unknown(),
                    kind: match &handler.catch {
                        crate::CatchType::Any => HandlerKind::CatchAll,
                        crate::CatchType::Type(_) => HandlerKind::Catch,
                    },
                }
            })
            .collect();
        let parent = drafts[..index]
            .iter()
            .enumerate()
            .filter(|(_, candidate)| strictly_contains(candidate.protected, draft.protected))
            .min_by_key(|(_, candidate)| exception_range_span(candidate.protected))
            .map(|(parent_index, _)| RegionId::from_index(parent_index));

        let region = cfg.add_region(Region {
            id: RegionId::from_raw(0),
            protected_blocks,
            handlers,
            parent,
        });
        for (handler_position, &handler_index) in draft.handler_indices.iter().enumerate() {
            handler_refs[handler_index] = Some(HandlerRef::new(region, handler_position));
        }
    }
    handler_refs
        .into_iter()
        .map(|handler| handler.expect("every validated handler is assigned to one region"))
        .collect()
}

fn strictly_contains(outer: crate::AddressRange, inner: crate::AddressRange) -> bool {
    outer != inner && outer.start <= inner.start && inner.end <= outer.end
}

fn exception_range_span(range: crate::AddressRange) -> u64 {
    range.end.get() - range.start.get()
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use cfglib::{
        BlockId, Cfg, Direction, DominatorTree, Edge, EdgeKind, EdgeProblem, EdgeRef, HandlerBody,
        HandlerKind, HandlerRef, solve_edge_problem, verify_edge_view,
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
        assert_eq!(graph.cfg().block_count(), 4);
        assert_eq!(graph.cfg().edge_count(), 4);
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
        let region = &graph.cfg().regions()[0];
        assert_eq!(graph.cfg().regions().len(), 1);
        assert_eq!(
            region.protected_blocks,
            exceptional
                .iter()
                .map(|edge| edge.source())
                .collect::<BTreeSet<_>>()
        );
        assert_eq!(region.handlers[0].entry, handler);
        assert_eq!(region.handlers[0].body, HandlerBody::Unknown);
        assert_eq!(region.handlers[0].kind, HandlerKind::CatchAll);

        let exception_model = graph.exception_model();
        assert_eq!(exception_model.landing_pads(), [handler]);
        assert_eq!(exception_model.eh_edges.len(), exceptional.len());
        assert_eq!(exception_model.protected_by[&handler].len(), 2);
        assert_eq!(
            exception_model.handlers[&handler],
            vec![HandlerRef::new(region.id, 0)]
        );
        for modeled in &exception_model.eh_edges {
            assert!(graph.cfg().edge(modeled.edge_id).payload().is_exceptional());
        }
        assert!(DominatorTree::compute(graph.cfg()).is_reachable(handler));
        assert!(verify_edge_view(&graph.normal_view()).is_ok());
        assert!(!DominatorTree::compute(&graph.normal_view()).is_reachable(handler));
    }

    #[test]
    fn registers_nested_regions_and_ordered_unknown_handlers() {
        let inner_range = AddressRange::new(CodeAddress::from(1_u32), CodeAddress::from(3_u32));
        let outer_range = AddressRange::new(CodeAddress::from(0_u32), CodeAddress::from(4_u32));
        let inner_handler = CodeAddress::from(4_u32);
        let outer_fallback = CodeAddress::from(5_u32);
        let body = FunctionBody::new(
            AddressUnit::Byte,
            vec![
                instruction(0, InstructionFlow::FallThrough),
                instruction(1, InstructionFlow::FallThrough),
                instruction(2, InstructionFlow::FallThrough),
                instruction(3, InstructionFlow::Return),
                instruction(4, InstructionFlow::Return),
                instruction(5, InstructionFlow::Return),
            ],
            vec![
                ExceptionHandler {
                    protected: inner_range,
                    handler: inner_handler,
                    catch: CatchType::Type("example/Inner".into()),
                },
                ExceptionHandler {
                    protected: outer_range,
                    handler: inner_handler,
                    catch: CatchType::Type("example/Outer".into()),
                },
                ExceptionHandler {
                    protected: outer_range,
                    handler: outer_fallback,
                    catch: CatchType::Any,
                },
            ],
        );

        let graph = build_control_flow_graph(&body).unwrap();
        let [outer, inner] = graph.cfg().regions() else {
            panic!("expected one outer and one inner exception region");
        };
        assert_eq!(outer.parent, None);
        assert_eq!(inner.parent, Some(outer.id));
        assert_eq!(outer.protected_blocks.len(), 4);
        assert_eq!(inner.protected_blocks.len(), 2);
        assert_eq!(
            outer
                .handlers
                .iter()
                .map(|handler| handler.kind)
                .collect::<Vec<_>>(),
            vec![HandlerKind::Catch, HandlerKind::CatchAll]
        );
        assert_eq!(inner.handlers[0].kind, HandlerKind::Catch);
        assert!(
            outer
                .handlers
                .iter()
                .chain(&inner.handlers)
                .all(|handler| handler.body == HandlerBody::Unknown)
        );
        assert_eq!(
            outer.handlers[0].entry,
            graph.block_for_instruction(inner_handler).unwrap()
        );
        assert_eq!(
            outer.handlers[1].entry,
            graph.block_for_instruction(outer_fallback).unwrap()
        );
        assert_eq!(
            graph.exception_handler_ref(ExceptionHandlerIndex::from_index(0)),
            Some(HandlerRef::new(inner.id, 0))
        );
        assert_eq!(
            graph.exception_handler_ref(ExceptionHandlerIndex::from_index(1)),
            Some(HandlerRef::new(outer.id, 0))
        );
        assert_eq!(
            graph.exception_handler_ref(ExceptionHandlerIndex::from_index(2)),
            Some(HandlerRef::new(outer.id, 1))
        );

        let entry = graph.block_for_instruction(CodeAddress::ZERO).unwrap();
        let outer_handler_indices = graph
            .cfg()
            .successor_edges(entry)
            .iter()
            .filter_map(|&edge| match graph.cfg().edge(edge).payload().role() {
                ControlFlowEdgeRole::Exception { handler, .. } => Some(*handler),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            outer_handler_indices,
            vec![
                ExceptionHandlerIndex::from_index(1),
                ExceptionHandlerIndex::from_index(2),
            ],
            "region grouping must not renumber native exception-table entries"
        );
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

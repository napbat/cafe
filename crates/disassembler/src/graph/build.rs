//! Basic-block discovery and cfglib edge construction.

use cfglib::{
    AddressBuildError, AddressEdgeInfo, AddressEdgeRole, AddressFlow, AddressGraph, AddressHandler,
    AddressInstruction, AddressSpace, HandlerKind, HandlerTypes, build_address_cfg, verify_with,
};

use super::validate::ControlFlowValidator;
use super::{
    ControlFlowEdge, ControlFlowEdgeRole, ControlFlowGraph, ExceptionHandlerIndex, GraphError,
};
use crate::{CatchType, CodeAddress, FunctionBody, Instruction, InstructionFlow};

impl AddressSpace for CodeAddress {
    fn distance_from(self, earlier: Self) -> u64 {
        self.get() - earlier.get()
    }
}

impl AddressInstruction for Instruction {
    type Address = CodeAddress;
    type CaseKey = i64;

    fn address(&self) -> CodeAddress {
        self.address
    }

    fn end_address(&self) -> Option<CodeAddress> {
        self.checked_end()
    }

    fn flow(&self) -> AddressFlow<CodeAddress, i64> {
        match &self.flow {
            InstructionFlow::FallThrough => AddressFlow::FallThrough,
            InstructionFlow::ConditionalBranch { target } => {
                AddressFlow::Conditional { target: *target }
            }
            InstructionFlow::UnconditionalBranch { target } => {
                AddressFlow::Unconditional { target: *target }
            }
            InstructionFlow::Switch { default, cases } => AddressFlow::Switch {
                default: *default,
                cases: cases.iter().map(|case| (case.key, case.target)).collect(),
            },
            InstructionFlow::Return => AddressFlow::Return,
            InstructionFlow::Throw => AddressFlow::Throw,
            InstructionFlow::IndirectBranch => AddressFlow::Indirect,
            InstructionFlow::SubroutineCall { target } => AddressFlow::Call { target: *target },
        }
    }

    fn retains_exception_edge(&self) -> bool {
        self.exception_behavior.retains_exception_edge()
    }
}

/// Builds a verified cfglib control-flow graph from a function body.
///
/// Leaders are introduced at the entry, direct branch targets, instructions
/// following terminators, and exception-range boundaries. Exception handlers
/// produce `ExceptionUnwind` edges from protected instructions whose native
/// semantics may throw, plus ordered region metadata with explicitly unknown
/// handler-body extents.
///
/// # Errors
///
/// Returns an error if instruction ranges overlap, a target is not an
/// instruction boundary, exception metadata is invalid, or cfglib reports a
/// structural invariant violation.
pub fn build_control_flow_graph(body: &FunctionBody) -> Result<ControlFlowGraph, GraphError> {
    let handlers = body
        .exception_handlers
        .iter()
        .map(|handler| AddressHandler {
            protected: handler.protected.start..handler.protected.end,
            entry: handler.handler,
            kind: match &handler.catch {
                CatchType::Any => HandlerKind::CatchAll,
                CatchType::Type(_) => HandlerKind::Catch,
            },
        })
        .collect::<Vec<_>>();

    let AddressGraph {
        mut cfg,
        instruction_blocks,
        handler_refs,
    } = build_address_cfg(body.instructions.clone(), &handlers, |info| {
        edge_payload(body, info)
    })
    .map_err(graph_error)?;

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
    for (block, address) in block_starts {
        cfg.block_mut(block).set_label(format!("address_{address}"));
    }

    let mut handler_types = HandlerTypes::new();
    for (handler, definition) in handler_refs.iter().copied().zip(&body.exception_handlers) {
        if let CatchType::Type(catch_type) = &definition.catch {
            handler_types.set(handler, catch_type.clone());
        }
    }

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
        handler_types,
    ))
}

fn edge_payload(
    body: &FunctionBody,
    info: AddressEdgeInfo<'_, CodeAddress, i64>,
) -> ControlFlowEdge {
    let role = match info.role {
        AddressEdgeRole::Sequential => ControlFlowEdgeRole::Sequential,
        AddressEdgeRole::ConditionalTaken => ControlFlowEdgeRole::ConditionalTaken,
        AddressEdgeRole::ConditionalFallThrough => ControlFlowEdgeRole::ConditionalFallThrough,
        AddressEdgeRole::Branch => ControlFlowEdgeRole::DirectBranch,
        AddressEdgeRole::SwitchDefault => ControlFlowEdgeRole::SwitchDefault,
        AddressEdgeRole::SwitchCase { key } => ControlFlowEdgeRole::SwitchCase { key: *key },
        AddressEdgeRole::Call => ControlFlowEdgeRole::SubroutineCall,
        AddressEdgeRole::CallContinuation { call_site } => {
            ControlFlowEdgeRole::SubroutineContinuation { call_site }
        }
        AddressEdgeRole::Unwind { handler } => {
            let definition = &body.exception_handlers[handler];
            ControlFlowEdgeRole::Exception {
                handler: ExceptionHandlerIndex::from_index(handler),
                protected: definition.protected,
                catch: definition.catch.clone(),
            }
        }
    };
    ControlFlowEdge::new(info.source, info.target, role)
}

fn graph_error(error: AddressBuildError<CodeAddress>) -> GraphError {
    match error {
        AddressBuildError::ZeroSizeInstruction { address } => {
            GraphError::ZeroInstructionSize { address }
        }
        AddressBuildError::OverlappingInstruction {
            address,
            previous_end,
        } => GraphError::OverlappingInstruction {
            address,
            previous_end,
        },
        AddressBuildError::AddressOverflow { address } => GraphError::AddressOverflow { address },
        AddressBuildError::MissingBranchTarget { source, target } => {
            GraphError::MissingBranchTarget {
                source_address: source,
                target,
            }
        }
        AddressBuildError::EmptyProtectedRange { start, end } => {
            GraphError::InvalidExceptionRange { start, end }
        }
        AddressBuildError::MissingRangeStart { address } => {
            GraphError::MissingExceptionStart { address }
        }
        AddressBuildError::MissingRangeEnd { address } => {
            GraphError::MissingExceptionEnd { address }
        }
        AddressBuildError::MissingHandlerEntry { address } => {
            GraphError::MissingExceptionHandler { address }
        }
    }
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
        ControlFlowEdgeRole, ExceptionBehavior, ExceptionHandler, ExceptionHandlerIndex,
        FunctionBody, Instruction, InstructionFlow, SwitchCase,
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
    fn omits_exception_edges_only_for_instructions_known_not_to_throw() {
        let body = FunctionBody::new(
            AddressUnit::Byte,
            vec![
                instruction(0, InstructionFlow::FallThrough)
                    .with_exception_behavior(ExceptionBehavior::CannotThrow),
                instruction(1, InstructionFlow::FallThrough)
                    .with_exception_behavior(ExceptionBehavior::MayThrow),
                instruction(2, InstructionFlow::Return),
                instruction(3, InstructionFlow::Return),
            ],
            vec![ExceptionHandler {
                protected: AddressRange::new(CodeAddress::ZERO, CodeAddress::from(3_u32)),
                handler: CodeAddress::from(3_u32),
                catch: CatchType::Any,
            }],
        );

        let graph = build_control_flow_graph(&body).unwrap();
        let throw_sites = graph
            .cfg()
            .edges()
            .filter(|edge| edge.kind() == EdgeKind::ExceptionUnwind)
            .map(|edge| edge.payload().source())
            .collect::<Vec<_>>();
        assert_eq!(
            throw_sites,
            vec![CodeAddress::from(1_u32), CodeAddress::from(2_u32)],
            "unknown behavior remains conservative"
        );
    }

    #[test]
    #[allow(clippy::too_many_lines)]
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
        let inner_ref = graph
            .exception_handler_ref(ExceptionHandlerIndex::from_index(0))
            .unwrap();
        let outer_ref = graph
            .exception_handler_ref(ExceptionHandlerIndex::from_index(1))
            .unwrap();
        let fallback_ref = graph
            .exception_handler_ref(ExceptionHandlerIndex::from_index(2))
            .unwrap();
        assert_eq!(
            graph.exception_handler_type(inner_ref),
            Some("example/Inner")
        );
        assert_eq!(
            graph.exception_handler_type(outer_ref),
            Some("example/Outer")
        );
        assert_eq!(graph.exception_handler_type(fallback_ref), None);
        assert_eq!(graph.exception_handler_types().len(), 2);

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

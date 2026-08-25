//! Exact and conservatively recovered exception-handler structure.

use std::collections::{BTreeMap, BTreeSet};

use cfglib::{BlockId, EdgeId, HandlerRef, TraversalDirection, reachable};

use super::{ControlFlowEdgeRole, ControlFlowGraph, ExceptionHandlerIndex};
use crate::{CatchType, CodeAddress, ExceptionHandler, InstructionFlow};

/// Why a handler's complete body cannot be recovered unambiguously.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum HandlerExtentIssue {
    /// Ordinary method flow can enter the handler entry block.
    EntryReachableFromNormalFlow,
    /// Another distinct handler entry can reach this handler entry normally.
    EntryReachableFromAnotherHandler,
    /// A conservatively owned interior block has a normal predecessor outside
    /// the recovered body.
    ExternalEntry {
        /// Interior block receiving the external edge.
        block: BlockId,
        /// Source block outside the recovered body.
        predecessor: BlockId,
    },
    /// The handler contains an indirect transfer whose destination is absent
    /// from the shared CFG.
    IndirectControlFlow {
        /// Block ending in the indirect transfer.
        block: BlockId,
    },
    /// A nonterminal instruction is missing at least one required normal
    /// successor.
    UnresolvedExit {
        /// Block whose continuation is unavailable.
        block: BlockId,
    },
}

/// Confidence category of a conservatively recovered handler extent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HandlerExtentStatus {
    /// The handler-owned subgraph is closed under represented normal flow.
    Isolated,
    /// The exclusive handler blocks flow into one or more shared continuations.
    SharedContinuation,
    /// One or more structural facts prevent an unambiguous boundary.
    Ambiguous,
}

/// Conservative block ownership and boundary evidence for one handler body.
///
/// [`Self::blocks`] contains only blocks reachable from this handler entry that
/// are unreachable from the method entry and every other distinct handler
/// entry. Shared tails are reported in [`Self::boundary_blocks`] rather than
/// claimed as part of the body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecoveredHandlerExtent {
    /// Blocks exclusively reachable from this handler entry.
    pub blocks: BTreeSet<BlockId>,
    /// Direct normal successors deliberately excluded because they are shared.
    pub boundary_blocks: BTreeSet<BlockId>,
    /// Deterministically ordered reasons the extent is ambiguous.
    pub issues: Vec<HandlerExtentIssue>,
}

impl RecoveredHandlerExtent {
    /// Classifies the recovered extent from its boundary and ambiguity evidence.
    #[must_use]
    pub fn status(&self) -> HandlerExtentStatus {
        if !self.issues.is_empty() {
            HandlerExtentStatus::Ambiguous
        } else if self.boundary_blocks.is_empty() {
            HandlerExtentStatus::Isolated
        } else {
            HandlerExtentStatus::SharedContinuation
        }
    }
}

/// Observable bytecode-level behavior of a catch-all handler.
///
/// This classification never claims that the source language used `finally`.
/// In particular, [`Self::ThrowingCleanup`] means only that every represented
/// exit from an isolated recovered body is a throw.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CatchAllBehavior {
    /// Every represented exit throws, matching an unwind-cleanup shape.
    ThrowingCleanup,
    /// Every represented exit returns normally from the function.
    CompletesNormally,
    /// The isolated body has both throwing and normally returning exits.
    MixedCompletion,
    /// The isolated body has no represented exit, such as a closed loop.
    NoObservableExit,
    /// Ambiguity or a shared continuation prevents reliable classification.
    Unresolved,
}

/// Recovered semantic category of one native handler-table entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RecoveredHandlerSemantics {
    /// A typed catch handler; the exact type remains in its definition.
    Catch,
    /// A catch-all handler and its observable exit behavior.
    CatchAll(CatchAllBehavior),
}

/// One exact exceptional transfer into a handler.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ExceptionThrowSite {
    /// Stable cfglib edge identity.
    pub edge: EdgeId,
    /// Isolated source block containing the potentially throwing instruction.
    pub block: BlockId,
    /// Exact native instruction address.
    pub address: CodeAddress,
}

/// Exact metadata and derived structure for one ordered handler-table entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecoveredExceptionHandler {
    /// Stable position in the source artifact's handler table.
    pub index: ExceptionHandlerIndex,
    /// Direct identity of the corresponding cfglib region handler.
    pub cfglib_handler: HandlerRef,
    /// Exact protected range, handler address, and caught type.
    pub definition: ExceptionHandler,
    /// cfglib block containing the exact handler entry address.
    pub entry_block: BlockId,
    /// Every exact exceptional edge selecting this table entry.
    pub throw_sites: Vec<ExceptionThrowSite>,
    /// Conservative recovered body and its boundary evidence.
    pub extent: RecoveredHandlerExtent,
    /// Typed-catch or catch-all bytecode behavior.
    pub semantics: RecoveredHandlerSemantics,
}

/// Exact and conservatively recovered EH data in native handler-table order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecoveredExceptionModel {
    handlers: Vec<RecoveredExceptionHandler>,
}

impl RecoveredExceptionModel {
    /// Computes recovered handler structure without mutating the canonical CFG.
    #[must_use]
    pub fn compute(graph: &ControlFlowGraph) -> Self {
        let cfg = graph.cfg();
        let normal_view = graph.normal_view();
        let method_reachable = reachable(&normal_view, [cfg.entry()], TraversalDirection::Outgoing);
        let handler_entries = graph
            .handler_refs
            .iter()
            .map(|&handler| handler_entry(cfg, handler))
            .collect::<BTreeSet<_>>();
        let handler_reachability = handler_entries
            .iter()
            .map(|&entry| {
                (
                    entry,
                    reachable(&normal_view, [entry], TraversalDirection::Outgoing),
                )
            })
            .collect::<BTreeMap<_, _>>();

        let handlers = graph
            .exception_handlers
            .iter()
            .cloned()
            .enumerate()
            .map(|(position, definition)| {
                let index = ExceptionHandlerIndex::from_index(position);
                let cfglib_handler = graph.handler_refs[position];
                let entry_block = handler_entry(cfg, cfglib_handler);
                let extent =
                    recover_extent(graph, entry_block, &method_reachable, &handler_reachability);
                let semantics = recover_semantics(&definition.catch, &extent, graph);
                let throw_sites = cfg
                    .edges()
                    .filter_map(|edge| match edge.payload().role() {
                        ControlFlowEdgeRole::Exception { handler, .. } if *handler == index => {
                            Some(ExceptionThrowSite {
                                edge: edge.id(),
                                block: edge.source(),
                                address: edge.payload().source(),
                            })
                        }
                        _ => None,
                    })
                    .collect();

                RecoveredExceptionHandler {
                    index,
                    cfglib_handler,
                    definition,
                    entry_block,
                    throw_sites,
                    extent,
                    semantics,
                }
            })
            .collect();
        Self { handlers }
    }

    /// Returns handlers in exact source table order.
    #[must_use]
    pub fn handlers(&self) -> &[RecoveredExceptionHandler] {
        &self.handlers
    }

    /// Returns the handler at an exact native table index.
    #[must_use]
    pub fn handler(&self, index: ExceptionHandlerIndex) -> Option<&RecoveredExceptionHandler> {
        self.handlers.get(index.index())
    }

    /// Finds the native handler represented by a cfglib handler identity.
    #[must_use]
    pub fn handler_by_ref(&self, handler: HandlerRef) -> Option<&RecoveredExceptionHandler> {
        self.handlers
            .iter()
            .find(|candidate| candidate.cfglib_handler == handler)
    }

    /// Number of native handler-table entries.
    #[must_use]
    pub fn len(&self) -> usize {
        self.handlers.len()
    }

    /// Whether the model contains no exception handlers.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.handlers.is_empty()
    }
}

fn handler_entry(
    cfg: &cfglib::Cfg<crate::Instruction, super::ControlFlowEdge>,
    handler: HandlerRef,
) -> BlockId {
    cfg.regions()[handler.region().index()].handlers[handler.index()].entry
}

fn recover_extent(
    graph: &ControlFlowGraph,
    entry: BlockId,
    method_reachable: &[bool],
    handler_reachability: &BTreeMap<BlockId, Vec<bool>>,
) -> RecoveredHandlerExtent {
    let cfg = graph.cfg();
    let reachable_from_entry = &handler_reachability[&entry];
    let blocks = cfg
        .blocks()
        .iter()
        .map(cfglib::BasicBlock::id)
        .filter(|block| {
            reachable_from_entry[block.index()]
                && !method_reachable[block.index()]
                && handler_reachability
                    .iter()
                    .all(|(&other, reachable)| other == entry || !reachable[block.index()])
        })
        .collect::<BTreeSet<_>>();

    let mut boundary_blocks = BTreeSet::new();
    if !blocks.contains(&entry) {
        boundary_blocks.insert(entry);
    }
    for &block in &blocks {
        for &edge in cfg.successor_edges(block) {
            let edge = cfg.edge(edge);
            if !edge.payload().is_exceptional() && !blocks.contains(&edge.target()) {
                boundary_blocks.insert(edge.target());
            }
        }
    }

    let mut issues = BTreeSet::new();
    if method_reachable[entry.index()] {
        issues.insert(HandlerExtentIssue::EntryReachableFromNormalFlow);
    }
    if handler_reachability
        .iter()
        .any(|(&other, reachable)| other != entry && reachable[entry.index()])
    {
        issues.insert(HandlerExtentIssue::EntryReachableFromAnotherHandler);
    }
    for &block in &blocks {
        if block != entry {
            for &edge in cfg.predecessor_edges(block) {
                let edge = cfg.edge(edge);
                if !edge.payload().is_exceptional() && !blocks.contains(&edge.source()) {
                    issues.insert(HandlerExtentIssue::ExternalEntry {
                        block,
                        predecessor: edge.source(),
                    });
                }
            }
        }

        let Some(instruction) = cfg.block(block).instructions().last() else {
            continue;
        };
        let represented_normal_successors = cfg
            .successor_edges(block)
            .iter()
            .filter(|&&edge| !cfg.edge(edge).payload().is_exceptional())
            .count();
        match &instruction.flow {
            InstructionFlow::IndirectBranch => {
                issues.insert(HandlerExtentIssue::IndirectControlFlow { block });
            }
            InstructionFlow::Return | InstructionFlow::Throw => {}
            flow if represented_normal_successors < required_normal_successors(flow) => {
                issues.insert(HandlerExtentIssue::UnresolvedExit { block });
            }
            _ => {}
        }
    }

    RecoveredHandlerExtent {
        blocks,
        boundary_blocks,
        issues: issues.into_iter().collect(),
    }
}

fn required_normal_successors(flow: &InstructionFlow) -> usize {
    match flow {
        InstructionFlow::FallThrough | InstructionFlow::UnconditionalBranch { .. } => 1,
        InstructionFlow::ConditionalBranch { .. } | InstructionFlow::SubroutineCall { .. } => 2,
        InstructionFlow::Switch { cases, .. } => cases.len() + 1,
        InstructionFlow::Return | InstructionFlow::Throw | InstructionFlow::IndirectBranch => 0,
    }
}

fn recover_semantics(
    catch: &CatchType,
    extent: &RecoveredHandlerExtent,
    graph: &ControlFlowGraph,
) -> RecoveredHandlerSemantics {
    if matches!(catch, CatchType::Type(_)) {
        return RecoveredHandlerSemantics::Catch;
    }
    if extent.status() != HandlerExtentStatus::Isolated {
        return RecoveredHandlerSemantics::CatchAll(CatchAllBehavior::Unresolved);
    }

    let cfg = graph.cfg();
    let mut throws = 0_usize;
    let mut returns = 0_usize;
    for &block in &extent.blocks {
        let has_normal_successor = cfg
            .successor_edges(block)
            .iter()
            .any(|&edge| !cfg.edge(edge).payload().is_exceptional());
        if has_normal_successor {
            continue;
        }
        match cfg
            .block(block)
            .instructions()
            .last()
            .map(|item| &item.flow)
        {
            Some(InstructionFlow::Throw) => throws += 1,
            Some(InstructionFlow::Return) => returns += 1,
            _ => {
                return RecoveredHandlerSemantics::CatchAll(CatchAllBehavior::Unresolved);
            }
        }
    }

    let behavior = match (throws, returns) {
        (0, 0) => CatchAllBehavior::NoObservableExit,
        (_, 0) => CatchAllBehavior::ThrowingCleanup,
        (0, _) => CatchAllBehavior::CompletesNormally,
        _ => CatchAllBehavior::MixedCompletion,
    };
    RecoveredHandlerSemantics::CatchAll(behavior)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AddressRange, AddressUnit, CodeSize, FunctionBody, Instruction};

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

    fn handler(
        protected_start: u32,
        protected_end: u32,
        entry: u32,
        catch: CatchType,
    ) -> ExceptionHandler {
        ExceptionHandler {
            protected: AddressRange::new(
                CodeAddress::from(protected_start),
                CodeAddress::from(protected_end),
            ),
            handler: CodeAddress::from(entry),
            catch,
        }
    }

    #[test]
    fn preserves_exact_order_mapping_and_throwing_cleanup_evidence() {
        let body = FunctionBody::new(
            AddressUnit::Byte,
            vec![
                instruction(0, InstructionFlow::Return),
                instruction(1, InstructionFlow::Throw),
            ],
            vec![
                handler(0, 1, 1, CatchType::Type("example/Error".into())),
                handler(0, 1, 1, CatchType::Any),
            ],
        );
        let graph = body.control_flow_graph().unwrap();
        let model = graph.recovered_exception_model();

        assert_eq!(model.len(), 2);
        let typed = model.handler(ExceptionHandlerIndex::from_index(0)).unwrap();
        let catch_all = model.handler(ExceptionHandlerIndex::from_index(1)).unwrap();
        assert_eq!(typed.definition, body.exception_handlers[0]);
        assert_eq!(catch_all.definition, body.exception_handlers[1]);
        assert_eq!(typed.entry_block, catch_all.entry_block);
        assert_ne!(typed.cfglib_handler, catch_all.cfglib_handler);
        assert_eq!(typed.semantics, RecoveredHandlerSemantics::Catch);
        assert_eq!(
            catch_all.semantics,
            RecoveredHandlerSemantics::CatchAll(CatchAllBehavior::ThrowingCleanup)
        );
        assert_eq!(catch_all.extent.status(), HandlerExtentStatus::Isolated);
        assert_eq!(
            catch_all.extent.blocks,
            [catch_all.entry_block].into_iter().collect()
        );
        assert!(catch_all.extent.boundary_blocks.is_empty());
        assert!(catch_all.extent.issues.is_empty());
        assert_eq!(typed.throw_sites.len(), 1);
        assert_eq!(catch_all.throw_sites.len(), 1);
        assert_ne!(typed.throw_sites[0].edge, catch_all.throw_sites[0].edge);
        assert_eq!(typed.throw_sites[0].address, CodeAddress::ZERO);
        assert_eq!(
            graph.exception_handler_ref(typed.index),
            Some(typed.cfglib_handler)
        );
        assert_eq!(
            graph.exception_handler_index(catch_all.cfglib_handler),
            Some(catch_all.index)
        );
        assert_eq!(model.handler_by_ref(typed.cfglib_handler), Some(typed));
    }

    #[test]
    fn stops_before_a_shared_normal_continuation() {
        let body = FunctionBody::new(
            AddressUnit::Byte,
            vec![
                instruction(0, InstructionFlow::FallThrough),
                instruction(1, InstructionFlow::Return),
                instruction(
                    2,
                    InstructionFlow::UnconditionalBranch {
                        target: CodeAddress::from(1_u32),
                    },
                ),
            ],
            vec![handler(0, 1, 2, CatchType::Any)],
        );
        let graph = body.control_flow_graph().unwrap();
        let model = graph.recovered_exception_model();
        let recovered = &model.handlers()[0];
        let continuation = graph
            .block_for_instruction(CodeAddress::from(1_u32))
            .unwrap();

        assert_eq!(
            recovered.extent.blocks,
            [recovered.entry_block].into_iter().collect()
        );
        assert_eq!(
            recovered.extent.boundary_blocks,
            [continuation].into_iter().collect()
        );
        assert_eq!(
            recovered.extent.status(),
            HandlerExtentStatus::SharedContinuation
        );
        assert_eq!(
            recovered.semantics,
            RecoveredHandlerSemantics::CatchAll(CatchAllBehavior::Unresolved)
        );
    }

    #[test]
    fn reports_a_handler_entry_reachable_from_method_flow() {
        let body = FunctionBody::new(
            AddressUnit::Byte,
            vec![
                instruction(0, InstructionFlow::FallThrough),
                instruction(1, InstructionFlow::Throw),
            ],
            vec![handler(0, 1, 1, CatchType::Any)],
        );
        let graph = body.control_flow_graph().unwrap();
        let model = graph.recovered_exception_model();
        let recovered = &model.handlers()[0];

        assert!(recovered.extent.blocks.is_empty());
        assert_eq!(
            recovered.extent.boundary_blocks,
            [recovered.entry_block].into_iter().collect()
        );
        assert_eq!(recovered.extent.status(), HandlerExtentStatus::Ambiguous);
        assert_eq!(
            recovered.extent.issues,
            vec![HandlerExtentIssue::EntryReachableFromNormalFlow]
        );
    }

    #[test]
    fn reports_indirect_handler_control_flow_as_ambiguous() {
        let body = FunctionBody::new(
            AddressUnit::Byte,
            vec![
                instruction(0, InstructionFlow::Return),
                instruction(1, InstructionFlow::IndirectBranch),
            ],
            vec![handler(0, 1, 1, CatchType::Any)],
        );
        let graph = body.control_flow_graph().unwrap();
        let model = graph.recovered_exception_model();
        let recovered = &model.handlers()[0];

        assert_eq!(recovered.extent.status(), HandlerExtentStatus::Ambiguous);
        assert_eq!(
            recovered.extent.issues,
            vec![HandlerExtentIssue::IndirectControlFlow {
                block: recovered.entry_block,
            }]
        );
        assert_eq!(
            recovered.semantics,
            RecoveredHandlerSemantics::CatchAll(CatchAllBehavior::Unresolved)
        );
    }

    #[test]
    fn reports_a_missing_handler_branch_arm_as_ambiguous() {
        let body = FunctionBody::new(
            AddressUnit::Byte,
            vec![
                instruction(0, InstructionFlow::Return),
                instruction(
                    1,
                    InstructionFlow::ConditionalBranch {
                        target: CodeAddress::from(1_u32),
                    },
                ),
            ],
            vec![handler(0, 1, 1, CatchType::Any)],
        );
        let graph = body.control_flow_graph().unwrap();
        let model = graph.recovered_exception_model();
        let recovered = &model.handlers()[0];

        assert_eq!(recovered.extent.status(), HandlerExtentStatus::Ambiguous);
        assert_eq!(
            recovered.extent.issues,
            vec![HandlerExtentIssue::UnresolvedExit {
                block: recovered.entry_block,
            }]
        );
    }

    #[test]
    fn reports_entry_reachable_from_another_handler() {
        let body = FunctionBody::new(
            AddressUnit::Byte,
            vec![
                instruction(0, InstructionFlow::Return),
                instruction(
                    1,
                    InstructionFlow::UnconditionalBranch {
                        target: CodeAddress::from(2_u32),
                    },
                ),
                instruction(2, InstructionFlow::Throw),
            ],
            vec![
                handler(0, 1, 1, CatchType::Type("example/First".into())),
                handler(0, 1, 2, CatchType::Any),
            ],
        );
        let graph = body.control_flow_graph().unwrap();
        let model = graph.recovered_exception_model();
        let second = &model.handlers()[1];

        assert!(second.extent.blocks.is_empty());
        assert_eq!(second.extent.status(), HandlerExtentStatus::Ambiguous);
        assert_eq!(
            second.extent.issues,
            vec![HandlerExtentIssue::EntryReachableFromAnotherHandler]
        );
    }

    #[test]
    fn reports_external_entry_into_handler_interior() {
        let body = FunctionBody::new(
            AddressUnit::Byte,
            vec![
                instruction(0, InstructionFlow::Return),
                instruction(
                    1,
                    InstructionFlow::UnconditionalBranch {
                        target: CodeAddress::from(3_u32),
                    },
                ),
                instruction(2, InstructionFlow::FallThrough),
                instruction(3, InstructionFlow::Throw),
            ],
            vec![handler(0, 1, 2, CatchType::Any)],
        );
        let graph = body.control_flow_graph().unwrap();
        let model = graph.recovered_exception_model();
        let recovered = &model.handlers()[0];
        let interior = graph
            .block_for_instruction(CodeAddress::from(3_u32))
            .unwrap();
        let predecessor = graph
            .block_for_instruction(CodeAddress::from(1_u32))
            .unwrap();

        assert_eq!(recovered.extent.status(), HandlerExtentStatus::Ambiguous);
        assert_eq!(
            recovered.extent.issues,
            vec![HandlerExtentIssue::ExternalEntry {
                block: interior,
                predecessor,
            }]
        );
    }

    #[test]
    fn classifies_isolated_catch_all_exit_shapes() {
        let returning = FunctionBody::new(
            AddressUnit::Byte,
            vec![
                instruction(0, InstructionFlow::Return),
                instruction(1, InstructionFlow::Return),
            ],
            vec![handler(0, 1, 1, CatchType::Any)],
        );
        let returning = returning.control_flow_graph().unwrap();
        assert_eq!(
            returning.recovered_exception_model().handlers()[0].semantics,
            RecoveredHandlerSemantics::CatchAll(CatchAllBehavior::CompletesNormally)
        );

        let mixed = FunctionBody::new(
            AddressUnit::Byte,
            vec![
                instruction(0, InstructionFlow::Return),
                instruction(
                    1,
                    InstructionFlow::ConditionalBranch {
                        target: CodeAddress::from(3_u32),
                    },
                ),
                instruction(2, InstructionFlow::Return),
                instruction(3, InstructionFlow::Throw),
            ],
            vec![handler(0, 1, 1, CatchType::Any)],
        );
        let mixed = mixed.control_flow_graph().unwrap();
        assert_eq!(
            mixed.recovered_exception_model().handlers()[0].semantics,
            RecoveredHandlerSemantics::CatchAll(CatchAllBehavior::MixedCompletion)
        );

        let looping = FunctionBody::new(
            AddressUnit::Byte,
            vec![
                instruction(0, InstructionFlow::Return),
                instruction(
                    1,
                    InstructionFlow::UnconditionalBranch {
                        target: CodeAddress::from(1_u32),
                    },
                ),
            ],
            vec![handler(0, 1, 1, CatchType::Any)],
        );
        let looping = looping.control_flow_graph().unwrap();
        assert_eq!(
            looping.recovered_exception_model().handlers()[0].semantics,
            RecoveredHandlerSemantics::CatchAll(CatchAllBehavior::NoObservableExit)
        );
    }
}

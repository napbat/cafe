//! Control-flow graph construction over shared disassembly IR.

mod build;
mod edge;
mod error;
mod exception;
mod structured;
mod validate;

use std::collections::BTreeMap;

use cfglib::{BlockId, Cfg, Edge, EdgeId, EhModel, FilteredEdges, HandlerRef, HandlerTypes};

use crate::{CodeAddress, ExceptionHandler, FunctionBody, Instruction};

pub use self::build::build_control_flow_graph;
pub use self::edge::{
    ControlFlowEdge, ControlFlowEdgeRole, ExceptionHandlerIndex, NormalControlFlow,
};
pub use self::error::GraphError;
pub use self::exception::{
    CatchAllBehavior, ExceptionThrowSite, HandlerExtentIssue, HandlerExtentStatus,
    RecoveredExceptionHandler, RecoveredExceptionModel, RecoveredHandlerExtent,
    RecoveredHandlerSemantics,
};
pub use self::structured::{
    RecoveredStructuredControlFlow, StructuredRegionDecision, StructuredRegionStatus,
};

use self::edge::is_normal_edge;

/// A verified cfglib CFG with source-address-to-block lookup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ControlFlowGraph {
    cfg: Cfg<Instruction, ControlFlowEdge>,
    instruction_blocks: BTreeMap<CodeAddress, BlockId>,
    exception_handlers: Vec<ExceptionHandler>,
    handler_refs: Vec<HandlerRef>,
    handler_types: HandlerTypes<String>,
}

impl ControlFlowGraph {
    pub(crate) const fn new(
        cfg: Cfg<Instruction, ControlFlowEdge>,
        instruction_blocks: BTreeMap<CodeAddress, BlockId>,
        exception_handlers: Vec<ExceptionHandler>,
        handler_refs: Vec<HandlerRef>,
        handler_types: HandlerTypes<String>,
    ) -> Self {
        Self {
            cfg,
            instruction_blocks,
            exception_handlers,
            handler_refs,
            handler_types,
        }
    }

    /// Returns the underlying graph for cfglib algorithms.
    #[must_use]
    pub const fn cfg(&self) -> &Cfg<Instruction, ControlFlowEdge> {
        &self.cfg
    }

    /// Consumes this wrapper and returns the underlying cfglib graph.
    #[must_use]
    pub fn into_cfg(self) -> Cfg<Instruction, ControlFlowEdge> {
        self.cfg
    }

    /// Returns a zero-copy graph view that excludes exceptional edges while
    /// retaining every block and stable ordinary edge identity.
    #[must_use]
    pub fn normal_view(&self) -> NormalControlFlow<'_> {
        FilteredEdges::new(
            &self.cfg,
            is_normal_edge as fn(EdgeId, &Edge<ControlFlowEdge>) -> bool,
        )
    }

    /// Computes cfglib's generic exception-flow projection.
    ///
    /// The projection classifies landing pads and protected source blocks from
    /// structural edge kinds. Regions retain ordered handler entries and catch
    /// classifications, with [`cfglib::HandlerBody::Unknown`] recording that
    /// JVM and DEX exception tables do not encode complete handler extents.
    /// Exact handler order, catch types, protected ranges, and throw sites
    /// remain available through each model edge's stable identity and the
    /// corresponding [`ControlFlowEdge`] payload.
    #[must_use]
    pub fn exception_model(&self) -> EhModel {
        EhModel::compute(&self.cfg)
    }

    /// Recovers conservative handler extents and catch-all behavior while
    /// retaining exact native handler and edge identities.
    #[must_use]
    pub fn recovered_exception_model(&self) -> RecoveredExceptionModel {
        RecoveredExceptionModel::compute(self)
    }

    /// Builds a derived CFG for conservative cfglib structured lifting.
    ///
    /// Only complete, non-overlapping recovered handler extents are promoted;
    /// the canonical graph and ambiguous native control flow remain unchanged.
    #[must_use]
    pub fn recovered_structured_control_flow(&self) -> RecoveredStructuredControlFlow {
        RecoveredStructuredControlFlow::compute(self)
    }

    /// Returns exact handler definitions in native table order.
    #[must_use]
    pub fn exception_handlers(&self) -> &[ExceptionHandler] {
        &self.exception_handlers
    }

    /// Returns exact format-native caught types keyed by cfglib handler identity.
    ///
    /// Catch-all handlers intentionally have no entry; their classification is
    /// represented by [`cfglib::HandlerKind::CatchAll`] on the region handler.
    #[must_use]
    pub const fn exception_handler_types(&self) -> &HandlerTypes<String> {
        &self.handler_types
    }

    /// Returns the exact format-native caught type for a typed handler.
    #[must_use]
    pub fn exception_handler_type(&self, handler: HandlerRef) -> Option<&str> {
        self.handler_types.metadata(handler).map(String::as_str)
    }

    /// Returns the cfglib handler corresponding to a native table index.
    #[must_use]
    pub fn exception_handler_ref(&self, index: ExceptionHandlerIndex) -> Option<HandlerRef> {
        self.handler_refs.get(index.index()).copied()
    }

    /// Returns the native table index corresponding to a cfglib handler.
    #[must_use]
    pub fn exception_handler_index(&self, handler: HandlerRef) -> Option<ExceptionHandlerIndex> {
        self.handler_refs
            .iter()
            .position(|&candidate| candidate == handler)
            .map(ExceptionHandlerIndex::from_index)
    }

    /// Returns the basic block containing an instruction start address.
    #[must_use]
    pub fn block_for_instruction(&self, address: CodeAddress) -> Option<BlockId> {
        self.instruction_blocks.get(&address).copied()
    }

    /// Renders this control-flow graph in Graphviz DOT format.
    #[must_use]
    pub fn to_dot(&self) -> String {
        self.cfg.to_dot()
    }
}

impl AsRef<Cfg<Instruction, ControlFlowEdge>> for ControlFlowGraph {
    fn as_ref(&self) -> &Cfg<Instruction, ControlFlowEdge> {
        self.cfg()
    }
}

impl FunctionBody {
    /// Builds and verifies this function's basic-block control-flow graph.
    ///
    /// # Errors
    ///
    /// Returns an error for invalid instruction sizes, ordering, branch
    /// targets, exception boundaries, or graph invariants.
    pub fn control_flow_graph(&self) -> Result<ControlFlowGraph, GraphError> {
        build_control_flow_graph(self)
    }
}

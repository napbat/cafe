//! Public JVM verification-frame and control-flow results.

use disassembler::cfglib::{
    DirectedGraphView, EdgeGraphView, EdgeId, EdgeRef, KeyedGraph, NodeId, RootedGraphView,
};

use crate::{Error, Result};

/// Symbolic verification value used before constant-pool stack-map encoding.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum FrameValue {
    /// Unusable local-variable slot.
    Top,
    /// Integer-like value.
    Integer,
    /// IEEE-754 single-precision value.
    Float,
    /// Signed 64-bit integer value.
    Long,
    /// IEEE-754 double-precision value.
    Double,
    /// Null reference.
    Null,
    /// Initialized object or array, stored as an internal name or array descriptor.
    Reference(String),
    /// Incoming receiver of a constructor before initialization.
    UninitializedThis,
    /// Object created by `new` before its constructor completes.
    Uninitialized {
        /// Internal class name selected by `new`.
        class: String,
        /// Bytecode offset of the allocation instruction.
        offset: u16,
    },
    /// Reserved local slot following a long or double value.
    WideContinuation,
}

impl FrameValue {
    /// Returns the number of JVM operand-stack or local slots occupied by a value.
    #[must_use]
    pub const fn slot_count(&self) -> usize {
        match self {
            Self::Long | Self::Double => 2,
            Self::Top
            | Self::Integer
            | Self::Float
            | Self::Null
            | Self::Reference(_)
            | Self::UninitializedThis
            | Self::Uninitialized { .. }
            | Self::WideContinuation => 1,
        }
    }

    pub(super) const fn is_category_two(&self) -> bool {
        matches!(self, Self::Long | Self::Double)
    }

    pub(super) const fn is_reference(&self) -> bool {
        matches!(
            self,
            Self::Null | Self::Reference(_) | Self::UninitializedThis | Self::Uninitialized { .. }
        )
    }
}

/// JVM local-variable and operand-stack state at one instruction boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrameState {
    pub(super) locals: Vec<FrameValue>,
    pub(super) stack: Vec<FrameValue>,
}

impl FrameState {
    /// Returns local slots from index zero upward.
    #[must_use]
    pub fn locals(&self) -> &[FrameValue] {
        &self.locals
    }

    /// Returns operand-stack values from bottom to top.
    #[must_use]
    pub fn stack(&self) -> &[FrameValue] {
        &self.stack
    }

    /// Returns current operand-stack depth in JVM slots.
    #[must_use]
    pub fn stack_slots(&self) -> usize {
        self.stack.iter().map(FrameValue::slot_count).sum()
    }
}

/// Reason for one JVM control-flow edge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FlowEdgeKind {
    /// Ordinary sequential execution.
    FallThrough,
    /// Direct branch or switch case.
    Branch,
    /// Exceptional transfer through one exception-table entry.
    Exception {
        /// Constant-pool class index, or zero for catch-all.
        catch_type: u16,
    },
}

/// One directed JVM instruction edge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FlowEdge {
    /// Source bytecode offset.
    pub source: usize,
    /// Target bytecode offset.
    pub target: usize,
    /// Reason execution can take the edge.
    pub kind: FlowEdgeKind,
}

/// Exception-aware control flow over decoded JVM instructions.
///
/// This is also a cfglib node, edge, and rooted view. Algorithms use dense
/// encoded-order node identities while [`FlowEdge`] retains exact bytecode
/// offsets for consumers and diagnostics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ControlFlow {
    entry: usize,
    edges: Vec<FlowEdge>,
    graph: KeyedGraph<usize, usize, FlowEdge>,
}

impl ControlFlow {
    pub(super) fn build(entry: usize, nodes: &[usize], edges: Vec<FlowEdge>) -> Result<Self> {
        let mut graph = KeyedGraph::new();
        for offset in nodes {
            graph.intern(offset);
        }
        for &edge in &edges {
            let source = graph.node_id(&edge.source).ok_or_else(|| {
                Error::invalid_bytecode(edge.source, "control-flow source is not an instruction")
            })?;
            let target = graph.node_id(&edge.target).ok_or_else(|| {
                Error::invalid_bytecode(edge.source, "control-flow target is not an instruction")
            })?;
            graph.add_edge(source, target, edge);
        }
        Ok(Self {
            entry,
            edges,
            graph,
        })
    }

    /// Returns the entry instruction offset.
    #[must_use]
    pub const fn entry(&self) -> usize {
        self.entry
    }

    /// Returns instruction offsets in encoded order.
    #[must_use]
    pub fn nodes(&self) -> &[usize] {
        self.graph.graph().nodes()
    }

    /// Returns typed directed edges in stable source order.
    #[must_use]
    pub fn edges(&self) -> &[FlowEdge] {
        &self.edges
    }

    /// Iterates over outgoing edges from one instruction.
    pub fn successors(&self, source: usize) -> impl Iterator<Item = &FlowEdge> {
        self.graph
            .node_id(&source)
            .into_iter()
            .flat_map(|node| self.graph.graph().outgoing_edges(node))
            .map(|&edge| self.graph.graph().edge(edge).payload())
    }

    pub(super) fn node_position(&self, offset: usize) -> Option<usize> {
        self.graph.node_id(&offset).map(NodeId::index)
    }

    pub(super) fn node_offset(&self, node: NodeId) -> usize {
        *self.graph.graph().node(node)
    }
}

impl DirectedGraphView for ControlFlow {
    type NodeId = NodeId;

    fn node_count(&self) -> usize {
        self.graph.graph().node_count()
    }

    fn successors(&self, node: NodeId) -> impl Iterator<Item = NodeId> + '_ {
        self.graph.graph().successors(node)
    }

    fn predecessors(&self, node: NodeId) -> impl Iterator<Item = NodeId> + '_ {
        self.graph.graph().predecessors(node)
    }
}

impl EdgeGraphView for ControlFlow {
    type EdgeId = EdgeId;
    type EdgeData = FlowEdge;

    fn edge_slot_count(&self) -> usize {
        self.graph.graph().edge_slot_count()
    }

    fn edge_ids(&self) -> impl Iterator<Item = EdgeId> + '_ {
        EdgeGraphView::edge_ids(&self.graph)
    }

    fn outgoing_edges(&self, node: NodeId) -> impl Iterator<Item = EdgeId> + '_ {
        self.graph.graph().outgoing_edges(node).iter().copied()
    }

    fn incoming_edges(&self, node: NodeId) -> impl Iterator<Item = EdgeId> + '_ {
        self.graph.graph().incoming_edges(node).iter().copied()
    }

    fn edge_ref(&self, edge: EdgeId) -> EdgeRef<'_, NodeId, EdgeId, FlowEdge> {
        EdgeGraphView::edge_ref(&self.graph, edge)
    }
}

impl RootedGraphView for ControlFlow {
    fn root(&self) -> NodeId {
        self.graph
            .node_id(&self.entry)
            .expect("control-flow entry is not an instruction node")
    }
}

/// Fixed-point JVM frames and exact resource maxima.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MethodAnalysis {
    pub(super) flow: ControlFlow,
    pub(super) entries: Vec<FrameState>,
    pub(super) exits: Vec<FrameState>,
    pub(super) max_stack: u16,
    pub(super) max_locals: u16,
}

impl MethodAnalysis {
    /// Returns the exception-aware instruction graph.
    #[must_use]
    pub const fn flow(&self) -> &ControlFlow {
        &self.flow
    }

    /// Returns the merged frame before a reachable instruction.
    #[must_use]
    pub fn entry_frame(&self, offset: usize) -> Option<&FrameState> {
        self.flow
            .node_position(offset)
            .and_then(|node| self.entries.get(node))
    }

    /// Returns the frame after normal completion of a reachable instruction.
    #[must_use]
    pub fn exit_frame(&self, offset: usize) -> Option<&FrameState> {
        self.flow
            .node_position(offset)
            .and_then(|node| self.exits.get(node))
    }

    /// Returns the exact maximum operand-stack depth in slots.
    #[must_use]
    pub const fn max_stack(&self) -> u16 {
        self.max_stack
    }

    /// Returns the minimum local-variable array size in slots.
    #[must_use]
    pub const fn max_locals(&self) -> u16 {
        self.max_locals
    }

    /// Iterates over reachable entry frames in bytecode order.
    pub fn entry_frames(&self) -> impl Iterator<Item = (usize, &FrameState)> {
        self.flow.nodes().iter().copied().zip(&self.entries)
    }

    pub(crate) fn frames_at(&self, instruction: usize) -> Option<(&FrameState, &FrameState)> {
        self.entries
            .get(instruction)
            .zip(self.exits.get(instruction))
    }
}

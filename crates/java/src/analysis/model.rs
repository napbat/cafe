//! Public JVM verification-frame and control-flow results.

use std::collections::BTreeMap;

use disassembler::cfglib::{DirectedGraphView, EdgeGraphView, EdgeRef, RootedGraphView};

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
/// encoded-order positions as node identities while [`FlowEdge`] retains exact
/// bytecode offsets for consumers and diagnostics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ControlFlow {
    pub(super) entry: usize,
    pub(super) nodes: Vec<usize>,
    pub(super) edges: Vec<FlowEdge>,
}

impl ControlFlow {
    /// Returns the entry instruction offset.
    #[must_use]
    pub const fn entry(&self) -> usize {
        self.entry
    }

    /// Returns instruction offsets in encoded order.
    #[must_use]
    pub fn nodes(&self) -> &[usize] {
        &self.nodes
    }

    /// Returns typed directed edges in stable source order.
    #[must_use]
    pub fn edges(&self) -> &[FlowEdge] {
        &self.edges
    }

    /// Iterates over outgoing edges from one instruction.
    pub fn successors(&self, source: usize) -> impl Iterator<Item = &FlowEdge> {
        self.edges.iter().filter(move |edge| edge.source == source)
    }

    pub(super) fn node_index(&self, offset: usize) -> usize {
        self.nodes
            .binary_search(&offset)
            .expect("control-flow edge endpoint is not an instruction node")
    }

    pub(super) fn node_offset(&self, node: usize) -> usize {
        self.nodes[node]
    }
}

impl DirectedGraphView for ControlFlow {
    type NodeId = usize;

    fn node_count(&self) -> usize {
        self.nodes.len()
    }

    fn successors(&self, node: usize) -> impl Iterator<Item = usize> + '_ {
        let source = self.node_offset(node);
        self.edges
            .iter()
            .filter(move |edge| edge.source == source)
            .map(|edge| self.node_index(edge.target))
    }

    fn predecessors(&self, node: usize) -> impl Iterator<Item = usize> + '_ {
        let target = self.node_offset(node);
        self.edges
            .iter()
            .filter(move |edge| edge.target == target)
            .map(|edge| self.node_index(edge.source))
    }
}

impl EdgeGraphView for ControlFlow {
    type EdgeId = usize;
    type EdgeData = FlowEdge;

    fn edge_slot_count(&self) -> usize {
        self.edges.len()
    }

    fn edge_ids(&self) -> impl Iterator<Item = usize> + '_ {
        0..self.edges.len()
    }

    fn outgoing_edges(&self, node: usize) -> impl Iterator<Item = usize> + '_ {
        let source = self.node_offset(node);
        self.edges
            .iter()
            .enumerate()
            .filter_map(move |(edge_id, edge)| (edge.source == source).then_some(edge_id))
    }

    fn incoming_edges(&self, node: usize) -> impl Iterator<Item = usize> + '_ {
        let target = self.node_offset(node);
        self.edges
            .iter()
            .enumerate()
            .filter_map(move |(edge_id, edge)| (edge.target == target).then_some(edge_id))
    }

    fn edge_ref(&self, edge: usize) -> EdgeRef<'_, usize, usize, FlowEdge> {
        let data = &self.edges[edge];
        EdgeRef::new(
            edge,
            self.node_index(data.source),
            self.node_index(data.target),
            data,
        )
    }
}

impl RootedGraphView for ControlFlow {
    fn root(&self) -> usize {
        self.node_index(self.entry)
    }
}

/// Fixed-point JVM frames and exact resource maxima.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MethodAnalysis {
    pub(super) flow: ControlFlow,
    pub(super) entries: BTreeMap<usize, FrameState>,
    pub(super) exits: BTreeMap<usize, FrameState>,
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
        self.entries.get(&offset)
    }

    /// Returns the frame after normal completion of a reachable instruction.
    #[must_use]
    pub fn exit_frame(&self, offset: usize) -> Option<&FrameState> {
        self.exits.get(&offset)
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
        self.entries.iter().map(|(&offset, frame)| (offset, frame))
    }
}

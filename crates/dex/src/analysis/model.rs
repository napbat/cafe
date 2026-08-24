//! Typed Dalvik register effects and logical result metadata.

use std::collections::BTreeMap;

use crate::file::TypeIndex;

/// Semantic category required or produced by a Dalvik register operand.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ValueKind {
    /// Unconstrained single-register value.
    Single,
    /// Unconstrained adjacent register pair.
    Wide,
    /// Integer-like value, including boolean, byte, char, and short values.
    Integer,
    /// IEEE 754 single-precision value.
    Float,
    /// Signed 64-bit integer value.
    Long,
    /// IEEE 754 double-precision value.
    Double,
    /// Object, array, or null reference.
    Reference,
}

impl ValueKind {
    /// Returns the number of DEX register words occupied by this category.
    #[must_use]
    pub const fn register_words(&self) -> u8 {
        match self {
            Self::Wide | Self::Long | Self::Double => 2,
            Self::Single | Self::Integer | Self::Float | Self::Reference => 1,
        }
    }
}

/// One typed register span read or written by a Dalvik instruction.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RegisterOperand {
    /// First register word.
    pub register: u16,
    /// Semantic value category occupying this register span.
    pub kind: ValueKind,
}

impl RegisterOperand {
    pub(super) const fn new(register: u16, kind: ValueKind) -> Self {
        Self { register, kind }
    }

    /// Returns the number of adjacent register words in this operand.
    #[must_use]
    pub const fn register_words(&self) -> u8 {
        self.kind.register_words()
    }
}

/// Value produced in DEX's implicit result slot rather than a named register.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProducedValue {
    /// Invocation result is determined by its method or call-site prototype.
    Prototype,
    /// Filled-array construction produces an array reference.
    Reference,
}

/// Register and exceptional behavior of one instruction-stream item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstructionSemantics {
    /// Registers read before the instruction executes.
    pub reads: Vec<RegisterOperand>,
    /// Registers defined when the instruction completes normally.
    pub writes: Vec<RegisterOperand>,
    /// Optional value placed in the implicit result slot.
    pub produced: Option<ProducedValue>,
    /// Whether ordinary execution may transfer through an exception handler.
    pub may_throw: bool,
    /// Whether this item is an executable opcode rather than a data payload.
    pub executable: bool,
}

impl InstructionSemantics {
    pub(super) fn operation(may_throw: bool) -> Self {
        Self {
            reads: Vec::new(),
            writes: Vec::new(),
            produced: None,
            may_throw,
            executable: true,
        }
    }

    pub(super) fn payload() -> Self {
        Self {
            reads: Vec::new(),
            writes: Vec::new(),
            produced: None,
            may_throw: false,
            executable: false,
        }
    }
}

/// Kind of payload selected by an executable Dalvik instruction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PayloadKind {
    /// Dense integer-switch table.
    PackedSwitch,
    /// Sparse integer-switch table.
    SparseSwitch,
    /// Encoded array initializer data.
    ArrayData,
}

/// One validated executable-instruction to payload association.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PayloadLink {
    /// Payload kind required by the referring opcode.
    pub kind: PayloadKind,
    /// Code-unit offset of the payload item.
    pub payload_offset: u32,
}

/// Semantic and structural facts retained for one instruction-stream item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnalyzedInstruction {
    /// Code-unit offset of the item.
    pub offset: u32,
    /// Register and exceptional behavior.
    pub semantics: InstructionSemantics,
    /// Producer immediately preceding this instruction's `move-result`, if any.
    pub result_producer: Option<u32>,
    /// `move-result` immediately consuming this instruction's result, if any.
    pub result_consumer: Option<u32>,
    /// Payload selected by this executable instruction, if any.
    pub payload: Option<PayloadLink>,
    /// Catch types entering this handler offset; `None` denotes catch-all.
    pub handler_types: Vec<Option<TypeIndex>>,
}

/// Validated method-body relationships used by later control/data-flow passes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BodyAnalysis {
    pub(super) stream_end: u32,
    pub(super) instructions: Vec<AnalyzedInstruction>,
    pub(super) positions: BTreeMap<u32, usize>,
    pub(super) payload_users: BTreeMap<u32, Vec<u32>>,
}

impl BodyAnalysis {
    /// Returns the exclusive end of the instruction stream in code units.
    #[must_use]
    pub const fn stream_end(&self) -> u32 {
        self.stream_end
    }

    /// Returns analyzed items in encoded order, including data payloads.
    #[must_use]
    pub fn instructions(&self) -> &[AnalyzedInstruction] {
        &self.instructions
    }

    /// Looks up an analyzed item by its exact code-unit offset.
    #[must_use]
    pub fn instruction(&self, offset: u32) -> Option<&AnalyzedInstruction> {
        self.positions
            .get(&offset)
            .map(|&position| &self.instructions[position])
    }

    /// Returns executable instructions that select the payload at `offset`.
    #[must_use]
    pub fn payload_users(&self, offset: u32) -> &[u32] {
        self.payload_users.get(&offset).map_or(&[], Vec::as_slice)
    }
}

/// Typed control-flow edge within one DEX method body.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FlowEdgeKind {
    /// Ordinary sequential execution.
    FallThrough,
    /// Direct conditional or unconditional branch.
    Branch,
    /// Dense or sparse switch case with its integer key.
    SwitchCase(i32),
    /// Exceptional transfer through an ordered typed or catch-all handler.
    Exception(Option<TypeIndex>),
}

/// One directed operation-to-operation edge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FlowEdge {
    /// Source operation offset in code units.
    pub source: u32,
    /// Target operation offset in code units.
    pub target: u32,
    /// Reason execution can take this edge.
    pub kind: FlowEdgeKind,
}

/// Exception-aware control flow over executable Dalvik operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ControlFlow {
    pub(super) entry: u32,
    pub(super) nodes: Vec<u32>,
    pub(super) edges: Vec<FlowEdge>,
}

impl ControlFlow {
    /// Returns the method-entry operation offset.
    #[must_use]
    pub const fn entry(&self) -> u32 {
        self.entry
    }

    /// Returns operation offsets in encoded order.
    #[must_use]
    pub fn nodes(&self) -> &[u32] {
        &self.nodes
    }

    /// Returns all typed directed edges in stable source order.
    #[must_use]
    pub fn edges(&self) -> &[FlowEdge] {
        &self.edges
    }

    /// Iterates over outgoing edges of one operation.
    pub fn successors(&self, source: u32) -> impl Iterator<Item = &FlowEdge> {
        self.edges.iter().filter(move |edge| edge.source == source)
    }

    /// Iterates over incoming edges of one operation.
    pub fn predecessors(&self, target: u32) -> impl Iterator<Item = &FlowEdge> {
        self.edges.iter().filter(move |edge| edge.target == target)
    }
}

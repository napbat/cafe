//! Exact consumer metadata carried by shared control-flow edges.

use cfglib::{Cfg, Edge, EdgeId, FilteredEdges};

use crate::{AddressRange, CatchType, CodeAddress, Instruction};

/// Stable position of an exception handler in the source artifact's ordered
/// handler table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ExceptionHandlerIndex(usize);

impl ExceptionHandlerIndex {
    /// Creates an index from its zero-based handler-table position.
    #[must_use]
    pub const fn from_index(index: usize) -> Self {
        Self(index)
    }

    /// Returns the zero-based handler-table position.
    #[must_use]
    pub const fn index(self) -> usize {
        self.0
    }
}

/// Exact semantic role of one shared control-flow edge.
///
/// cfglib's [`cfglib::EdgeKind`] remains the format-neutral structural class;
/// this role retains distinctions required by Java-ecosystem consumers.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ControlFlowEdgeRole {
    /// Ordinary execution into the next basic block.
    Sequential,
    /// Taken arm of a conditional branch.
    ConditionalTaken,
    /// Untaken arm of a conditional branch.
    ConditionalFallThrough,
    /// Direct unconditional branch.
    DirectBranch,
    /// Default arm of a switch.
    SwitchDefault,
    /// Keyed arm of a switch.
    SwitchCase {
        /// Exact signed switch key.
        key: i64,
    },
    /// Transfer into a legacy in-function subroutine.
    SubroutineCall,
    /// Normal continuation paired with one legacy subroutine call site.
    SubroutineContinuation {
        /// Address of the instruction that established this continuation.
        call_site: CodeAddress,
    },
    /// Exceptional transfer from one isolated throwing instruction.
    Exception {
        /// Ordered source handler-table identity.
        handler: ExceptionHandlerIndex,
        /// Exact protected range that selected the handler.
        protected: AddressRange,
        /// Catch-all or exact resolved catch type.
        catch: CatchType,
    },
}

impl ControlFlowEdgeRole {
    /// Returns whether this role represents exceptional rather than ordinary
    /// control flow.
    #[must_use]
    pub const fn is_exceptional(&self) -> bool {
        matches!(self, Self::Exception { .. })
    }
}

/// Exact source-level identity and semantics attached to a cfglib edge.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ControlFlowEdge {
    source: CodeAddress,
    target: CodeAddress,
    role: ControlFlowEdgeRole,
}

impl ControlFlowEdge {
    pub(crate) const fn new(
        source: CodeAddress,
        target: CodeAddress,
        role: ControlFlowEdgeRole,
    ) -> Self {
        Self {
            source,
            target,
            role,
        }
    }

    /// Returns the exact originating instruction address.
    ///
    /// For an exception edge this is the isolated potentially-throwing
    /// instruction, not merely the beginning of a larger protected block.
    #[must_use]
    pub const fn source(&self) -> CodeAddress {
        self.source
    }

    /// Returns the exact target instruction address.
    #[must_use]
    pub const fn target(&self) -> CodeAddress {
        self.target
    }

    /// Returns the detailed semantic role.
    #[must_use]
    pub const fn role(&self) -> &ControlFlowEdgeRole {
        &self.role
    }

    /// Returns whether this is an exceptional edge.
    #[must_use]
    pub const fn is_exceptional(&self) -> bool {
        self.role.is_exceptional()
    }
}

/// Zero-copy view of a shared CFG containing only ordinary control-flow edges.
pub type NormalControlFlow<'graph> = FilteredEdges<
    'graph,
    Cfg<Instruction, ControlFlowEdge>,
    fn(EdgeId, &Edge<ControlFlowEdge>) -> bool,
>;

pub(crate) fn is_normal_edge(_: EdgeId, edge: &Edge<ControlFlowEdge>) -> bool {
    !edge.payload().is_exceptional()
}

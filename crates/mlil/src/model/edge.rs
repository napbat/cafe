//! Typed MLIL control-flow edge metadata.

use disassembler::cfglib::EdgeKind;
use disassembler::{AddressRange, CatchType};

use super::InstructionId;

/// Exact semantic role of one stable MLIL edge.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum EdgeRole {
    /// Synthetic function entry into the first semantic block.
    Entry,
    /// Normal completion of a potentially throwing operation into its commit block.
    Commit,
    /// Ordinary sequential execution.
    FallThrough,
    /// Taken conditional arm.
    BranchTrue,
    /// Not-taken conditional arm.
    BranchFalse,
    /// Explicit unconditional jump.
    Jump,
    /// Default switch arm.
    SwitchDefault,
    /// Signed switch selector.
    SwitchCase(i64),
    /// Exceptional transfer through an ordered native handler.
    Exception {
        /// Resolved catch type or catch-all.
        catch: CatchType,
        /// Stable native table order within the method.
        handler_order: u32,
        /// Exact protected native address range.
        protected: AddressRange,
    },
}

impl EdgeRole {
    /// Returns whether this edge carries exceptional pre-state.
    #[must_use]
    pub const fn is_exception(&self) -> bool {
        matches!(self, Self::Exception { .. })
    }

    pub(crate) const fn cfglib_kind(&self) -> EdgeKind {
        match self {
            Self::Entry | Self::Commit | Self::FallThrough => EdgeKind::Fallthrough,
            Self::BranchTrue => EdgeKind::ConditionalTrue,
            Self::BranchFalse => EdgeKind::ConditionalFalse,
            Self::Jump => EdgeKind::Jump,
            Self::SwitchCase(_) => EdgeKind::SwitchCase,
            // The default arm is the dispatch's sequential fallback, which
            // is how cfglib's structuring recognizes an explicit default.
            Self::SwitchDefault => EdgeKind::Unconditional,
            Self::Exception { .. } => EdgeKind::ExceptionUnwind,
        }
    }
}

/// Caller-owned metadata stored on a cfglib edge.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct EdgeMetadata {
    /// Exact semantic edge role.
    pub role: EdgeRole,
    /// Potentially throwing instruction that generated an exception edge.
    pub throw_site: Option<InstructionId>,
}

impl EdgeMetadata {
    /// Creates ordinary edge metadata.
    #[must_use]
    pub const fn ordinary(role: EdgeRole) -> Self {
        Self {
            role,
            throw_site: None,
        }
    }

    /// Creates exceptional edge metadata tied to an exact MLIL instruction.
    #[must_use]
    pub const fn exceptional(role: EdgeRole, throw_site: InstructionId) -> Self {
        Self {
            role,
            throw_site: Some(throw_site),
        }
    }
}

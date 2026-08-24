//! Intraprocedural control-flow classification.

use super::{CodeAddress, SwitchCase};

/// Control-flow effect of one decoded instruction.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub enum InstructionFlow {
    /// Execution continues at the next instruction.
    #[default]
    FallThrough,
    /// The target is taken when a condition is true; otherwise execution falls through.
    ConditionalBranch {
        /// Absolute taken target.
        target: CodeAddress,
    },
    /// Execution always continues at the direct target.
    UnconditionalBranch {
        /// Absolute branch target.
        target: CodeAddress,
    },
    /// Multi-way integer dispatch with an explicit default target.
    Switch {
        /// Default branch target.
        default: CodeAddress,
        /// Keyed case targets.
        cases: Vec<SwitchCase>,
    },
    /// Execution returns from the current function.
    Return,
    /// Execution throws and has no ordinary successor.
    Throw,
    /// Target is computed at runtime and cannot be represented directly.
    IndirectBranch,
    /// Legacy in-function subroutine call with a direct entry and fallthrough return site.
    SubroutineCall {
        /// Subroutine entry address.
        target: CodeAddress,
    },
}

impl InstructionFlow {
    /// Returns whether an instruction following this one starts a basic block.
    #[must_use]
    pub const fn ends_basic_block(&self) -> bool {
        !matches!(self, Self::FallThrough)
    }
}

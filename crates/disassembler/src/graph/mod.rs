//! Control-flow graph construction over shared disassembly IR.

mod build;
mod error;

use std::collections::BTreeMap;

use cfglib::{BlockId, Cfg};

use crate::{CodeAddress, FunctionBody, Instruction};

pub use self::build::build_control_flow_graph;
pub use self::error::GraphError;

/// A verified cfglib CFG with source-address-to-block lookup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ControlFlowGraph {
    cfg: Cfg<Instruction>,
    instruction_blocks: BTreeMap<CodeAddress, BlockId>,
}

impl ControlFlowGraph {
    pub(crate) const fn new(
        cfg: Cfg<Instruction>,
        instruction_blocks: BTreeMap<CodeAddress, BlockId>,
    ) -> Self {
        Self {
            cfg,
            instruction_blocks,
        }
    }

    /// Returns the underlying graph for cfglib algorithms.
    #[must_use]
    pub const fn cfg(&self) -> &Cfg<Instruction> {
        &self.cfg
    }

    /// Consumes this wrapper and returns the underlying cfglib graph.
    #[must_use]
    pub fn into_cfg(self) -> Cfg<Instruction> {
        self.cfg
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

impl AsRef<Cfg<Instruction>> for ControlFlowGraph {
    fn as_ref(&self) -> &Cfg<Instruction> {
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

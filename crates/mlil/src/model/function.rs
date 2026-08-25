//! Verified MLIL function storage and derived analyses.

use disassembler::FunctionCoordinate;
use disassembler::cfglib::{
    AstNode, BlockExprTrees, Cfg, ConstFact, DefUseChains, DominatorTree, Facts, Liveness,
    ProgramPoint, SccpAnalysis, SsaForm,
};

use crate::{Result, VerificationReport, verify::verify_function};

use super::{
    Constant, EdgeMetadata, Instruction, InstructionId, ProvenanceMap, Variable, VariableId,
};

/// One verified semantic function backed by a cfglib control-flow graph.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Function {
    pub(crate) cfg: Cfg<Instruction, EdgeMetadata>,
    pub(crate) variables: Vec<Variable>,
    pub(crate) provenance: ProvenanceMap,
    pub(crate) instruction_points: Vec<ProgramPoint>,
}

impl Function {
    /// Returns the exact edge-bearing MLIL control-flow graph.
    #[must_use]
    pub const fn cfg(&self) -> &Cfg<Instruction, EdgeMetadata> {
        &self.cfg
    }

    /// Returns the source function coordinate system.
    #[must_use]
    pub fn source(&self) -> &FunctionCoordinate {
        self.provenance.source()
    }

    /// Returns variables in dense identity order.
    #[must_use]
    pub fn variables(&self) -> &[Variable] {
        &self.variables
    }

    /// Looks up one declared variable.
    #[must_use]
    pub fn variable(&self, id: VariableId) -> Option<&Variable> {
        self.variables.get(id.index())
    }

    /// Returns stable native-to-MLIL provenance.
    #[must_use]
    pub const fn provenance(&self) -> &ProvenanceMap {
        &self.provenance
    }

    /// Looks up one instruction by stable identity.
    #[must_use]
    pub fn instruction(&self, id: InstructionId) -> Option<&Instruction> {
        let point = *self.instruction_points.get(id.index())?;
        self.cfg
            .blocks()
            .get(point.block.index())?
            .instructions()
            .get(point.inst_idx)
    }

    /// Returns the current graph location of one stable instruction.
    #[must_use]
    pub fn instruction_point(&self, id: InstructionId) -> Option<ProgramPoint> {
        self.instruction_points.get(id.index()).copied()
    }

    /// Verifies structural, control-flow, typing, and provenance invariants.
    #[must_use]
    pub fn verify(&self) -> VerificationReport {
        verify_function(self)
    }

    /// Computes a dominator tree over ordinary and exceptional control flow.
    #[must_use]
    pub fn dominators(&self) -> DominatorTree {
        DominatorTree::compute(&self.cfg)
    }

    /// Computes a fully renamed SSA view while retaining the source graph.
    ///
    /// # Errors
    ///
    /// Returns a verification report if the stored function is invalid.
    pub fn ssa(&self) -> Result<SsaForm<VariableId>> {
        let report = self.verify();
        if !report.is_ok() {
            return Err(report.into());
        }
        let dominators = self.dominators();
        Ok(SsaForm::compute(&self.cfg, &dominators))
    }

    /// Computes definition-to-use and use-to-definition chains.
    #[must_use]
    pub fn def_use(&self) -> DefUseChains {
        DefUseChains::compute(&self.cfg)
    }

    /// Computes block-entry and block-exit liveness.
    #[must_use]
    pub fn liveness(&self) -> Liveness<VariableId> {
        Liveness::compute(&self.cfg)
    }

    /// Computes conservative forward constant facts without changing the function.
    #[must_use]
    pub fn constants(&self) -> Facts<ConstFact<VariableId, Constant>> {
        disassembler::cfglib::constant_propagation(&self.cfg)
    }

    /// Computes sparse constants over the derived SSA view.
    ///
    /// # Errors
    ///
    /// Returns a verification report if the stored function is invalid.
    pub fn sparse_constants(&self) -> Result<SccpAnalysis<VariableId, Constant>> {
        let ssa = self.ssa()?;
        Ok(SccpAnalysis::compute(&self.cfg, &ssa))
    }

    /// Recovers pure, single-use expression trees independently in each block.
    #[must_use]
    pub fn expressions(
        &self,
    ) -> Vec<BlockExprTrees<VariableId, crate::ExpressionOperator, Constant>> {
        disassembler::cfglib::recover_expressions(&self.cfg)
    }

    /// Recovers structured control flow, retaining explicit labels and gotos
    /// when the graph is irreducible.
    #[must_use]
    pub fn structured_control_flow(&self) -> AstNode<Instruction> {
        disassembler::cfglib::lift(&self.cfg)
    }

    /// Returns a copy-propagated graph for presentation and downstream analysis.
    ///
    /// The canonical function and its identity-indexed provenance are not
    /// mutated. Removed copies retain no graph position in the returned view.
    #[must_use]
    pub fn copy_propagated_cfg(
        &self,
    ) -> (
        Cfg<Instruction, EdgeMetadata>,
        disassembler::cfglib::CopyPropagationStats,
    ) {
        let mut cfg = self.cfg.clone();
        let statistics = disassembler::cfglib::copy_propagation(&mut cfg);
        (cfg, statistics)
    }

    /// Reports effect-aware dead definitions and unreachable blocks.
    #[must_use]
    pub fn dead_code(&self) -> disassembler::cfglib::DeadCode {
        disassembler::cfglib::DeadCode::compute(&self.cfg)
    }

    /// Returns a graph with effect-free dead definitions removed.
    ///
    /// Managed-memory reads and writes, allocation, calls, synchronization,
    /// throwing operations, and control flow remain observable and are never
    /// removed. The canonical function and its stable provenance are not
    /// mutated; instruction positions in the returned view are derived data.
    #[must_use]
    pub fn dead_code_eliminated_cfg(&self) -> (Cfg<Instruction, EdgeMetadata>, usize) {
        let mut cfg = self.cfg.clone();
        let removed = disassembler::cfglib::dead_code_elimination(&mut cfg);
        (cfg, removed)
    }
}

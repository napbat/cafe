//! Verified MLIL function storage and derived analyses.

use disassembler::FunctionCoordinate;
use disassembler::cfglib::{Cfg, DominatorTree, ProgramPoint, SsaForm};

use crate::{Result, VerificationReport, verify::verify_function};

use super::{EdgeMetadata, Instruction, InstructionId, ProvenanceMap, Variable, VariableId};

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
}

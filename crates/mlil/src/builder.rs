//! Checked construction of MLIL functions.

use disassembler::cfglib::{BlockId, Cfg, EdgeId, ProgramPoint};
use disassembler::{AddressRange, FunctionCoordinate};

use crate::model::{
    EdgeMetadata, EntityId, Function, Instruction, InstructionId, NativeVariable, Operation,
    ProvenanceMap, TypedVariable, Variable, VariableId, VariableRole,
};
use crate::{Error, Result};

/// Incremental builder that assigns dense stable MLIL identities.
pub struct FunctionBuilder {
    cfg: Cfg<Instruction, EdgeMetadata>,
    variables: Vec<Variable>,
    provenance: ProvenanceMap,
    instruction_points: Vec<ProgramPoint>,
}

impl FunctionBuilder {
    /// Creates a builder with an empty synthetic root block.
    #[must_use]
    pub fn new(source: FunctionCoordinate) -> Self {
        let mut cfg = Cfg::with_edge_payload();
        cfg.block_mut(cfg.entry()).set_label("root");
        Self {
            cfg,
            variables: Vec::new(),
            provenance: ProvenanceMap::new(source),
            instruction_points: Vec::new(),
        }
    }

    /// Returns the synthetic root block.
    #[must_use]
    pub fn entry(&self) -> BlockId {
        self.cfg.entry()
    }

    /// Allocates a semantic block with a diagnostic label.
    pub fn new_block(&mut self, label: impl Into<String>) -> BlockId {
        let block = self.cfg.new_block();
        self.cfg.block_mut(block).set_label(label);
        block
    }

    /// Declares one mutable pre-SSA variable.
    ///
    /// # Errors
    ///
    /// Returns an error if the function exceeds the compact identity space.
    pub fn declare_variable(
        &mut self,
        role: VariableRole,
        native: Option<NativeVariable>,
    ) -> Result<VariableId> {
        let raw = u32::try_from(self.variables.len()).map_err(|_| {
            Error::InvalidConstruction("variable count exceeds u32::MAX".to_owned())
        })?;
        let id = VariableId::from_raw(raw);
        self.variables.push(Variable { id, role, native });
        Ok(id)
    }

    /// Appends one typed semantic instruction to a block.
    ///
    /// `may_throw` records possible implicit exceptional transfer independently
    /// of whether the operation is an explicit `throw`.
    ///
    /// # Errors
    ///
    /// Returns an error for an invalid block, identity-space exhaustion, or an
    /// invalid native range.
    pub fn append_instruction(
        &mut self,
        block: BlockId,
        operation: Operation,
        uses: Vec<TypedVariable>,
        defs: Vec<TypedVariable>,
        may_throw: bool,
        source: Option<AddressRange>,
    ) -> Result<InstructionId> {
        self.require_block(block)?;
        let raw = u32::try_from(self.instruction_points.len()).map_err(|_| {
            Error::InvalidConstruction("instruction count exceeds u32::MAX".to_owned())
        })?;
        let id = InstructionId::from_raw(raw);
        let point = ProgramPoint {
            block,
            inst_idx: self.cfg.block(block).instructions().len(),
        };
        self.cfg
            .block_mut(block)
            .push(Instruction::new(id, operation, uses, defs, may_throw));
        self.instruction_points.push(point);
        if let Some(range) = source {
            self.provenance.insert(range, EntityId::Instruction(id))?;
        }
        Ok(id)
    }

    /// Adds one exact semantic edge.
    ///
    /// # Errors
    ///
    /// Returns an error for an invalid endpoint or native range.
    pub fn add_edge(
        &mut self,
        source: BlockId,
        target: BlockId,
        metadata: EdgeMetadata,
        native_range: Option<AddressRange>,
    ) -> Result<EdgeId> {
        self.require_block(source)?;
        self.require_block(target)?;
        let kind = metadata.role.cfglib_kind();
        let edge = self
            .cfg
            .add_edge_with_payload(source, target, kind, metadata);
        if let Some(range) = native_range {
            self.provenance.insert(range, EntityId::Edge(edge))?;
        }
        Ok(edge)
    }

    /// Records an additional many-to-many native correspondence.
    ///
    /// # Errors
    ///
    /// Returns an error when `source` is empty or reversed.
    pub fn map_entity(&mut self, source: AddressRange, entity: EntityId) -> Result<bool> {
        self.provenance.insert(source, entity)
    }

    /// Completes and strictly verifies the function.
    ///
    /// # Errors
    ///
    /// Returns every discovered invariant violation as one report.
    pub fn finish(self) -> Result<Function> {
        let function = Function {
            cfg: self.cfg,
            variables: self.variables,
            provenance: self.provenance,
            instruction_points: self.instruction_points,
        };
        let report = function.verify();
        if report.is_ok() {
            Ok(function)
        } else {
            Err(report.into())
        }
    }

    fn require_block(&self, block: BlockId) -> Result<()> {
        if block.index() < self.cfg.block_count() {
            Ok(())
        } else {
            Err(Error::InvalidConstruction(format!(
                "block {block} is outside a {}-block function",
                self.cfg.block_count()
            )))
        }
    }
}

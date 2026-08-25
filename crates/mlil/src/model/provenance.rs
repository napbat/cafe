//! Many-to-many provenance between native bytecode and MLIL entities.

use disassembler::cfglib::{BlockId, EdgeId};
use disassembler::{AddressRange, CodeAddress, FunctionCoordinate};

use crate::{Error, Result};

use super::{InstructionId, VariableId};

/// Stable identity of an MLIL entity that can originate from native code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum EntityId {
    /// Control-flow block.
    Block(BlockId),
    /// Stable control-flow edge.
    Edge(EdgeId),
    /// Semantic instruction.
    Instruction(InstructionId),
    /// Mutable pre-SSA variable.
    Variable(VariableId),
}

/// One native range mapped to one MLIL entity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ProvenanceEntry {
    /// Half-open source range in the native function coordinate system.
    pub source: AddressRange,
    /// Generated MLIL entity represented by the source range.
    pub entity: EntityId,
}

/// Deterministic many-to-many native-to-MLIL provenance.
///
/// Overlapping source ranges and multiple entities per range are intentional:
/// one native instruction can expand into several semantic instructions, and
/// one MLIL entity can represent fused native instructions. Synthetic MLIL
/// entities simply have no entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProvenanceMap {
    source: FunctionCoordinate,
    entries: Vec<ProvenanceEntry>,
}

impl ProvenanceMap {
    /// Creates an empty map for one native function.
    #[must_use]
    pub const fn new(source: FunctionCoordinate) -> Self {
        Self {
            source,
            entries: Vec::new(),
        }
    }

    /// Returns the native function coordinate system.
    #[must_use]
    pub const fn source(&self) -> &FunctionCoordinate {
        &self.source
    }

    /// Adds one mapping in deterministic range-then-entity order.
    ///
    /// Returns `true` for a new entry and `false` for an exact duplicate.
    ///
    /// # Errors
    ///
    /// Returns an error when `source` is empty or reversed.
    pub fn insert(&mut self, source: AddressRange, entity: EntityId) -> Result<bool> {
        if source.is_empty() {
            return Err(Error::InvalidProvenance(format!(
                "source range {}..{} is empty or reversed",
                source.start, source.end
            )));
        }
        let entry = ProvenanceEntry { source, entity };
        match self.entries.binary_search(&entry) {
            Ok(_) => Ok(false),
            Err(position) => {
                self.entries.insert(position, entry);
                Ok(true)
            }
        }
    }

    /// Returns all mappings in deterministic order.
    #[must_use]
    pub fn entries(&self) -> &[ProvenanceEntry] {
        &self.entries
    }

    /// Returns mappings whose native range contains `address`.
    pub fn mappings_from(&self, address: CodeAddress) -> impl Iterator<Item = &ProvenanceEntry> {
        self.entries
            .iter()
            .filter(move |entry| entry.source.contains(address))
    }

    /// Returns mappings that identify `entity`.
    pub fn mappings_to(&self, entity: EntityId) -> impl Iterator<Item = &ProvenanceEntry> {
        self.entries
            .iter()
            .filter(move |entry| entry.entity == entity)
    }

    /// Returns whether no native correspondence has been recorded.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Returns the number of distinct mappings.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.entries.len()
    }
}

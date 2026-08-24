//! Explicit bytecode-offset mappings used by metadata-aware rewrites.

use std::collections::BTreeMap;

use crate::bytecode::Instruction;
use crate::{Error, Result};

/// Mapping from offsets in an old method body to offsets in its replacement.
///
/// The map is deliberately explicit: inserted or removed instructions have no
/// universally correct metadata correspondence. Callers retain control over
/// those semantic choices while Cafe validates every referenced boundary.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BytecodeOffsetMap {
    offsets: BTreeMap<u16, u16>,
}

impl BytecodeOffsetMap {
    /// Creates an empty offset map.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            offsets: BTreeMap::new(),
        }
    }

    /// Builds a map by pairing instructions in order and mapping both code ends.
    ///
    /// # Errors
    ///
    /// Returns an error if the instruction counts differ or an offset does not
    /// fit the class-file `u16` offset representation.
    pub fn from_instruction_pairs(
        old: &[Instruction],
        old_code_length: usize,
        new: &[Instruction],
        new_code_length: usize,
    ) -> Result<Self> {
        if old.len() != new.len() {
            return Err(Error::invalid_assembly(format!(
                "cannot infer an offset map from {} old and {} new instructions",
                old.len(),
                new.len()
            )));
        }
        let mut map = Self::new();
        for (old, new) in old.iter().zip(new) {
            map.insert(to_u16(old.offset)?, to_u16(new.offset)?)?;
        }
        map.insert(to_u16(old_code_length)?, to_u16(new_code_length)?)?;
        Ok(map)
    }

    /// Adds one old-to-new boundary correspondence.
    ///
    /// Repeating the same correspondence is harmless.
    ///
    /// # Errors
    ///
    /// Returns an error if the old offset was already mapped differently.
    pub fn insert(&mut self, old: u16, new: u16) -> Result<()> {
        if let Some(existing) = self.offsets.insert(old, new)
            && existing != new
        {
            self.offsets.insert(old, existing);
            return Err(Error::invalid_assembly(format!(
                "bytecode offset {old} is already mapped to {existing}, not {new}"
            )));
        }
        Ok(())
    }

    /// Returns the replacement offset associated with an old offset.
    #[must_use]
    pub fn get(&self, old: u16) -> Option<u16> {
        self.offsets.get(&old).copied()
    }

    /// Returns the number of explicit correspondences.
    #[must_use]
    pub fn len(&self) -> usize {
        self.offsets.len()
    }

    /// Returns whether the map has no correspondences.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.offsets.is_empty()
    }

    pub(super) fn require(&self, old: u16, context: &str) -> Result<u16> {
        self.get(old).ok_or_else(|| {
            Error::invalid_assembly(format!(
                "offset map has no replacement for {context} bytecode offset {old}"
            ))
        })
    }
}

fn to_u16(offset: usize) -> Result<u16> {
    u16::try_from(offset).map_err(|_| {
        Error::invalid_assembly(format!(
            "bytecode offset {offset} exceeds the class-file u16 metadata range"
        ))
    })
}

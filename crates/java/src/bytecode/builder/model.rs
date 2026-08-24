//! Public identities and completed output for symbolic bytecode construction.

use std::sync::atomic::{AtomicU64, Ordering};

use crate::classfile::ExceptionHandler;
use crate::{Error, Result};

use disassembler::{AddressRange, CodeAddress};

use super::super::{Instruction, Opcode, Operand};

const FIRST_BUILDER_SCOPE: u64 = 1;
const BUILDER_SCOPE_INCREMENT: u64 = 1;

static NEXT_BUILDER_SCOPE: AtomicU64 = AtomicU64::new(FIRST_BUILDER_SCOPE);

pub(super) fn next_builder_scope() -> u64 {
    NEXT_BUILDER_SCOPE.fetch_add(BUILDER_SCOPE_INCREMENT, Ordering::Relaxed)
}

/// Builder-local symbolic bytecode position.
///
/// Labels may be used as branch, switch, and exception-region boundaries.
/// A label created by one builder is rejected by every other builder.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Label {
    pub(super) scope: u64,
    pub(super) index: usize,
}

/// Stable identity of one instruction requested from a [`CodeBuilder`](super::CodeBuilder).
///
/// Relaxation may expand one requested conditional branch into two encoded
/// instructions. Its identity continues to refer to the first instruction in
/// that expansion.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct InstructionId {
    pub(super) scope: u64,
    pub(super) index: usize,
}

/// JVM computational category used by ergonomic local load/store emission.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LocalKind {
    /// Integer-like local (`boolean`, `byte`, `char`, `short`, or `int`).
    Integer,
    /// Signed 64-bit integer local.
    Long,
    /// IEEE 754 single-precision local.
    Float,
    /// IEEE 754 double-precision local.
    Double,
    /// Object or array local.
    Reference,
}

/// Catch classification for a symbolic exception handler.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CatchTarget {
    /// Handler accepts every throwable value.
    Any,
    /// Handler accepts the class at this constant-pool index.
    Class(u16),
}

/// Completed JVM bytecode, resolved metadata, and stable layout lookup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuiltCode {
    pub(super) scope: u64,
    pub(super) code: Vec<u8>,
    pub(super) instructions: Vec<Instruction>,
    pub(super) instruction_offsets: Vec<usize>,
    pub(super) label_offsets: Vec<usize>,
    pub(super) exception_table: Vec<ExceptionHandler>,
}

impl BuiltCode {
    /// Returns the assembled method bytecode.
    #[must_use]
    pub fn code(&self) -> &[u8] {
        &self.code
    }

    /// Consumes this output and returns the assembled method bytecode.
    #[must_use]
    pub fn into_code(self) -> Vec<u8> {
        self.code
    }

    /// Returns every decoded output instruction, including relaxation helpers.
    #[must_use]
    pub fn instructions(&self) -> &[Instruction] {
        &self.instructions
    }

    /// Returns the byte offset assigned to a requested instruction.
    #[must_use]
    pub fn instruction_offset(&self, instruction: InstructionId) -> Option<usize> {
        (instruction.scope == self.scope)
            .then(|| self.instruction_offsets.get(instruction.index).copied())
            .flatten()
    }

    /// Returns the byte offset assigned to a bound label.
    #[must_use]
    pub fn label_offset(&self, label: Label) -> Option<usize> {
        (label.scope == self.scope)
            .then(|| self.label_offsets.get(label.index).copied())
            .flatten()
    }

    /// Resolves a half-open bytecode range between two bound labels.
    ///
    /// This is the provenance adapter for symbolic generation: callers may
    /// place the returned range into a format-neutral
    /// [`disassembler::SourceMap`] after final branch layout is known.
    ///
    /// # Errors
    ///
    /// Returns an error for a foreign label, a reversed or empty range, or an
    /// address that cannot be represented by the shared address type.
    pub fn label_range(&self, start: Label, end: Label) -> Result<AddressRange> {
        let start = self.label_offset(start).ok_or_else(|| {
            Error::invalid_assembly("source-map start label does not belong to this built method")
        })?;
        let end = self.label_offset(end).ok_or_else(|| {
            Error::invalid_assembly("source-map end label does not belong to this built method")
        })?;
        if start >= end {
            return Err(Error::invalid_assembly(format!(
                "source-map bytecode range {start}..{end} is empty or reversed"
            )));
        }
        Ok(AddressRange::new(
            CodeAddress::new(
                u64::try_from(start)
                    .map_err(|_| Error::invalid_assembly("source-map start offset exceeds u64"))?,
            ),
            CodeAddress::new(
                u64::try_from(end)
                    .map_err(|_| Error::invalid_assembly("source-map end offset exceeds u64"))?,
            ),
        ))
    }

    /// Returns the exception table resolved from symbolic region labels.
    #[must_use]
    pub fn exception_table(&self) -> &[ExceptionHandler] {
        &self.exception_table
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct PendingInstruction {
    pub(super) kind: PendingInstructionKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum PendingInstructionKind {
    Plain {
        opcode: Opcode,
        operand: Operand,
    },
    Branch {
        opcode: Opcode,
        target: Label,
        form: BranchForm,
    },
    TableSwitch {
        default: Label,
        low: i32,
        targets: Vec<Label>,
    },
    LookupSwitch {
        default: Label,
        pairs: Vec<(i32, Label)>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum BranchForm {
    Short,
    Wide,
    ExpandedConditional,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct PendingExceptionHandler {
    pub(super) start: Label,
    pub(super) end: Label,
    pub(super) handler: Label,
    pub(super) catch: CatchTarget,
}

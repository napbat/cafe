//! Errors produced while constructing a control-flow graph.

use crate::CodeAddress;

/// Invalid disassembly IR that prevents exact basic-block construction.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum GraphError {
    /// An instruction cannot occupy zero address units.
    #[error("instruction at {address} has zero encoded size")]
    ZeroInstructionSize {
        /// Invalid instruction address.
        address: CodeAddress,
    },
    /// Adding an instruction's address and size overflowed the address type.
    #[error("instruction at {address} overflows the code address space")]
    AddressOverflow {
        /// Instruction whose end address overflowed.
        address: CodeAddress,
    },
    /// Instruction ranges overlap or are not in increasing order.
    #[error("instruction at {address} overlaps the previous instruction ending at {previous_end}")]
    OverlappingInstruction {
        /// Address of the later invalid instruction.
        address: CodeAddress,
        /// Exclusive end of the previous instruction.
        previous_end: CodeAddress,
    },
    /// A direct control-flow target does not name an instruction boundary.
    #[error("instruction at {source_address} targets missing instruction boundary {target}")]
    MissingBranchTarget {
        /// Instruction containing the invalid target.
        source_address: CodeAddress,
        /// Target that is not an instruction boundary.
        target: CodeAddress,
    },
    /// An exception handler protects an empty or reversed range.
    #[error("invalid exception range {start}..{end}")]
    InvalidExceptionRange {
        /// Inclusive protected start.
        start: CodeAddress,
        /// Exclusive protected end.
        end: CodeAddress,
    },
    /// The beginning of an exception range is not an instruction boundary.
    #[error("exception range starts at missing instruction boundary {address}")]
    MissingExceptionStart {
        /// Invalid protected-range start.
        address: CodeAddress,
    },
    /// The end of an exception range is neither an instruction nor code-end boundary.
    #[error("exception range ends at missing code boundary {address}")]
    MissingExceptionEnd {
        /// Invalid protected-range end.
        address: CodeAddress,
    },
    /// An exception handler target is not an instruction boundary.
    #[error("exception handler targets missing instruction boundary {address}")]
    MissingExceptionHandler {
        /// Invalid handler target.
        address: CodeAddress,
    },
    /// cfglib rejected a graph constructed from otherwise validated input.
    #[error("cfglib rejected the constructed graph: {details}")]
    InvalidGraph {
        /// Combined cfglib verification messages.
        details: String,
    },
}

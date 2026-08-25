//! Adapter contract implemented by concrete bytecode-format crates.

use crate::Disassembly;

/// A decoded format-specific value that can lift into shared disassembly IR.
pub trait DisassemblySource {
    /// Error produced while resolving or lifting the source representation.
    type Error;

    /// Lifts this source into the format-neutral disassembly boundary.
    ///
    /// # Errors
    ///
    /// Returns the source adapter's error when native instructions, symbols,
    /// or exception metadata cannot be decoded or resolved.
    fn disassemble(&self) -> Result<Disassembly, Self::Error>;
}

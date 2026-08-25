//! Program adapter contract implemented by concrete format crates.

use crate::Module;

/// A decoded format-specific value that can produce a shared program module.
pub trait ModuleSource {
    /// Error produced while resolving or lifting the native representation.
    type Error;

    /// Builds an owned module with native metadata and disassembled bodies.
    ///
    /// # Errors
    ///
    /// Returns the source adapter's error when native definitions, symbols,
    /// instructions, or metadata cannot be decoded or resolved.
    fn to_module(&self) -> Result<Module, Self::Error>;
}

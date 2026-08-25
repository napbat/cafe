//! Neutral contract for format crates that emit owned modules.

use crate::Module;

/// Format-specific backend that emits one owned Program module.
///
/// The Program crate owns only this direction-neutral contract. JVM and DEX
/// crates implement it for their native output models, keeping native tables,
/// validation, and assembly errors inside the corresponding frontend.
pub trait ModuleEmitter {
    /// Native artifact produced for one module.
    type Output;
    /// Format-specific emission failure.
    type Error;

    /// Emits and validates one module.
    ///
    /// # Errors
    ///
    /// Returns a format-specific error when the module has the wrong format or
    /// contains a value the selected backend cannot represent.
    fn emit_module(&mut self, module: &Module) -> Result<Self::Output, Self::Error>;
}

impl Module {
    /// Emits this module through a format-specific backend.
    ///
    /// # Errors
    ///
    /// Returns the backend's native emission error.
    pub fn emit_with<E: ModuleEmitter>(&self, emitter: &mut E) -> Result<E::Output, E::Error> {
        emitter.emit_module(self)
    }
}

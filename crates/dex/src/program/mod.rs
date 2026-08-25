//! DEX adapter for the shared owned program model.

mod adapter;
mod emission;

pub use self::adapter::{
    MethodBodyMode, ProgramOptions, lift_file, lift_file_named, lift_file_named_with_options,
    lift_file_with_options,
};
pub use self::emission::{
    DexEmissionError, DexEmissionOptions, DexEmitter, DexReferenceHandle,
    DexReferenceResolutionError, DexReferenceResolver, SymbolicDexReferenceResolver, emit_module,
};

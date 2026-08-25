//! JVM adapter for the shared owned program model.

mod adapter;
mod emission;

pub use self::adapter::{MethodBodyMode, ProgramOptions, lift_class, lift_class_with_options};
pub use self::emission::{
    DisplayJavaReferenceResolver, JavaEmissionError, JavaEmissionOptions, JavaEmitter,
    JavaReferenceResolutionError, JavaReferenceResolver, emit_module,
};

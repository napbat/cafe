//! JVM adapter for the shared owned program model.

mod adapter;
mod emission;

pub use self::adapter::{MethodBodyMode, ProgramOptions, lower_class, lower_class_with_options};
pub use self::emission::{
    DisplayJavaReferenceResolver, JavaEmissionError, JavaEmissionOptions, JavaEmitter,
    JavaReferenceResolutionError, JavaReferenceResolver, emit_module,
};

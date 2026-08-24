//! JVM adapter for the shared owned program model.

mod adapter;

pub use self::adapter::{MethodBodyMode, ProgramOptions, lower_class, lower_class_with_options};

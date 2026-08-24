//! JVM adapter for the shared Cafe object model.

mod adapter;

pub use self::adapter::{CafeOptions, MethodBodyMode, lower_class, lower_class_with_options};

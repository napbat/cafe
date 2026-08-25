//! Adapter from JVM class files to the shared cross-format disassembly model.

mod adapter;
mod instruction;

pub use self::adapter::{lift_class, lift_method};

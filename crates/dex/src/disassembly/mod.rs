//! Adapter from DEX definitions to the shared cross-format disassembly model.

mod adapter;
mod instruction;

pub(crate) use self::adapter::lift_body;
pub use self::adapter::{lift_file, lift_file_named, lift_method};

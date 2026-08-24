//! Adapter from DEX definitions to the shared cross-format disassembly model.

mod adapter;
mod instruction;

pub(crate) use self::adapter::lower_body;
pub use self::adapter::{lower_file, lower_file_named};

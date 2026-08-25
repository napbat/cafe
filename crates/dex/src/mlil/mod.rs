//! Dalvik LLIL to shared MLIL lifting.
//!
//! The adapter maps explicit Dalvik registers and implicit result/exception
//! channels into MLIL variables, elides non-executable payload nodes while
//! retaining their provenance, and preserves exceptional register pre-state
//! through normal-only commit blocks.

mod error;
mod instruction;
mod lift;
mod reference;
mod state;

pub use self::error::{Error, Result};
pub use self::lift::{
    lift_body, lift_body_with_hierarchy, lift_method, lift_method_with_hierarchy,
};

#[cfg(test)]
mod tests;

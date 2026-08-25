//! JVM LLIL to shared MLIL lifting.
//!
//! The adapter uses verified JVM entry/exit frames for every instruction,
//! preserves native stack/local identities, and splits potentially throwing
//! definitions through normal-only commit blocks so handler edges observe the
//! exact JVM pre-state.

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

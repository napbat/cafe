//! Bidirectional JVM LLIL and shared MLIL adaptation.
//!
//! The adapter uses verified JVM entry/exit frames for every instruction,
//! preserves native stack/local identities, and splits potentially throwing
//! definitions through normal-only commit blocks so handler edges observe the
//! exact JVM pre-state. Its lowering path allocates semantic variables to JVM
//! locals, schedules operand-stack operations, lays out symbolic control flow,
//! and rebuilds ordered exception ranges into independently verified LLIL.

mod error;
mod instruction;
mod lift;
mod lower;
mod reference;
mod state;

pub use self::error::{Error, Result};
pub use self::lift::{
    lift_body, lift_body_with_hierarchy, lift_method, lift_method_with_hierarchy,
};
pub use self::lower::{
    LoweredBody, SourceJavaReferenceResolver, lower_body, lower_body_from_source,
    lower_body_with_resolver,
};

#[cfg(test)]
mod tests;

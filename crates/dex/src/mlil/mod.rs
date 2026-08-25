//! Bidirectional Dalvik LLIL and shared MLIL adaptation.
//!
//! The adapter maps explicit Dalvik registers and implicit result/exception
//! channels into MLIL variables, elides non-executable payload nodes while
//! retaining their provenance, and preserves exceptional register pre-state
//! through normal-only commit blocks. Its lowering path allocates a fresh
//! register frame, schedules range operands, lays out branches and payloads,
//! and rebuilds ordered try regions into independently verified LLIL.

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
    DexIntrinsicInstruction, DexIntrinsicLoweringError, DexIntrinsicRequest,
    DexMlilIntrinsicLowerer, DexMlilReferenceResolver, LoweredBody, RejectDexIntrinsics,
    SourceDexReferenceResolver, TargetDexReferenceResolver, lower_body, lower_body_from_source,
    lower_body_with_resolver, lower_body_with_resolver_and_intrinsics,
};

#[cfg(test)]
mod tests;

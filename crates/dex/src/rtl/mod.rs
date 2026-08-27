//! Dalvik register-transfer adaptation between DEX LLIL and canonical MLIL.
//!
//! Dalvik RTL retains explicit virtual-register and implicit result/exception
//! storage while canonical Java MLIL remains independent of DEX encoding.
//! Raising and lowering use cfglib's checked web, provenance, signature,
//! region, and exact-edge translations.

mod adapter;
mod dialect;
mod lift;
mod placement;

pub use self::adapter::{
    lift_body, lift_body_with_hierarchy, lift_method, lift_method_with_hierarchy, lower_body,
    lower_function, raise_function, raise_function_with_hierarchy,
};
pub use self::dialect::{
    DexRtlDialect, DexStorage, EdgeMetadata, Function, Lowered, RegisterConstraint,
};

#[cfg(test)]
mod tests;

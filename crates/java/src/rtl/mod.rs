//! JVM register-transfer adaptation between JVM LLIL and canonical MLIL.
//!
//! JVM RTL models local slots and operand-stack positions as target-owned
//! storage while canonical Java MLIL remains independent of the JVM stack
//! machine. Raising and lowering use cfglib's checked web, provenance,
//! signature, region, and exact-edge translations.

mod adapter;
mod dialect;
mod lift;
mod placement;

pub use self::adapter::{
    lift_body, lift_body_with_hierarchy, lift_method, lift_method_with_hierarchy, lower_body,
    lower_function, raise_function, raise_function_with_hierarchy,
};
pub use self::dialect::{
    EdgeMetadata, Function, JvmConstraint, JvmRtlDialect, JvmStorage, Lowered,
};

#[cfg(test)]
mod tests;

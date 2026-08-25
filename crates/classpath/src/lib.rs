//! Unified classpath declarations and hierarchy queries for JVM and DEX analysis.
//!
//! [`ClasspathHierarchy`] normalizes JVM internal names and DEX object
//! descriptors into one declaration world. Equivalent definitions merge,
//! conflicting definitions are diagnosed, and explicit [`JvmHierarchyView`]
//! and [`DexHierarchyView`] adapters feed the native verification analyses.

mod build;
mod error;
mod hierarchy;
mod model;

pub use self::error::{Error, Result};
pub use self::hierarchy::{ClasspathHierarchy, DexHierarchyView, JvmHierarchyView};
pub use self::model::{ClassDeclaration, ClassDescriptor, DirectParents};

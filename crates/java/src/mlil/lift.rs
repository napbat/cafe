//! Canonical JVM LLIL to RTL to MLIL lifting entry points.

use mlil::Function;

use crate::analysis::{ClassHierarchy, ReferenceHierarchy};
use crate::classfile::{ClassFile, ConstantPool, MethodAccessFlags, MethodInfo};
use crate::llil;

use super::Result;

/// Lifts one class method through JVM RTL into shared MLIL when it has code.
///
/// # Errors
///
/// Returns an error for malformed metadata or bytecode, unsupported legacy
/// subroutines, failed frame/RTL analysis, or invalid generated MLIL.
pub fn lift_method(class: &ClassFile, method: &MethodInfo) -> Result<Option<Function>> {
    let hierarchy = ClassHierarchy::from_classes([class])?;
    lift_method_with_hierarchy(class, method, &hierarchy)
}

/// Lifts one class method through JVM RTL using caller-supplied relationships.
///
/// # Errors
///
/// Returns an error for malformed metadata or bytecode, unsupported legacy
/// subroutines, failed frame/RTL analysis, or invalid generated MLIL.
pub fn lift_method_with_hierarchy(
    class: &ClassFile,
    method: &MethodInfo,
    hierarchy: &dyn ReferenceHierarchy,
) -> Result<Option<Function>> {
    crate::rtl::lift_method_with_hierarchy(class, method, hierarchy)?
        .map(|function| crate::rtl::raise_function_with_hierarchy(&function, hierarchy))
        .transpose()
}

/// Lifts a standalone verified JVM LLIL body through JVM RTL into shared MLIL.
///
/// # Errors
///
/// Returns an error for invalid descriptors, constant references, LLIL/native
/// disagreement, frame analysis failures, invalid RTL, or invalid MLIL.
pub fn lift_body(
    pool: &ConstantPool,
    owner: &str,
    name: &str,
    descriptor: &str,
    access_flags: MethodAccessFlags,
    body: &llil::Body,
) -> Result<Function> {
    let hierarchy = ClassHierarchy::new();
    lift_body_with_hierarchy(
        pool,
        owner,
        name,
        descriptor,
        access_flags,
        body,
        &hierarchy,
    )
}

/// Lifts a standalone JVM LLIL body through RTL with caller relationships.
///
/// # Errors
///
/// Returns an error for invalid descriptors, constant references, LLIL/native
/// disagreement, frame analysis failures, invalid RTL, or invalid MLIL.
pub fn lift_body_with_hierarchy(
    pool: &ConstantPool,
    owner: &str,
    name: &str,
    descriptor: &str,
    access_flags: MethodAccessFlags,
    body: &llil::Body,
    hierarchy: &dyn ReferenceHierarchy,
) -> Result<Function> {
    let function = crate::rtl::lift_body_with_hierarchy(
        pool,
        owner,
        name,
        descriptor,
        access_flags,
        body,
        hierarchy,
    )?;
    crate::rtl::raise_function_with_hierarchy(&function, hierarchy)
}

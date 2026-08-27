//! Canonical Dalvik LLIL to RTL to MLIL lifting entry points.

use mlil::Function;

use crate::analysis::{DexHierarchy, ReferenceHierarchy};
use crate::file::{DexFile, EncodedMethod};
use crate::llil;

use super::Result;

/// Lifts one encoded method through Dalvik RTL when it has executable code.
///
/// # Errors
///
/// Returns an error for invalid identifiers, body relationships, register
/// states, payloads, exception metadata, invalid RTL, or invalid MLIL.
pub fn lift_method(file: &DexFile, declaration: &EncodedMethod) -> Result<Option<Function>> {
    let hierarchy = DexHierarchy::from_file(file)?;
    lift_method_with_hierarchy(file, declaration, &hierarchy)
}

/// Lifts one encoded method through Dalvik RTL with caller relationships.
///
/// # Errors
///
/// Returns an error for invalid identifiers, body relationships, register
/// states, payloads, exception metadata, invalid RTL, or invalid MLIL.
pub fn lift_method_with_hierarchy(
    file: &DexFile,
    declaration: &EncodedMethod,
    hierarchy: &dyn ReferenceHierarchy,
) -> Result<Option<Function>> {
    crate::rtl::lift_method_with_hierarchy(file, declaration, hierarchy)?
        .map(|function| crate::rtl::raise_function_with_hierarchy(&function, hierarchy))
        .transpose()
}

/// Lifts an edited Dalvik LLIL body through Dalvik RTL into shared MLIL.
///
/// # Errors
///
/// Returns an error for LLIL/native disagreement, invalid identifiers or
/// register states, invalid RTL, or invalid generated MLIL.
pub fn lift_body(
    file: &DexFile,
    declaration: &EncodedMethod,
    body: &llil::Body,
) -> Result<Function> {
    let hierarchy = DexHierarchy::from_file(file)?;
    lift_body_with_hierarchy(file, declaration, body, &hierarchy)
}

/// Lifts an edited Dalvik LLIL body through RTL with caller relationships.
///
/// # Errors
///
/// Returns an error for LLIL/native disagreement, invalid identifiers or
/// register states, invalid RTL, or invalid generated MLIL.
pub fn lift_body_with_hierarchy(
    file: &DexFile,
    declaration: &EncodedMethod,
    body: &llil::Body,
    hierarchy: &dyn ReferenceHierarchy,
) -> Result<Function> {
    let function = crate::rtl::lift_body_with_hierarchy(file, declaration, body, hierarchy)?;
    crate::rtl::raise_function_with_hierarchy(&function, hierarchy)
}

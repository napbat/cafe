//! Public Dalvik LLIL/RTL/MLIL conversion entry points.

use std::collections::BTreeSet;

use cfglib::ir::rtl::Statement;

use crate::analysis::{DexHierarchy, ReferenceHierarchy};
use crate::file::{DexFile, EncodedMethod};
use crate::llil;
use crate::mlil::{self as dex_mlil, Result};

use super::{DexRtlDialect, Function, Lowered};

/// Lowers canonical Java MLIL into checked Dalvik RTL.
///
/// # Errors
///
/// Returns an error when target placement, instruction selection, edge
/// identity mapping, signature transfer, or region transfer fails.
pub fn lower_function(function: &mlil::Function) -> Result<Lowered> {
    Ok(cfglib::ir::rtl::lower::<DexRtlDialect>(function)?)
}

/// Raises Dalvik RTL into canonical Java MLIL using an open hierarchy.
///
/// # Errors
///
/// Returns an error for incompatible web constraints or invalid generated
/// semantic control flow.
pub fn raise_function(function: &Function) -> Result<mlil::Function> {
    let hierarchy = DexHierarchy::new();
    raise_function_with_hierarchy(function, &hierarchy)
}

/// Raises Dalvik RTL into canonical Java MLIL with caller-supplied hierarchy
/// relationships.
///
/// # Errors
///
/// Returns an error for incompatible web constraints or invalid generated
/// semantic control flow.
pub fn raise_function_with_hierarchy(
    function: &Function,
    hierarchy: &dyn ReferenceHierarchy,
) -> Result<mlil::Function> {
    let context = reference_merge_context(function, hierarchy);
    Ok(cfglib::ir::rtl::lift(function, &context)?
        .builder
        .finish()?)
}

/// Lifts verified Dalvik LLIL into checked Dalvik RTL.
///
/// The register-analysis-aware Dalvik decoder resolves references, payloads,
/// exception tables, and exact native provenance directly into target RTL.
///
/// # Errors
///
/// Returns the same failures as [`dex_mlil::lift_body`] plus RTL placement
/// failures.
pub fn lift_body(
    file: &DexFile,
    declaration: &EncodedMethod,
    body: &llil::Body,
) -> Result<Function> {
    let hierarchy = DexHierarchy::from_file(file)?;
    lift_body_with_hierarchy(file, declaration, body, &hierarchy)
}

/// Lifts verified Dalvik LLIL into Dalvik RTL with caller-supplied hierarchy
/// relationships.
///
/// # Errors
///
/// Returns the same failures as [`dex_mlil::lift_body_with_hierarchy`] plus
/// RTL placement failures.
pub fn lift_body_with_hierarchy(
    file: &DexFile,
    declaration: &EncodedMethod,
    body: &llil::Body,
    hierarchy: &dyn ReferenceHierarchy,
) -> Result<Function> {
    super::lift::lift_body_with_hierarchy(file, declaration, body, hierarchy)
}

/// Lifts one executable encoded method into Dalvik RTL.
///
/// # Errors
///
/// Returns an error for malformed identifiers, instructions, analysis, or
/// RTL construction.
pub fn lift_method(file: &DexFile, declaration: &EncodedMethod) -> Result<Option<Function>> {
    let hierarchy = DexHierarchy::from_file(file)?;
    lift_method_with_hierarchy(file, declaration, &hierarchy)
}

/// Lifts one executable encoded method into Dalvik RTL with caller-supplied
/// hierarchy relationships.
///
/// # Errors
///
/// Returns an error for malformed identifiers, instructions, analysis, or
/// RTL construction.
pub fn lift_method_with_hierarchy(
    file: &DexFile,
    declaration: &EncodedMethod,
    hierarchy: &dyn ReferenceHierarchy,
) -> Result<Option<Function>> {
    super::lift::lift_method_with_hierarchy(file, declaration, hierarchy)
}

/// Lowers Dalvik RTL through canonical MLIL into freshly encoded Dalvik LLIL.
///
/// # Errors
///
/// Returns an error when raising fails or the semantic function has no
/// canonical Dalvik encoding.
pub fn lower_body(file: &DexFile, function: &Function) -> Result<dex_mlil::LoweredBody> {
    let semantic = raise_function(function)?;
    dex_mlil::lower_body(file, &semantic)
}

fn reference_merge_context(
    function: &Function,
    hierarchy: &dyn ReferenceHierarchy,
) -> mlil::rtl::ReferenceMergeContext {
    let mut descriptors = BTreeSet::new();
    for block in function.cfg().blocks() {
        for node in block.instructions() {
            node.statement().for_each_read(&mut |_, _, constraint| {
                remember_descriptor(constraint.value_type(), &mut descriptors);
            });
            if let Statement::Transfer { assignments, .. } = node.statement() {
                for (_, value) in assignments {
                    remember_descriptor(value.shape().scalar.value_type(), &mut descriptors);
                }
            }
        }
    }
    mlil::rtl::ReferenceMergeContext::from_descriptors(descriptors, |left, right| {
        hierarchy.common_supertype(left, right)
    })
}

fn remember_descriptor(value_type: &mlil::ValueType, descriptors: &mut BTreeSet<String>) {
    match value_type {
        mlil::ValueType::Reference(Some(descriptor))
        | mlil::ValueType::UninitializedThis(descriptor)
        | mlil::ValueType::Uninitialized { descriptor, .. } => {
            descriptors.insert(descriptor.clone());
        }
        _ => {}
    }
}

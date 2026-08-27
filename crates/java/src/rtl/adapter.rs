//! Public JVM LLIL/RTL/MLIL conversion entry points.

use std::collections::BTreeSet;

use cfglib::ir::rtl::Statement;

use crate::analysis::{ClassHierarchy, ReferenceHierarchy};
use crate::classfile::{ClassFile, ConstantPool, MethodAccessFlags, MethodInfo};
use crate::llil;
use crate::mlil::{self as jvm_mlil, Result};

use super::{Function, JvmRtlDialect, Lowered};

/// Lowers canonical Java MLIL into checked JVM RTL.
///
/// # Errors
///
/// Returns an error when target placement, instruction selection, edge
/// identity mapping, signature transfer, or region transfer fails.
pub fn lower_function(function: &mlil::Function) -> Result<Lowered> {
    Ok(cfglib::ir::rtl::lower::<JvmRtlDialect>(function)?)
}

/// Raises JVM RTL into canonical Java MLIL using an open hierarchy.
///
/// # Errors
///
/// Returns an error for incompatible web constraints or invalid generated
/// semantic control flow.
pub fn raise_function(function: &Function) -> Result<mlil::Function> {
    let hierarchy = ClassHierarchy::new();
    raise_function_with_hierarchy(function, &hierarchy)
}

/// Raises JVM RTL into canonical Java MLIL with caller-supplied hierarchy
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

/// Lifts verified JVM LLIL into checked JVM RTL.
///
/// The verifier-aware JVM decoder resolves constants, frames, exception
/// tables, and exact native provenance directly into target storage RTL.
///
/// # Errors
///
/// Returns the same failures as [`jvm_mlil::lift_body`] plus RTL placement
/// failures.
pub fn lift_body(
    pool: &ConstantPool,
    owner: &str,
    name: &str,
    descriptor: &str,
    access_flags: MethodAccessFlags,
    body: &llil::Body,
) -> Result<Function> {
    super::lift::lift_body(pool, owner, name, descriptor, access_flags, body)
}

/// Lifts verified JVM LLIL into JVM RTL with caller-supplied hierarchy
/// relationships.
///
/// # Errors
///
/// Returns the same failures as [`jvm_mlil::lift_body_with_hierarchy`] plus
/// RTL placement failures.
pub fn lift_body_with_hierarchy(
    pool: &ConstantPool,
    owner: &str,
    name: &str,
    descriptor: &str,
    access_flags: MethodAccessFlags,
    body: &llil::Body,
    hierarchy: &dyn ReferenceHierarchy,
) -> Result<Function> {
    super::lift::lift_body_with_hierarchy(
        pool,
        owner,
        name,
        descriptor,
        access_flags,
        body,
        hierarchy,
    )
}

/// Lifts one executable class method into JVM RTL.
///
/// # Errors
///
/// Returns an error for malformed method metadata, bytecode, analysis, or
/// RTL construction.
pub fn lift_method(class: &ClassFile, method: &MethodInfo) -> Result<Option<Function>> {
    let hierarchy = ClassHierarchy::from_classes([class])?;
    lift_method_with_hierarchy(class, method, &hierarchy)
}

/// Lifts one executable class method into JVM RTL with caller-supplied
/// hierarchy relationships.
///
/// # Errors
///
/// Returns an error for malformed method metadata, bytecode, analysis, or
/// RTL construction.
pub fn lift_method_with_hierarchy(
    class: &ClassFile,
    method: &MethodInfo,
    hierarchy: &dyn ReferenceHierarchy,
) -> Result<Option<Function>> {
    let Some(code) = method.code() else {
        return Ok(None);
    };
    let owner = class.class_name()?;
    let name = method.name(&class.constant_pool)?;
    let descriptor = method.descriptor(&class.constant_pool)?;
    super::lift::lift_code_with_hierarchy(
        &class.constant_pool,
        owner,
        name,
        descriptor,
        method.access_flags,
        code,
        hierarchy,
    )
    .map(Some)
}

/// Lowers JVM RTL through canonical MLIL into freshly encoded JVM LLIL.
///
/// # Errors
///
/// Returns an error when raising fails or the semantic function has no
/// canonical JVM encoding.
pub fn lower_body(function: &Function, pool: &mut ConstantPool) -> Result<jvm_mlil::LoweredBody> {
    let semantic = raise_function(function)?;
    jvm_mlil::lower_body(&semantic, pool)
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

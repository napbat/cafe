//! Symbolic shared-reference interning for canonical DEX output.

use disassembler::{Reference, ReferenceKind, ReferenceSymbol};

use super::descriptor::{field_type_valid, method_parts};
use super::{DexReferenceHandle, DexReferenceResolutionError, DexReferenceResolver};
use crate::file::DexBuilder;

/// Interns structured Cafe symbols, falling back to unambiguous DEX displays.
#[derive(Debug, Clone, Copy, Default)]
pub struct SymbolicDexReferenceResolver;

impl DexReferenceResolver for SymbolicDexReferenceResolver {
    fn intern(
        &mut self,
        reference: &Reference,
        builder: &mut DexBuilder,
    ) -> Result<DexReferenceHandle, DexReferenceResolutionError> {
        if let Some(symbol) = &reference.symbol {
            return intern_symbol(reference.kind, symbol, builder);
        }
        let display = reference
            .display
            .as_deref()
            .ok_or_else(|| failure("reference has no resolved symbolic value"))?;
        match reference.kind {
            ReferenceKind::String => builder
                .intern_string(display)
                .map(DexReferenceHandle::String)
                .map_err(native),
            ReferenceKind::Type => builder
                .intern_type(display)
                .map(DexReferenceHandle::Type)
                .map_err(native),
            ReferenceKind::Field => {
                let (owner, name, descriptor) = field(display)?;
                intern_field(builder, owner, name, descriptor)
            }
            ReferenceKind::Method | ReferenceKind::InterfaceMethod => {
                let (owner, name, descriptor) = method(display)?;
                intern_method(builder, owner, name, descriptor)
            }
            ReferenceKind::MethodPrototype => intern_prototype(builder, display),
            ReferenceKind::Constant
            | ReferenceKind::MethodHandle
            | ReferenceKind::DynamicCallSite => Err(failure(
                "reference kind needs recursive DEX metadata not retained by its display",
            )),
        }
    }
}

fn intern_symbol(
    kind: ReferenceKind,
    symbol: &ReferenceSymbol,
    builder: &mut DexBuilder,
) -> Result<DexReferenceHandle, DexReferenceResolutionError> {
    match symbol {
        ReferenceSymbol::String(value) => builder
            .intern_utf16(value.utf16_units.clone())
            .map(DexReferenceHandle::String)
            .map_err(native),
        ReferenceSymbol::Type(descriptor) => builder
            .intern_type(descriptor)
            .map(DexReferenceHandle::Type)
            .map_err(native),
        ReferenceSymbol::Field {
            owner,
            name,
            descriptor,
        } => {
            let owner = builder.intern_type(owner).map_err(native)?;
            let name = builder
                .intern_utf16(name.utf16_units.clone())
                .map_err(native)?;
            let field_type = builder.intern_type(descriptor).map_err(native)?;
            builder
                .intern_field(owner, name, field_type)
                .map(DexReferenceHandle::Field)
                .map_err(native)
        }
        ReferenceSymbol::Method {
            owner,
            name,
            descriptor,
        } => {
            let owner = builder.intern_type(owner).map_err(native)?;
            let name = builder
                .intern_utf16(name.utf16_units.clone())
                .map_err(native)?;
            let prototype = prototype_handle(builder, descriptor)?;
            builder
                .intern_method(owner, name, prototype)
                .map(DexReferenceHandle::Method)
                .map_err(native)
        }
        ReferenceSymbol::MethodPrototype(descriptor) => intern_prototype(builder, descriptor),
        ReferenceSymbol::Integer(_)
        | ReferenceSymbol::Float(_)
        | ReferenceSymbol::Long(_)
        | ReferenceSymbol::Double(_) => Err(failure(format!(
            "{kind:?} is not a DEX identifier-table reference"
        ))),
    }
}

fn intern_field(
    builder: &mut DexBuilder,
    owner: &str,
    name: &str,
    descriptor: &str,
) -> Result<DexReferenceHandle, DexReferenceResolutionError> {
    if !field_type_valid(descriptor) {
        return Err(failure("field display has an invalid descriptor"));
    }
    builder
        .intern_field_named(owner, name, descriptor)
        .map(DexReferenceHandle::Field)
        .map_err(native)
}

fn intern_method(
    builder: &mut DexBuilder,
    owner: &str,
    name: &str,
    descriptor: &str,
) -> Result<DexReferenceHandle, DexReferenceResolutionError> {
    let owner = builder.intern_type(owner).map_err(native)?;
    let name = builder.intern_string(name).map_err(native)?;
    let prototype = prototype_handle(builder, descriptor)?;
    builder
        .intern_method(owner, name, prototype)
        .map(DexReferenceHandle::Method)
        .map_err(native)
}

fn intern_prototype(
    builder: &mut DexBuilder,
    descriptor: &str,
) -> Result<DexReferenceHandle, DexReferenceResolutionError> {
    prototype_handle(builder, descriptor).map(DexReferenceHandle::Prototype)
}

pub(super) fn prototype_handle(
    builder: &mut DexBuilder,
    descriptor: &str,
) -> Result<crate::file::PrototypeHandle, DexReferenceResolutionError> {
    let (parameters, return_type) = method_parts(descriptor).map_err(failure)?;
    let return_type = builder.intern_type(&return_type).map_err(native)?;
    let parameters = parameters
        .iter()
        .map(|parameter| builder.intern_type(parameter).map_err(native))
        .collect::<Result<Vec<_>, _>>()?;
    builder
        .intern_prototype(return_type, parameters)
        .map_err(native)
}

fn field(display: &str) -> Result<(&str, &str, &str), DexReferenceResolutionError> {
    let (owner, member) = display
        .split_once("->")
        .ok_or_else(|| failure("field display lacks `->`"))?;
    let (name, descriptor) = member
        .split_once(':')
        .ok_or_else(|| failure("field display lacks `:`"))?;
    Ok((owner, name, descriptor))
}

fn method(display: &str) -> Result<(&str, &str, &str), DexReferenceResolutionError> {
    let (owner, member) = display
        .split_once("->")
        .ok_or_else(|| failure("method display lacks `->`"))?;
    let split = member
        .find('(')
        .ok_or_else(|| failure("method display lacks a descriptor"))?;
    Ok((owner, &member[..split], &member[split..]))
}

#[allow(clippy::needless_pass_by_value)]
fn native(error: crate::Error) -> DexReferenceResolutionError {
    failure(error.to_string())
}

fn failure(message: impl Into<String>) -> DexReferenceResolutionError {
    DexReferenceResolutionError::new(message)
}

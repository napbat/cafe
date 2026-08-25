//! Symbolic lookup against target DEX identifier tables.

use std::borrow::Cow;

use disassembler::{Reference, ReferenceKind, ReferenceSymbol};

use crate::DexReferenceResolutionError;
use crate::file::{DexFile, FieldIndex, MethodIndex, PrototypeIndex, TypeIndex};
use crate::instruction::IndexKind;

use super::DexMlilReferenceResolver;

/// Resolves structured MLIL symbols against an explicitly supplied target DEX.
///
/// Unlike [`super::SourceDexReferenceResolver`], this resolver ignores native
/// source indices. The target file must already contain every referenced
/// identifier; use a [`crate::file::DexBuilder`] when constructing those tables.
#[derive(Debug, Clone, Copy, Default)]
pub struct TargetDexReferenceResolver;

impl DexMlilReferenceResolver for TargetDexReferenceResolver {
    fn resolve(
        &mut self,
        file: &DexFile,
        reference: &Reference,
        expected: IndexKind,
    ) -> Result<u32, DexReferenceResolutionError> {
        if !reference_kind_matches(reference.kind, expected) {
            return Err(failure(format!(
                "reference kind {:?} is incompatible with {expected:?}",
                reference.kind
            )));
        }
        match (expected, reference.symbol.as_ref()) {
            (IndexKind::String, Some(ReferenceSymbol::String(value))) => file
                .strings()
                .iter()
                .position(|candidate| candidate.utf16_units == value.utf16_units)
                .and_then(|position| u32::try_from(position).ok()),
            (IndexKind::Type, Some(ReferenceSymbol::Type(descriptor))) => {
                find_type(file, descriptor).map(TypeIndex::get)
            }
            (
                IndexKind::Field,
                Some(ReferenceSymbol::Field {
                    owner,
                    name,
                    descriptor,
                }),
            ) => find_field(file, owner, &name.utf16_units, descriptor).map(FieldIndex::get),
            (
                IndexKind::Method,
                Some(ReferenceSymbol::Method {
                    owner,
                    name,
                    descriptor,
                }),
            ) => find_method(file, owner, &name.utf16_units, descriptor).map(MethodIndex::get),
            (IndexKind::Prototype, Some(ReferenceSymbol::MethodPrototype(descriptor))) => {
                find_prototype(file, descriptor).map(PrototypeIndex::get)
            }
            _ => None,
        }
        .ok_or_else(|| {
            failure(format!(
                "target DEX has no symbolic {expected:?} identifier for the MLIL reference"
            ))
        })
    }

    fn resolve_type(
        &mut self,
        file: &DexFile,
        descriptor: &str,
    ) -> Result<TypeIndex, DexReferenceResolutionError> {
        find_type(file, descriptor).ok_or_else(|| {
            failure(format!(
                "target DEX has no type identifier for `{}`",
                dex_descriptor(descriptor)
            ))
        })
    }

    fn resolve_prototype(
        &mut self,
        file: &DexFile,
        descriptor: &str,
    ) -> Result<PrototypeIndex, DexReferenceResolutionError> {
        find_prototype(file, descriptor).ok_or_else(|| {
            failure(format!(
                "target DEX has no prototype identifier for `{descriptor}`"
            ))
        })
    }
}

fn find_type(file: &DexFile, descriptor: &str) -> Option<TypeIndex> {
    let descriptor = dex_descriptor(descriptor);
    file.types().iter().enumerate().find_map(|(position, _)| {
        let index = TypeIndex::new(u32::try_from(position).ok()?);
        (file.type_descriptor(index).ok()? == descriptor).then_some(index)
    })
}

fn find_prototype(file: &DexFile, descriptor: &str) -> Option<PrototypeIndex> {
    file.prototypes()
        .iter()
        .enumerate()
        .find_map(|(position, _)| {
            let index = PrototypeIndex::new(u32::try_from(position).ok()?);
            (file.prototype_descriptor(index).ok()? == descriptor).then_some(index)
        })
}

fn find_field(file: &DexFile, owner: &str, name: &[u16], descriptor: &str) -> Option<FieldIndex> {
    let owner = dex_descriptor(owner);
    file.fields()
        .iter()
        .enumerate()
        .find_map(|(position, field)| {
            let index = FieldIndex::new(u32::try_from(position).ok()?);
            let candidate_name = &file.resolve_string(field.name).ok()?.utf16_units;
            (file.type_descriptor(field.class).ok()? == owner
                && candidate_name == name
                && file.type_descriptor(field.field_type).ok()? == descriptor)
                .then_some(index)
        })
}

fn find_method(file: &DexFile, owner: &str, name: &[u16], descriptor: &str) -> Option<MethodIndex> {
    let owner = dex_descriptor(owner);
    file.methods()
        .iter()
        .enumerate()
        .find_map(|(position, method)| {
            let index = MethodIndex::new(u32::try_from(position).ok()?);
            let candidate_name = &file.resolve_string(method.name).ok()?.utf16_units;
            (file.type_descriptor(method.class).ok()? == owner
                && candidate_name == name
                && file.prototype_descriptor(method.prototype).ok()? == descriptor)
                .then_some(index)
        })
}

fn dex_descriptor(value: &str) -> Cow<'_, str> {
    if value.starts_with('[')
        || (value.starts_with('L') && value.ends_with(';'))
        || matches!(value, "V" | "Z" | "B" | "C" | "S" | "I" | "J" | "F" | "D")
    {
        Cow::Borrowed(value)
    } else {
        Cow::Owned(format!("L{value};"))
    }
}

const fn reference_kind_matches(kind: ReferenceKind, expected: IndexKind) -> bool {
    match expected {
        IndexKind::String => matches!(kind, ReferenceKind::String),
        IndexKind::Type => matches!(kind, ReferenceKind::Type),
        IndexKind::Field => matches!(kind, ReferenceKind::Field),
        IndexKind::Method => {
            matches!(kind, ReferenceKind::Method | ReferenceKind::InterfaceMethod)
        }
        IndexKind::Prototype => matches!(kind, ReferenceKind::MethodPrototype),
        IndexKind::CallSite => matches!(kind, ReferenceKind::DynamicCallSite),
        IndexKind::MethodHandle => matches!(kind, ReferenceKind::MethodHandle),
    }
}

fn failure(message: impl Into<String>) -> DexReferenceResolutionError {
    DexReferenceResolutionError::new(message)
}

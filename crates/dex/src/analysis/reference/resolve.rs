//! Identifier-table and encoded-value resolution.

use crate::file::{
    CallSiteIndex, DexFile, DexString, EncodedAnnotation, EncodedValue, FieldIndex,
    MethodHandleIndex, MethodIndex, PrototypeIndex, ResolvedMethodHandleTarget, StringIndex,
    TypeIndex,
};
use crate::instruction::{IndexKind, Instruction, InstructionData, Operands};
use crate::{Error, Result};

use super::{
    AnnotationElementSymbol, AnnotationSymbol, CallSiteSymbol, ExactString, FieldSymbol,
    InstructionReference, InstructionReferences, MethodHandleSymbol, MethodHandleTargetSymbol,
    MethodSymbol, PrototypeSymbol, ResolvedValue, TypeSymbol,
};

/// Resolves every identifier operand carried by one Dalvik instruction.
///
/// Non-indexed operations and payload items return an empty reference pair.
/// Exact UTF-16 string content is retained alongside the convenient text view.
///
/// # Errors
///
/// Returns an error for an incompatible operand form or an invalid table index.
pub fn resolve_instruction_references(
    file: &DexFile,
    instruction: &Instruction,
) -> Result<InstructionReferences> {
    let InstructionData::Operation { opcode, operands } = instruction.data() else {
        return Ok(InstructionReferences {
            primary: None,
            secondary_prototype: None,
        });
    };
    let Some(kind) = opcode.index_kind() else {
        return Ok(InstructionReferences {
            primary: None,
            secondary_prototype: None,
        });
    };
    let (primary_index, secondary_index) = indexed_operands(operands).ok_or_else(|| {
        Error::invalid_instruction(
            instruction.offset(),
            format!("{} lacks its indexed operand", opcode.mnemonic()),
        )
    })?;
    let primary = Some(resolve_reference(file, kind, primary_index)?);
    let secondary_prototype = secondary_index
        .map(|index| prototype(file, PrototypeIndex::new(index)))
        .transpose()?;
    Ok(InstructionReferences {
        primary,
        secondary_prototype,
    })
}

/// Resolves one recursively encoded DEX value.
///
/// # Errors
///
/// Returns an error when any nested identifier index is invalid.
pub fn resolve_value(file: &DexFile, value: &EncodedValue) -> Result<ResolvedValue> {
    Ok(match value {
        EncodedValue::Byte(value) => ResolvedValue::Byte(*value),
        EncodedValue::Short(value) => ResolvedValue::Short(*value),
        EncodedValue::Char(value) => ResolvedValue::Char(*value),
        EncodedValue::Int(value) => ResolvedValue::Int(*value),
        EncodedValue::Long(value) => ResolvedValue::Long(*value),
        EncodedValue::Float(value) => ResolvedValue::Float(*value),
        EncodedValue::Double(value) => ResolvedValue::Double(*value),
        EncodedValue::MethodType(index) => ResolvedValue::MethodType(prototype(file, *index)?),
        EncodedValue::MethodHandle(index) => {
            ResolvedValue::MethodHandle(method_handle(file, *index)?)
        }
        EncodedValue::String(index) => ResolvedValue::String(string(file, *index)?),
        EncodedValue::Type(index) => ResolvedValue::Type(type_symbol(file, *index)?),
        EncodedValue::Field(index) => ResolvedValue::Field(field(file, *index)?),
        EncodedValue::Method(index) => ResolvedValue::Method(method(file, *index)?),
        EncodedValue::Enum(index) => ResolvedValue::Enum(field(file, *index)?),
        EncodedValue::Array(values) => ResolvedValue::Array(
            values
                .iter()
                .map(|value| resolve_value(file, value))
                .collect::<Result<Vec<_>>>()?,
        ),
        EncodedValue::Annotation(annotation) => {
            ResolvedValue::Annotation(resolve_annotation(file, annotation)?)
        }
        EncodedValue::Null => ResolvedValue::Null,
        EncodedValue::Boolean(value) => ResolvedValue::Boolean(*value),
    })
}

fn resolve_reference(file: &DexFile, kind: IndexKind, index: u32) -> Result<InstructionReference> {
    Ok(match kind {
        IndexKind::String => InstructionReference::String(string(file, StringIndex::new(index))?),
        IndexKind::Type => InstructionReference::Type(type_symbol(file, TypeIndex::new(index))?),
        IndexKind::Field => InstructionReference::Field(field(file, FieldIndex::new(index))?),
        IndexKind::Method => InstructionReference::Method(method(file, MethodIndex::new(index))?),
        IndexKind::Prototype => {
            InstructionReference::Prototype(prototype(file, PrototypeIndex::new(index))?)
        }
        IndexKind::CallSite => {
            InstructionReference::CallSite(call_site(file, CallSiteIndex::new(index))?)
        }
        IndexKind::MethodHandle => {
            InstructionReference::MethodHandle(method_handle(file, MethodHandleIndex::new(index))?)
        }
    })
}

fn indexed_operands(operands: &Operands) -> Option<(u32, Option<u32>)> {
    match operands {
        Operands::RegisterIndex { index, .. } | Operands::RegistersIndex { index, .. } => {
            Some((*index, None))
        }
        Operands::RegisterListIndex {
            index,
            secondary_index,
            ..
        }
        | Operands::RegisterRangeIndex {
            index,
            secondary_index,
            ..
        } => Some((*index, *secondary_index)),
        _ => None,
    }
}

fn string(file: &DexFile, index: StringIndex) -> Result<ExactString> {
    Ok(exact_string(file.resolve_string(index)?))
}

fn exact_string(value: &DexString) -> ExactString {
    ExactString {
        text: value.text.clone(),
        utf16_units: value.utf16_units.clone(),
    }
}

fn type_symbol(file: &DexFile, index: TypeIndex) -> Result<TypeSymbol> {
    Ok(TypeSymbol {
        descriptor: file.type_descriptor(index)?.to_owned(),
    })
}

fn field(file: &DexFile, index: FieldIndex) -> Result<FieldSymbol> {
    let identity = file.resolve_field_id(index)?;
    Ok(FieldSymbol {
        owner: file.type_descriptor(identity.class)?.to_owned(),
        name: string(file, identity.name)?,
        descriptor: file.type_descriptor(identity.field_type)?.to_owned(),
    })
}

fn method(file: &DexFile, index: MethodIndex) -> Result<MethodSymbol> {
    let identity = file.resolve_method_id(index)?;
    Ok(MethodSymbol {
        owner: file.type_descriptor(identity.class)?.to_owned(),
        name: string(file, identity.name)?,
        descriptor: file.prototype_descriptor(identity.prototype)?,
    })
}

fn prototype(file: &DexFile, index: PrototypeIndex) -> Result<PrototypeSymbol> {
    Ok(PrototypeSymbol {
        descriptor: file.prototype_descriptor(index)?,
    })
}

fn method_handle(file: &DexFile, index: MethodHandleIndex) -> Result<MethodHandleSymbol> {
    let handle = file.resolve_method_handle(index)?;
    let target = match handle.target {
        ResolvedMethodHandleTarget::Field(_) => {
            let raw = file.resolve_method_handle_id(index)?;
            MethodHandleTargetSymbol::Field(field(
                file,
                FieldIndex::new(u32::from(raw.target_index)),
            )?)
        }
        ResolvedMethodHandleTarget::Method(_) => {
            let raw = file.resolve_method_handle_id(index)?;
            MethodHandleTargetSymbol::Method(method(
                file,
                MethodIndex::new(u32::from(raw.target_index)),
            )?)
        }
    };
    Ok(MethodHandleSymbol {
        kind: handle.kind,
        target,
    })
}

fn call_site(file: &DexFile, index: CallSiteIndex) -> Result<CallSiteSymbol> {
    let call_site = file.resolve_call_site(index)?;
    let components = call_site.components().ok_or_else(|| {
        Error::invalid_instruction(0, "call site lacks its required typed prefix")
    })?;
    Ok(CallSiteSymbol {
        bootstrap_method: method_handle(file, components.bootstrap_method)?,
        method_name: string(file, components.method_name)?,
        descriptor: file.prototype_descriptor(components.method_type)?,
        arguments: components
            .arguments
            .iter()
            .map(|value| resolve_value(file, value))
            .collect::<Result<Vec<_>>>()?,
    })
}

fn resolve_annotation(file: &DexFile, annotation: &EncodedAnnotation) -> Result<AnnotationSymbol> {
    Ok(AnnotationSymbol {
        descriptor: file.type_descriptor(annotation.annotation_type)?.to_owned(),
        elements: annotation
            .elements
            .iter()
            .map(|element| {
                Ok(AnnotationElementSymbol {
                    name: string(file, element.name)?,
                    value: resolve_value(file, &element.value)?,
                })
            })
            .collect::<Result<Vec<_>>>()?,
    })
}

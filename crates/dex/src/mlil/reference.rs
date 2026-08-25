//! DEX identifier-table symbols converted into shared references.

use disassembler::{ExactText, Reference, ReferenceKind, ReferenceSymbol};

use crate::analysis::{
    CallSiteSymbol, ExactString, FieldSymbol, InstructionReference, InstructionReferences,
    MethodSymbol, PrototypeSymbol, TypeSymbol, resolve_instruction_references,
};
use crate::file::DexFile;
use crate::instruction::Instruction;
use crate::llil::Invocation;

use super::{Error, Result};

pub(super) fn constant_reference(
    file: &DexFile,
    instruction: &Instruction,
    index: u32,
) -> Result<Reference> {
    match references(file, instruction)?.primary {
        Some(InstructionReference::String(value)) => {
            Ok(
                Reference::resolved(ReferenceKind::String, index, &value.text)
                    .with_symbol(ReferenceSymbol::String(exact(value))),
            )
        }
        Some(InstructionReference::Type(value)) => Ok(type_symbol(index, value)),
        Some(InstructionReference::Prototype(value)) => Ok(prototype(index, value)),
        Some(InstructionReference::MethodHandle(value)) => Ok(Reference::resolved(
            ReferenceKind::MethodHandle,
            index,
            format!("{:?} {:?}", value.kind, value.target),
        )),
        _ => Err(Error::unsupported(
            instruction.offset(),
            "constant opcode did not resolve to its expected identifier table",
        )),
    }
}

pub(super) fn type_reference(
    file: &DexFile,
    instruction: &Instruction,
    index: u32,
) -> Result<Reference> {
    let Some(InstructionReference::Type(symbol)) = references(file, instruction)?.primary else {
        return Err(Error::unsupported(
            instruction.offset(),
            "type opcode did not resolve to a type descriptor",
        ));
    };
    Ok(type_symbol(index, symbol))
}

pub(super) fn field_reference(
    file: &DexFile,
    instruction: &Instruction,
    index: u32,
) -> Result<(Reference, String)> {
    let Some(InstructionReference::Field(symbol)) = references(file, instruction)?.primary else {
        return Err(Error::unsupported(
            instruction.offset(),
            "field opcode did not resolve to a field identity",
        ));
    };
    let descriptor = symbol.descriptor.clone();
    Ok((field(index, symbol), descriptor))
}

pub(super) fn call_reference(
    file: &DexFile,
    instruction: &Instruction,
    invocation: Invocation,
    index: u32,
) -> Result<(Reference, String)> {
    let references = references(file, instruction)?;
    match (invocation, references.primary) {
        (Invocation::Custom, Some(InstructionReference::CallSite(symbol))) => {
            let descriptor = symbol.descriptor.clone();
            Ok((call_site(index, &symbol), descriptor))
        }
        (_, Some(InstructionReference::Method(symbol))) => {
            let descriptor = if invocation == Invocation::Polymorphic {
                references.secondary_prototype.as_ref().map_or_else(
                    || symbol.descriptor.clone(),
                    |value| value.descriptor.clone(),
                )
            } else {
                symbol.descriptor.clone()
            };
            Ok((method(index, symbol), descriptor))
        }
        _ => Err(Error::unsupported(
            instruction.offset(),
            "invoke opcode did not resolve to a method or call site",
        )),
    }
}

fn references(file: &DexFile, instruction: &Instruction) -> Result<InstructionReferences> {
    Ok(resolve_instruction_references(file, instruction)?)
}

fn type_symbol(index: u32, symbol: TypeSymbol) -> Reference {
    Reference::resolved(ReferenceKind::Type, index, &symbol.descriptor)
        .with_symbol(ReferenceSymbol::Type(symbol.descriptor))
}

fn field(index: u32, symbol: FieldSymbol) -> Reference {
    let display = format!(
        "{}.{}:{}",
        symbol.owner, symbol.name.text, symbol.descriptor
    );
    Reference::resolved(ReferenceKind::Field, index, display).with_symbol(ReferenceSymbol::Field {
        owner: symbol.owner,
        name: exact(symbol.name),
        descriptor: symbol.descriptor,
    })
}

fn method(index: u32, symbol: MethodSymbol) -> Reference {
    let display = format!("{}.{}{}", symbol.owner, symbol.name.text, symbol.descriptor);
    Reference::resolved(ReferenceKind::Method, index, display).with_symbol(
        ReferenceSymbol::Method {
            owner: symbol.owner,
            name: exact(symbol.name),
            descriptor: symbol.descriptor,
        },
    )
}

fn prototype(index: u32, symbol: PrototypeSymbol) -> Reference {
    Reference::resolved(ReferenceKind::MethodPrototype, index, &symbol.descriptor)
        .with_symbol(ReferenceSymbol::MethodPrototype(symbol.descriptor))
}

fn call_site(index: u32, symbol: &CallSiteSymbol) -> Reference {
    Reference::resolved(
        ReferenceKind::DynamicCallSite,
        index,
        format!("{}{}", symbol.method_name.text, symbol.descriptor),
    )
}

fn exact(value: ExactString) -> ExactText {
    ExactText {
        text: value.text,
        utf16_units: value.utf16_units,
    }
}

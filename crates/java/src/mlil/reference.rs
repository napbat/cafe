//! JVM constant-pool symbols converted into shared typed references.

use ::mlil::Constant;
use disassembler::{ExactText, Reference, ReferenceKind, ReferenceSymbol};

use crate::analysis::{
    ClassSymbol, DynamicSymbol, ExactString, FieldSymbol, InstructionReference, LoadableConstant,
    MethodReferenceKind, MethodSymbol, resolve_instruction_reference,
};
use crate::bytecode::Instruction;
use crate::classfile::ConstantPool;

use super::{Error, Result};

pub(super) fn loadable_constant(
    pool: &ConstantPool,
    instruction: &Instruction,
    index: u16,
) -> Result<Constant> {
    let reference = require_reference(pool, instruction)?;
    let InstructionReference::Constant(value) = reference else {
        return Err(Error::unsupported(
            instruction.offset,
            "ldc did not resolve to a loadable constant",
        ));
    };
    Ok(match value {
        LoadableConstant::Integer(value) => Constant::Integer(value),
        LoadableConstant::Float(value) => Constant::Float(value),
        LoadableConstant::Long(value) => Constant::Long(value),
        LoadableConstant::Double(value) => Constant::Double(value),
        LoadableConstant::String(value) => Constant::Reference(
            Reference::resolved(ReferenceKind::String, u32::from(index), &value.text)
                .with_symbol(ReferenceSymbol::String(exact(value))),
        ),
        LoadableConstant::Class(value) => Constant::Reference(class(index, value)),
        LoadableConstant::MethodType(descriptor) => Constant::Reference(
            Reference::resolved(
                ReferenceKind::MethodPrototype,
                u32::from(index),
                &descriptor,
            )
            .with_symbol(ReferenceSymbol::MethodPrototype(descriptor)),
        ),
        LoadableConstant::MethodHandle(handle) => Constant::Reference(Reference::resolved(
            ReferenceKind::MethodHandle,
            u32::from(index),
            format!("{:?} {:?}", handle.kind, handle.target),
        )),
        LoadableConstant::Dynamic(dynamic) => {
            Constant::Reference(dynamic_constant(index, &dynamic))
        }
    })
}

pub(super) fn field(
    pool: &ConstantPool,
    instruction: &Instruction,
    index: u16,
) -> Result<(Reference, String)> {
    let InstructionReference::Field(symbol) = require_reference(pool, instruction)? else {
        return Err(Error::unsupported(
            instruction.offset,
            "field opcode did not resolve to a field symbol",
        ));
    };
    let descriptor = symbol.descriptor.clone();
    Ok((field_symbol(index, symbol), descriptor))
}

pub(super) fn method(
    pool: &ConstantPool,
    instruction: &Instruction,
    index: u16,
) -> Result<(Reference, String)> {
    match require_reference(pool, instruction)? {
        InstructionReference::Method(symbol) => {
            let descriptor = symbol.descriptor.clone();
            Ok((method_symbol(index, symbol), descriptor))
        }
        InstructionReference::DynamicCallSite(symbol) => {
            let descriptor = symbol.descriptor.clone();
            Ok((dynamic_call_site(index, &symbol), descriptor))
        }
        _ => Err(Error::unsupported(
            instruction.offset,
            "invocation did not resolve to a method or dynamic call site",
        )),
    }
}

pub(super) fn class_reference(
    pool: &ConstantPool,
    instruction: &Instruction,
    index: u16,
) -> Result<Reference> {
    let InstructionReference::Class(symbol) = require_reference(pool, instruction)? else {
        return Err(Error::unsupported(
            instruction.offset,
            "type opcode did not resolve to a class symbol",
        ));
    };
    Ok(class(index, symbol))
}

fn require_reference(
    pool: &ConstantPool,
    instruction: &Instruction,
) -> Result<InstructionReference> {
    resolve_instruction_reference(pool, instruction)?.ok_or_else(|| {
        Error::unsupported(
            instruction.offset,
            "indexed instruction did not expose a symbolic reference",
        )
    })
}

fn class(index: u16, symbol: ClassSymbol) -> Reference {
    Reference::resolved(ReferenceKind::Type, u32::from(index), &symbol.name.text)
        .with_symbol(ReferenceSymbol::Type(symbol.name.text))
}

fn field_symbol(index: u16, symbol: FieldSymbol) -> Reference {
    let display = format!(
        "{}.{}:{}",
        symbol.owner.name.text, symbol.name.text, symbol.descriptor
    );
    Reference::resolved(ReferenceKind::Field, u32::from(index), display).with_symbol(
        ReferenceSymbol::Field {
            owner: symbol.owner.name.text,
            name: exact(symbol.name),
            descriptor: symbol.descriptor,
        },
    )
}

fn method_symbol(index: u16, symbol: MethodSymbol) -> Reference {
    let kind = match symbol.kind {
        MethodReferenceKind::Class => ReferenceKind::Method,
        MethodReferenceKind::Interface => ReferenceKind::InterfaceMethod,
    };
    let display = format!(
        "{}.{}{}",
        symbol.owner.name.text, symbol.name.text, symbol.descriptor
    );
    Reference::resolved(kind, u32::from(index), display).with_symbol(ReferenceSymbol::Method {
        owner: symbol.owner.name.text,
        name: exact(symbol.name),
        descriptor: symbol.descriptor,
    })
}

fn dynamic_call_site(index: u16, symbol: &DynamicSymbol) -> Reference {
    Reference::resolved(
        ReferenceKind::DynamicCallSite,
        u32::from(index),
        format!(
            "{}{} bootstrap#{}",
            symbol.name.text, symbol.descriptor, symbol.bootstrap_method
        ),
    )
}

fn dynamic_constant(index: u16, symbol: &DynamicSymbol) -> Reference {
    Reference::resolved(
        ReferenceKind::Constant,
        u32::from(index),
        format!(
            "{}:{} bootstrap#{}",
            symbol.name.text, symbol.descriptor, symbol.bootstrap_method
        ),
    )
}

fn exact(value: ExactString) -> ExactText {
    ExactText {
        text: value.text,
        utf16_units: value.utf16_units,
    }
}

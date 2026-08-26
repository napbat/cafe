//! JVM category selection from target-neutral MLIL type evidence.

use ::mlil::{
    AllocationKind, ArrayAccess, BranchOperandKind, CallKind, FieldAccess, Function, Instruction,
    Operation, ValueType,
};
use disassembler::ReferenceSymbol;

use super::super::{Error, Result};

pub(super) fn zero_use_is_reference(
    function: &Function,
    instruction: &Instruction,
    position: usize,
) -> Result<bool> {
    let reference = match instruction.operation() {
        Operation::TypeRefine => instruction
            .def_types()
            .get(position)
            .is_some_and(ValueType::is_reference),
        Operation::Branch(predicate) => matches!(
            predicate.operands,
            BranchOperandKind::ReferencePair | BranchOperandKind::ReferenceNull
        ),
        Operation::Return => method_types(&function.source().symbol.signature)
            .and_then(|(_, result)| result)
            .ok_or_else(|| lowering(instruction, "cannot classify the method return type"))?,
        Operation::Throw
        | Operation::ArrayLength
        | Operation::InitializeArray { .. }
        | Operation::CheckCast(_)
        | Operation::InstanceOf(_)
        | Operation::Monitor(_) => true,
        Operation::Array { access, element } => match (access, position) {
            (_, 0) => true,
            (ArrayAccess::Put, 2) => matches!(element, ::mlil::ElementType::Reference),
            _ => false,
        },
        Operation::Field { access, field } => match (access, position) {
            (FieldAccess::GetInstance | FieldAccess::PutInstance, 0) => true,
            (FieldAccess::PutInstance, 1) | (FieldAccess::PutStatic, 0) => {
                reference_field_type(field)
                    .and_then(descriptor_is_reference)
                    .ok_or_else(|| lowering(instruction, "cannot classify the field value type"))?
            }
            _ => false,
        },
        Operation::Call {
            kind, descriptor, ..
        } => call_use_is_reference(*kind, descriptor.as_deref(), position)
            .ok_or_else(|| lowering(instruction, "cannot classify a call operand type"))?,
        Operation::Allocate(AllocationKind::InitializedArray { array_type }) => array_type
            .descriptor()
            .strip_prefix('[')
            .and_then(descriptor_is_reference)
            .ok_or_else(|| lowering(instruction, "cannot classify an initialized-array element"))?,
        Operation::Copy
        | Operation::ParallelCopy
        | Operation::Discard
        | Operation::Unary(_)
        | Operation::Binary(_)
        | Operation::Convert(_)
        | Operation::Compare(_)
        | Operation::Switch(_)
        | Operation::Allocate(_)
        | Operation::Nop
        | Operation::Constant(_)
        | Operation::CaughtException(_)
        | Operation::Intrinsic(_)
        | Operation::Select
        | Operation::Jump => false,
    };
    Ok(reference)
}

pub(super) fn method_return_is_reference(descriptor: &str) -> Option<bool> {
    method_types(descriptor).and_then(|(_, result)| result)
}

fn reference_field_type(reference: &disassembler::Reference) -> Option<&str> {
    match &reference.symbol {
        Some(ReferenceSymbol::Field { descriptor, .. }) => Some(descriptor),
        _ => None,
    }
}

fn call_use_is_reference(
    kind: CallKind,
    descriptor: Option<&str>,
    position: usize,
) -> Option<bool> {
    let receiver = !matches!(kind, CallKind::Static | CallKind::Dynamic);
    if receiver && position == 0 {
        return Some(true);
    }
    let (parameters, _) = method_types(descriptor?)?;
    parameters
        .get(position.checked_sub(usize::from(receiver))?)
        .copied()
}

fn method_types(descriptor: &str) -> Option<(Vec<bool>, Option<bool>)> {
    let bytes = descriptor.as_bytes();
    if bytes.first() != Some(&b'(') {
        return None;
    }
    let mut cursor = 1usize;
    let mut parameters = Vec::new();
    while bytes.get(cursor) != Some(&b')') {
        parameters.push(parse_type(bytes, &mut cursor)?);
    }
    cursor += 1;
    if bytes.get(cursor) == Some(&b'V') {
        cursor += 1;
        return (cursor == bytes.len()).then_some((parameters, None));
    }
    let result = parse_type(bytes, &mut cursor)?;
    (cursor == bytes.len()).then_some((parameters, Some(result)))
}

fn descriptor_is_reference(descriptor: &str) -> Option<bool> {
    let mut cursor = 0usize;
    let reference = parse_type(descriptor.as_bytes(), &mut cursor)?;
    (cursor == descriptor.len()).then_some(reference)
}

fn parse_type(bytes: &[u8], cursor: &mut usize) -> Option<bool> {
    match *bytes.get(*cursor)? {
        b'[' => {
            while bytes.get(*cursor) == Some(&b'[') {
                *cursor += 1;
            }
            parse_type(bytes, cursor)?;
            Some(true)
        }
        b'L' => {
            *cursor += 1;
            let start = *cursor;
            while bytes.get(*cursor) != Some(&b';') {
                *cursor += 1;
                bytes.get(*cursor)?;
            }
            if *cursor == start {
                return None;
            }
            *cursor += 1;
            Some(true)
        }
        b'Z' | b'B' | b'C' | b'S' | b'I' | b'J' | b'F' | b'D' => {
            *cursor += 1;
            Some(false)
        }
        _ => None,
    }
}

fn lowering(instruction: &Instruction, message: &str) -> Error {
    Error::lowering(instruction.id(), message)
}

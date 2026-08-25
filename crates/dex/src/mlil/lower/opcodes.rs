//! Dalvik opcode selection from target-neutral MLIL semantics.

use ::mlil::{
    ArrayAccess, BinaryOperator, BranchOperandKind, BranchPredicate, CallKind, Conversion,
    ElementType, FieldAccess, Instruction, InstructionId, Relation, ThreeWayComparison,
    UnaryOperator, ValueType,
};
use disassembler::{Reference, ReferenceSymbol};

use crate::instruction::Opcode;

use super::{Error, Result};

pub(super) fn unary_opcode(operator: UnaryOperator, value_type: &ValueType) -> Opcode {
    match (operator, value_type) {
        (UnaryOperator::Negate, ValueType::Long | ValueType::Bits64) => Opcode::NegLong,
        (UnaryOperator::Negate, ValueType::Float) => Opcode::NegFloat,
        (UnaryOperator::Negate, ValueType::Double) => Opcode::NegDouble,
        (UnaryOperator::Negate, _) => Opcode::NegInt,
        (UnaryOperator::BitwiseNot, ValueType::Long | ValueType::Bits64) => Opcode::NotLong,
        (UnaryOperator::BitwiseNot, _) => Opcode::NotInt,
    }
}

pub(super) fn binary_opcode(
    operator: BinaryOperator,
    value_type: &ValueType,
    id: InstructionId,
) -> Result<Opcode> {
    use BinaryOperator as B;
    let kind = match value_type {
        ValueType::Long | ValueType::Bits64 => 'l',
        ValueType::Float => 'f',
        ValueType::Double => 'd',
        _ => 'i',
    };
    let opcode = match (operator, kind) {
        (B::Add, 'i') => Opcode::AddInt,
        (B::Add, 'l') => Opcode::AddLong,
        (B::Add, 'f') => Opcode::AddFloat,
        (B::Add, 'd') => Opcode::AddDouble,
        (B::Subtract | B::ReverseSubtract, 'i') => Opcode::SubInt,
        (B::Subtract, 'l') => Opcode::SubLong,
        (B::Subtract, 'f') => Opcode::SubFloat,
        (B::Subtract, 'd') => Opcode::SubDouble,
        (B::Multiply, 'i') => Opcode::MulInt,
        (B::Multiply, 'l') => Opcode::MulLong,
        (B::Multiply, 'f') => Opcode::MulFloat,
        (B::Multiply, 'd') => Opcode::MulDouble,
        (B::Divide, 'i') => Opcode::DivInt,
        (B::Divide, 'l') => Opcode::DivLong,
        (B::Divide, 'f') => Opcode::DivFloat,
        (B::Divide, 'd') => Opcode::DivDouble,
        (B::Remainder, 'i') => Opcode::RemInt,
        (B::Remainder, 'l') => Opcode::RemLong,
        (B::Remainder, 'f') => Opcode::RemFloat,
        (B::Remainder, 'd') => Opcode::RemDouble,
        (B::And, 'i') => Opcode::AndInt,
        (B::And, 'l') => Opcode::AndLong,
        (B::Or, 'i') => Opcode::OrInt,
        (B::Or, 'l') => Opcode::OrLong,
        (B::Xor, 'i') => Opcode::XorInt,
        (B::Xor, 'l') => Opcode::XorLong,
        (B::ShiftLeft, 'i') => Opcode::ShlInt,
        (B::ShiftLeft, 'l') => Opcode::ShlLong,
        (B::ShiftRight, 'i') => Opcode::ShrInt,
        (B::ShiftRight, 'l') => Opcode::ShrLong,
        (B::UnsignedShiftRight, 'i') => Opcode::UshrInt,
        (B::UnsignedShiftRight, 'l') => Opcode::UshrLong,
        _ => {
            return Err(Error::lowering(
                id,
                "operator is incompatible with Dalvik value category",
            ));
        }
    };
    Ok(opcode)
}

pub(super) const fn conversion_opcode(conversion: Conversion) -> Opcode {
    match conversion {
        Conversion::IntToLong => Opcode::IntToLong,
        Conversion::IntToFloat => Opcode::IntToFloat,
        Conversion::IntToDouble => Opcode::IntToDouble,
        Conversion::LongToInt => Opcode::LongToInt,
        Conversion::LongToFloat => Opcode::LongToFloat,
        Conversion::LongToDouble => Opcode::LongToDouble,
        Conversion::FloatToInt => Opcode::FloatToInt,
        Conversion::FloatToLong => Opcode::FloatToLong,
        Conversion::FloatToDouble => Opcode::FloatToDouble,
        Conversion::DoubleToInt => Opcode::DoubleToInt,
        Conversion::DoubleToLong => Opcode::DoubleToLong,
        Conversion::DoubleToFloat => Opcode::DoubleToFloat,
        Conversion::IntToByte => Opcode::IntToByte,
        Conversion::IntToChar => Opcode::IntToChar,
        Conversion::IntToShort => Opcode::IntToShort,
    }
}

pub(super) const fn comparison_opcode(comparison: ThreeWayComparison) -> Opcode {
    match comparison {
        ThreeWayComparison::Long => Opcode::CmpLong,
        ThreeWayComparison::FloatNanLow => Opcode::CmplFloat,
        ThreeWayComparison::FloatNanHigh => Opcode::CmpgFloat,
        ThreeWayComparison::DoubleNanLow => Opcode::CmplDouble,
        ThreeWayComparison::DoubleNanHigh => Opcode::CmpgDouble,
    }
}

pub(super) const fn branch_opcode(predicate: BranchPredicate) -> Opcode {
    if matches!(predicate.operands, BranchOperandKind::Boolean) {
        return if matches!(predicate.relation, Relation::Equal) {
            Opcode::IfNez
        } else {
            Opcode::IfEqz
        };
    }
    let zero = matches!(
        predicate.operands,
        BranchOperandKind::IntegerZero | BranchOperandKind::ReferenceNull
    );
    match (predicate.relation, zero) {
        (Relation::Equal, true) => Opcode::IfEqz,
        (Relation::NotEqual, true) => Opcode::IfNez,
        (Relation::Less, true) => Opcode::IfLtz,
        (Relation::GreaterOrEqual, true) => Opcode::IfGez,
        (Relation::Greater, true) => Opcode::IfGtz,
        (Relation::LessOrEqual, true) => Opcode::IfLez,
        (Relation::Equal, false) => Opcode::IfEq,
        (Relation::NotEqual, false) => Opcode::IfNe,
        (Relation::Less, false) => Opcode::IfLt,
        (Relation::GreaterOrEqual, false) => Opcode::IfGe,
        (Relation::Greater, false) => Opcode::IfGt,
        (Relation::LessOrEqual, false) => Opcode::IfLe,
    }
}

pub(super) const fn inverted_branch(opcode: Opcode) -> Opcode {
    match opcode {
        Opcode::IfEq => Opcode::IfNe,
        Opcode::IfNe => Opcode::IfEq,
        Opcode::IfLt => Opcode::IfGe,
        Opcode::IfGe => Opcode::IfLt,
        Opcode::IfGt => Opcode::IfLe,
        Opcode::IfLe => Opcode::IfGt,
        Opcode::IfEqz => Opcode::IfNez,
        Opcode::IfNez => Opcode::IfEqz,
        Opcode::IfLtz => Opcode::IfGez,
        Opcode::IfGez => Opcode::IfLtz,
        Opcode::IfGtz => Opcode::IfLez,
        Opcode::IfLez => Opcode::IfGtz,
        _ => opcode,
    }
}

pub(super) const fn return_opcode(value_type: &ValueType) -> Opcode {
    match value_type {
        ValueType::Long | ValueType::Double | ValueType::Bits64 => Opcode::ReturnWide,
        ValueType::Null
        | ValueType::Reference(_)
        | ValueType::UninitializedThis(_)
        | ValueType::Uninitialized { .. } => Opcode::ReturnObject,
        _ => Opcode::Return,
    }
}

pub(super) const fn array_opcode(access: ArrayAccess, element: ElementType) -> Opcode {
    match (access, element) {
        (ArrayAccess::Get, ElementType::Bits32 | ElementType::Integer | ElementType::Float) => {
            Opcode::Aget
        }
        (ArrayAccess::Get, ElementType::Bits64 | ElementType::Long | ElementType::Double) => {
            Opcode::AgetWide
        }
        (ArrayAccess::Get, ElementType::Reference) => Opcode::AgetObject,
        (ArrayAccess::Get, ElementType::Boolean) => Opcode::AgetBoolean,
        (ArrayAccess::Get, ElementType::Byte | ElementType::ByteOrBoolean) => Opcode::AgetByte,
        (ArrayAccess::Get, ElementType::Char) => Opcode::AgetChar,
        (ArrayAccess::Get, ElementType::Short) => Opcode::AgetShort,
        (ArrayAccess::Put, ElementType::Bits32 | ElementType::Integer | ElementType::Float) => {
            Opcode::Aput
        }
        (ArrayAccess::Put, ElementType::Bits64 | ElementType::Long | ElementType::Double) => {
            Opcode::AputWide
        }
        (ArrayAccess::Put, ElementType::Reference) => Opcode::AputObject,
        (ArrayAccess::Put, ElementType::Boolean) => Opcode::AputBoolean,
        (ArrayAccess::Put, ElementType::Byte | ElementType::ByteOrBoolean) => Opcode::AputByte,
        (ArrayAccess::Put, ElementType::Char) => Opcode::AputChar,
        (ArrayAccess::Put, ElementType::Short) => Opcode::AputShort,
    }
}

pub(super) fn field_opcode(
    access: FieldAccess,
    field: &Reference,
    instruction: &Instruction,
) -> Opcode {
    let descriptor = match &field.symbol {
        Some(ReferenceSymbol::Field { descriptor, .. }) => descriptor.as_str(),
        _ => "",
    };
    let value_type = match access {
        FieldAccess::GetInstance | FieldAccess::GetStatic => instruction.def_types().first(),
        FieldAccess::PutInstance | FieldAccess::PutStatic => instruction.use_types().last(),
    };
    let suffix = match descriptor.as_bytes().first() {
        Some(b'Z') => 'z',
        Some(b'B') => 'b',
        Some(b'C') => 'c',
        Some(b'S') => 's',
        Some(b'J' | b'D') => 'w',
        Some(b'L' | b'[') => 'o',
        _ => match value_type {
            Some(ValueType::Long | ValueType::Double | ValueType::Bits64) => 'w',
            Some(value) if value.is_reference() => 'o',
            _ => 'n',
        },
    };
    match (access, suffix) {
        (FieldAccess::GetInstance, 'w') => Opcode::IgetWide,
        (FieldAccess::GetInstance, 'o') => Opcode::IgetObject,
        (FieldAccess::GetInstance, 'z') => Opcode::IgetBoolean,
        (FieldAccess::GetInstance, 'b') => Opcode::IgetByte,
        (FieldAccess::GetInstance, 'c') => Opcode::IgetChar,
        (FieldAccess::GetInstance, 's') => Opcode::IgetShort,
        (FieldAccess::GetInstance, _) => Opcode::Iget,
        (FieldAccess::PutInstance, 'w') => Opcode::IputWide,
        (FieldAccess::PutInstance, 'o') => Opcode::IputObject,
        (FieldAccess::PutInstance, 'z') => Opcode::IputBoolean,
        (FieldAccess::PutInstance, 'b') => Opcode::IputByte,
        (FieldAccess::PutInstance, 'c') => Opcode::IputChar,
        (FieldAccess::PutInstance, 's') => Opcode::IputShort,
        (FieldAccess::PutInstance, _) => Opcode::Iput,
        (FieldAccess::GetStatic, 'w') => Opcode::SgetWide,
        (FieldAccess::GetStatic, 'o') => Opcode::SgetObject,
        (FieldAccess::GetStatic, 'z') => Opcode::SgetBoolean,
        (FieldAccess::GetStatic, 'b') => Opcode::SgetByte,
        (FieldAccess::GetStatic, 'c') => Opcode::SgetChar,
        (FieldAccess::GetStatic, 's') => Opcode::SgetShort,
        (FieldAccess::GetStatic, _) => Opcode::Sget,
        (FieldAccess::PutStatic, 'w') => Opcode::SputWide,
        (FieldAccess::PutStatic, 'o') => Opcode::SputObject,
        (FieldAccess::PutStatic, 'z') => Opcode::SputBoolean,
        (FieldAccess::PutStatic, 'b') => Opcode::SputByte,
        (FieldAccess::PutStatic, 'c') => Opcode::SputChar,
        (FieldAccess::PutStatic, 's') => Opcode::SputShort,
        (FieldAccess::PutStatic, _) => Opcode::Sput,
    }
}

pub(super) const fn call_opcode(kind: CallKind) -> Opcode {
    match kind {
        CallKind::Virtual => Opcode::InvokeVirtualRange,
        CallKind::Super => Opcode::InvokeSuperRange,
        CallKind::Direct => Opcode::InvokeDirectRange,
        CallKind::Static => Opcode::InvokeStaticRange,
        CallKind::Interface => Opcode::InvokeInterfaceRange,
        CallKind::Polymorphic => Opcode::InvokePolymorphicRange,
        CallKind::Dynamic => Opcode::InvokeCustomRange,
    }
}

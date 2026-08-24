//! Opcode families used by register transfer.

use crate::instruction::Opcode;

pub(super) const fn is_move_object(opcode: Opcode) -> bool {
    matches!(
        opcode,
        Opcode::MoveObject | Opcode::MoveObjectFrom16 | Opcode::MoveObject16
    )
}

pub(super) const fn is_invocation(opcode: Opcode) -> bool {
    matches!(
        opcode,
        Opcode::InvokeVirtual
            | Opcode::InvokeSuper
            | Opcode::InvokeDirect
            | Opcode::InvokeStatic
            | Opcode::InvokeInterface
            | Opcode::InvokeVirtualRange
            | Opcode::InvokeSuperRange
            | Opcode::InvokeDirectRange
            | Opcode::InvokeStaticRange
            | Opcode::InvokeInterfaceRange
            | Opcode::InvokePolymorphic
            | Opcode::InvokePolymorphicRange
            | Opcode::InvokeCustom
            | Opcode::InvokeCustomRange
    )
}

pub(super) const fn is_field_get(opcode: Opcode) -> bool {
    matches!(
        opcode,
        Opcode::Iget
            | Opcode::IgetWide
            | Opcode::IgetObject
            | Opcode::IgetBoolean
            | Opcode::IgetByte
            | Opcode::IgetChar
            | Opcode::IgetShort
            | Opcode::Sget
            | Opcode::SgetWide
            | Opcode::SgetObject
            | Opcode::SgetBoolean
            | Opcode::SgetByte
            | Opcode::SgetChar
            | Opcode::SgetShort
    )
}

pub(super) const fn is_field_put(opcode: Opcode) -> bool {
    matches!(
        opcode,
        Opcode::Iput
            | Opcode::IputWide
            | Opcode::IputObject
            | Opcode::IputBoolean
            | Opcode::IputByte
            | Opcode::IputChar
            | Opcode::IputShort
            | Opcode::Sput
            | Opcode::SputWide
            | Opcode::SputObject
            | Opcode::SputBoolean
            | Opcode::SputByte
            | Opcode::SputChar
            | Opcode::SputShort
    )
}

pub(super) const fn is_instance_field_get(opcode: Opcode) -> bool {
    matches!(
        opcode,
        Opcode::Iget
            | Opcode::IgetWide
            | Opcode::IgetObject
            | Opcode::IgetBoolean
            | Opcode::IgetByte
            | Opcode::IgetChar
            | Opcode::IgetShort
    )
}

pub(super) const fn is_instance_field_put(opcode: Opcode) -> bool {
    matches!(
        opcode,
        Opcode::Iput
            | Opcode::IputWide
            | Opcode::IputObject
            | Opcode::IputBoolean
            | Opcode::IputByte
            | Opcode::IputChar
            | Opcode::IputShort
    )
}

pub(super) const fn is_array_get(opcode: Opcode) -> bool {
    matches!(
        opcode,
        Opcode::Aget
            | Opcode::AgetWide
            | Opcode::AgetObject
            | Opcode::AgetBoolean
            | Opcode::AgetByte
            | Opcode::AgetChar
            | Opcode::AgetShort
    )
}

pub(super) const fn is_array_put(opcode: Opcode) -> bool {
    matches!(
        opcode,
        Opcode::Aput
            | Opcode::AputWide
            | Opcode::AputObject
            | Opcode::AputBoolean
            | Opcode::AputByte
            | Opcode::AputChar
            | Opcode::AputShort
    )
}

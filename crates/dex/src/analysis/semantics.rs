//! Complete register-effect classification for standard Dalvik opcodes.

use crate::instruction::{Instruction, InstructionData, Opcode, Operands};
use crate::{Error, Result};

use super::{InstructionSemantics, ProducedValue, RegisterOperand, ValueKind};

/// Computes typed register reads, writes, implicit results, and throw behavior.
///
/// Payload pseudo-instructions are reported as non-executable items without
/// register effects. Malformed manually constructed opcode/operand pairings are
/// rejected contextually even though decoded DEX files already satisfy them.
///
/// # Errors
///
/// Returns an error when an executable opcode has an incompatible operand form.
pub fn instruction_semantics(instruction: &Instruction) -> Result<InstructionSemantics> {
    let InstructionData::Operation { opcode, operands } = instruction.data() else {
        return Ok(InstructionSemantics::payload());
    };
    let mut semantics = InstructionSemantics::operation(may_throw(*opcode));
    classify(*opcode, operands, instruction.offset(), &mut semantics)?;
    Ok(semantics)
}

#[allow(clippy::match_same_arms, clippy::too_many_lines)]
fn classify(
    opcode: Opcode,
    operands: &Operands,
    offset: u32,
    semantics: &mut InstructionSemantics,
) -> Result<()> {
    use Opcode as O;
    use ValueKind as V;

    match opcode {
        O::Nop | O::ReturnVoid => require_none(operands),
        O::Goto | O::Goto16 | O::Goto32 => require_branch(operands),

        O::Move | O::MoveFrom16 | O::Move16 => move_registers(operands, V::Single, semantics),
        O::MoveWide | O::MoveWideFrom16 | O::MoveWide16 => {
            move_registers(operands, V::Wide, semantics)
        }
        O::MoveObject | O::MoveObjectFrom16 | O::MoveObject16 => {
            move_registers(operands, V::Reference, semantics)
        }

        O::MoveResult => write_one(operands, V::Single, semantics),
        O::MoveResultWide => write_one(operands, V::Wide, semantics),
        O::MoveResultObject | O::MoveException => write_one(operands, V::Reference, semantics),

        O::Return => read_one(operands, V::Single, semantics),
        O::ReturnWide => read_one(operands, V::Wide, semantics),
        O::ReturnObject | O::MonitorEnter | O::MonitorExit | O::Throw => {
            read_one(operands, V::Reference, semantics)
        }

        O::Const4 | O::Const16 | O::Const | O::ConstHigh16 => {
            write_one(operands, V::Single, semantics)
        }
        O::ConstWide16 | O::ConstWide32 | O::ConstWide | O::ConstWideHigh16 => {
            write_one(operands, V::Wide, semantics)
        }
        O::ConstString
        | O::ConstStringJumbo
        | O::ConstClass
        | O::NewInstance
        | O::ConstMethodHandle
        | O::ConstMethodType => write_one(operands, V::Reference, semantics),

        O::CheckCast => read_write_one(operands, V::Reference, semantics),
        O::InstanceOf => write_first_read_second(operands, V::Integer, V::Reference, semantics),
        O::ArrayLength => write_first_read_second(operands, V::Integer, V::Reference, semantics),
        O::NewArray => write_first_read_second(operands, V::Reference, V::Integer, semantics),

        O::FilledNewArray | O::FilledNewArrayRange => {
            read_words_and_produce(operands, ProducedValue::Reference, semantics)
        }
        O::FillArrayData => read_one(operands, V::Reference, semantics),
        O::PackedSwitch | O::SparseSwitch => read_one(operands, V::Integer, semantics),

        O::CmplFloat | O::CmpgFloat => {
            write_first_read_two(operands, V::Integer, V::Float, V::Float, semantics)
        }
        O::CmplDouble | O::CmpgDouble => {
            write_first_read_two(operands, V::Integer, V::Double, V::Double, semantics)
        }
        O::CmpLong => write_first_read_two(operands, V::Integer, V::Long, V::Long, semantics),

        O::IfEq | O::IfNe => read_two(operands, V::Single, V::Single, semantics),
        O::IfLt | O::IfGe | O::IfGt | O::IfLe => {
            read_two(operands, V::Integer, V::Integer, semantics)
        }
        O::IfEqz | O::IfNez => read_one(operands, V::Single, semantics),
        O::IfLtz | O::IfGez | O::IfGtz | O::IfLez => read_one(operands, V::Integer, semantics),

        O::Aget => aget(operands, V::Single, semantics),
        O::AgetWide => aget(operands, V::Wide, semantics),
        O::AgetObject => aget(operands, V::Reference, semantics),
        O::AgetBoolean | O::AgetByte | O::AgetChar | O::AgetShort => {
            aget(operands, V::Integer, semantics)
        }
        O::Aput => aput(operands, V::Single, semantics),
        O::AputWide => aput(operands, V::Wide, semantics),
        O::AputObject => aput(operands, V::Reference, semantics),
        O::AputBoolean | O::AputByte | O::AputChar | O::AputShort => {
            aput(operands, V::Integer, semantics)
        }

        O::Iget => iget(operands, V::Single, semantics),
        O::IgetWide => iget(operands, V::Wide, semantics),
        O::IgetObject => iget(operands, V::Reference, semantics),
        O::IgetBoolean | O::IgetByte | O::IgetChar | O::IgetShort => {
            iget(operands, V::Integer, semantics)
        }
        O::Iput => iput(operands, V::Single, semantics),
        O::IputWide => iput(operands, V::Wide, semantics),
        O::IputObject => iput(operands, V::Reference, semantics),
        O::IputBoolean | O::IputByte | O::IputChar | O::IputShort => {
            iput(operands, V::Integer, semantics)
        }

        O::Sget => write_one(operands, V::Single, semantics),
        O::SgetWide => write_one(operands, V::Wide, semantics),
        O::SgetObject => write_one(operands, V::Reference, semantics),
        O::SgetBoolean | O::SgetByte | O::SgetChar | O::SgetShort => {
            write_one(operands, V::Integer, semantics)
        }
        O::Sput => read_one(operands, V::Single, semantics),
        O::SputWide => read_one(operands, V::Wide, semantics),
        O::SputObject => read_one(operands, V::Reference, semantics),
        O::SputBoolean | O::SputByte | O::SputChar | O::SputShort => {
            read_one(operands, V::Integer, semantics)
        }

        O::InvokeVirtual
        | O::InvokeSuper
        | O::InvokeDirect
        | O::InvokeStatic
        | O::InvokeInterface
        | O::InvokeVirtualRange
        | O::InvokeSuperRange
        | O::InvokeDirectRange
        | O::InvokeStaticRange
        | O::InvokeInterfaceRange
        | O::InvokePolymorphic
        | O::InvokePolymorphicRange
        | O::InvokeCustom
        | O::InvokeCustomRange => {
            read_words_and_produce(operands, ProducedValue::Prototype, semantics)
        }

        O::NegInt | O::NotInt | O::IntToByte | O::IntToChar | O::IntToShort => {
            unary(operands, V::Integer, V::Integer, semantics)
        }
        O::NegLong | O::NotLong => unary(operands, V::Long, V::Long, semantics),
        O::NegFloat => unary(operands, V::Float, V::Float, semantics),
        O::NegDouble => unary(operands, V::Double, V::Double, semantics),
        O::IntToLong => unary(operands, V::Long, V::Integer, semantics),
        O::IntToFloat => unary(operands, V::Float, V::Integer, semantics),
        O::IntToDouble => unary(operands, V::Double, V::Integer, semantics),
        O::LongToInt => unary(operands, V::Integer, V::Long, semantics),
        O::LongToFloat => unary(operands, V::Float, V::Long, semantics),
        O::LongToDouble => unary(operands, V::Double, V::Long, semantics),
        O::FloatToInt => unary(operands, V::Integer, V::Float, semantics),
        O::FloatToLong => unary(operands, V::Long, V::Float, semantics),
        O::FloatToDouble => unary(operands, V::Double, V::Float, semantics),
        O::DoubleToInt => unary(operands, V::Integer, V::Double, semantics),
        O::DoubleToLong => unary(operands, V::Long, V::Double, semantics),
        O::DoubleToFloat => unary(operands, V::Float, V::Double, semantics),

        O::AddInt
        | O::SubInt
        | O::MulInt
        | O::DivInt
        | O::RemInt
        | O::AndInt
        | O::OrInt
        | O::XorInt
        | O::ShlInt
        | O::ShrInt
        | O::UshrInt => binary(operands, V::Integer, V::Integer, V::Integer, semantics),
        O::AddLong
        | O::SubLong
        | O::MulLong
        | O::DivLong
        | O::RemLong
        | O::AndLong
        | O::OrLong
        | O::XorLong => binary(operands, V::Long, V::Long, V::Long, semantics),
        O::ShlLong | O::ShrLong | O::UshrLong => {
            binary(operands, V::Long, V::Long, V::Integer, semantics)
        }
        O::AddFloat | O::SubFloat | O::MulFloat | O::DivFloat | O::RemFloat => {
            binary(operands, V::Float, V::Float, V::Float, semantics)
        }
        O::AddDouble | O::SubDouble | O::MulDouble | O::DivDouble | O::RemDouble => {
            binary(operands, V::Double, V::Double, V::Double, semantics)
        }

        O::AddInt2Addr
        | O::SubInt2Addr
        | O::MulInt2Addr
        | O::DivInt2Addr
        | O::RemInt2Addr
        | O::AndInt2Addr
        | O::OrInt2Addr
        | O::XorInt2Addr
        | O::ShlInt2Addr
        | O::ShrInt2Addr
        | O::UshrInt2Addr => two_address(operands, V::Integer, V::Integer, semantics),
        O::AddLong2Addr
        | O::SubLong2Addr
        | O::MulLong2Addr
        | O::DivLong2Addr
        | O::RemLong2Addr
        | O::AndLong2Addr
        | O::OrLong2Addr
        | O::XorLong2Addr => two_address(operands, V::Long, V::Long, semantics),
        O::ShlLong2Addr | O::ShrLong2Addr | O::UshrLong2Addr => {
            two_address(operands, V::Long, V::Integer, semantics)
        }
        O::AddFloat2Addr
        | O::SubFloat2Addr
        | O::MulFloat2Addr
        | O::DivFloat2Addr
        | O::RemFloat2Addr => two_address(operands, V::Float, V::Float, semantics),
        O::AddDouble2Addr
        | O::SubDouble2Addr
        | O::MulDouble2Addr
        | O::DivDouble2Addr
        | O::RemDouble2Addr => two_address(operands, V::Double, V::Double, semantics),

        O::AddIntLit16
        | O::RsubInt
        | O::MulIntLit16
        | O::DivIntLit16
        | O::RemIntLit16
        | O::AndIntLit16
        | O::OrIntLit16
        | O::XorIntLit16
        | O::AddIntLit8
        | O::RsubIntLit8
        | O::MulIntLit8
        | O::DivIntLit8
        | O::RemIntLit8
        | O::AndIntLit8
        | O::OrIntLit8
        | O::XorIntLit8
        | O::ShlIntLit8
        | O::ShrIntLit8
        | O::UshrIntLit8 => literal_binary(operands, semantics),
    }
    .map_err(|expected| operand_error(opcode, operands, offset, expected))
}

fn require_none(operands: &Operands) -> std::result::Result<(), &'static str> {
    matches!(operands, Operands::None)
        .then_some(())
        .ok_or("no register operands")
}

fn require_branch(operands: &Operands) -> std::result::Result<(), &'static str> {
    matches!(operands, Operands::Branch { .. })
        .then_some(())
        .ok_or("a branch target")
}

fn move_registers(
    operands: &Operands,
    kind: ValueKind,
    semantics: &mut InstructionSemantics,
) -> std::result::Result<(), &'static str> {
    let (destination, source) = two_registers(operands)?;
    semantics
        .reads
        .push(RegisterOperand::new(source, kind.clone()));
    semantics
        .writes
        .push(RegisterOperand::new(destination, kind));
    Ok(())
}

fn write_one(
    operands: &Operands,
    kind: ValueKind,
    semantics: &mut InstructionSemantics,
) -> std::result::Result<(), &'static str> {
    semantics
        .writes
        .push(RegisterOperand::new(first_register(operands)?, kind));
    Ok(())
}

fn read_one(
    operands: &Operands,
    kind: ValueKind,
    semantics: &mut InstructionSemantics,
) -> std::result::Result<(), &'static str> {
    semantics
        .reads
        .push(RegisterOperand::new(first_register(operands)?, kind));
    Ok(())
}

fn read_write_one(
    operands: &Operands,
    kind: ValueKind,
    semantics: &mut InstructionSemantics,
) -> std::result::Result<(), &'static str> {
    let register = first_register(operands)?;
    semantics
        .reads
        .push(RegisterOperand::new(register, kind.clone()));
    semantics.writes.push(RegisterOperand::new(register, kind));
    Ok(())
}

fn write_first_read_second(
    operands: &Operands,
    output: ValueKind,
    input: ValueKind,
    semantics: &mut InstructionSemantics,
) -> std::result::Result<(), &'static str> {
    let (destination, source) = two_registers(operands)?;
    semantics.reads.push(RegisterOperand::new(source, input));
    semantics
        .writes
        .push(RegisterOperand::new(destination, output));
    Ok(())
}

fn write_first_read_two(
    operands: &Operands,
    output: ValueKind,
    left: ValueKind,
    right: ValueKind,
    semantics: &mut InstructionSemantics,
) -> std::result::Result<(), &'static str> {
    let (destination, left_register, right_register) = three_registers(operands)?;
    semantics
        .reads
        .push(RegisterOperand::new(left_register, left));
    semantics
        .reads
        .push(RegisterOperand::new(right_register, right));
    semantics
        .writes
        .push(RegisterOperand::new(destination, output));
    Ok(())
}

fn read_two(
    operands: &Operands,
    first_kind: ValueKind,
    second_kind: ValueKind,
    semantics: &mut InstructionSemantics,
) -> std::result::Result<(), &'static str> {
    let (first, second) = two_registers(operands)?;
    semantics
        .reads
        .push(RegisterOperand::new(first, first_kind));
    semantics
        .reads
        .push(RegisterOperand::new(second, second_kind));
    Ok(())
}

fn aget(
    operands: &Operands,
    output: ValueKind,
    semantics: &mut InstructionSemantics,
) -> std::result::Result<(), &'static str> {
    let (destination, array, index) = three_registers(operands)?;
    semantics
        .reads
        .push(RegisterOperand::new(array, ValueKind::Reference));
    semantics
        .reads
        .push(RegisterOperand::new(index, ValueKind::Integer));
    semantics
        .writes
        .push(RegisterOperand::new(destination, output));
    Ok(())
}

fn aput(
    operands: &Operands,
    value: ValueKind,
    semantics: &mut InstructionSemantics,
) -> std::result::Result<(), &'static str> {
    let (value_register, array, index) = three_registers(operands)?;
    semantics
        .reads
        .push(RegisterOperand::new(value_register, value));
    semantics
        .reads
        .push(RegisterOperand::new(array, ValueKind::Reference));
    semantics
        .reads
        .push(RegisterOperand::new(index, ValueKind::Integer));
    Ok(())
}

fn iget(
    operands: &Operands,
    output: ValueKind,
    semantics: &mut InstructionSemantics,
) -> std::result::Result<(), &'static str> {
    write_first_read_second(operands, output, ValueKind::Reference, semantics)
}

fn iput(
    operands: &Operands,
    value: ValueKind,
    semantics: &mut InstructionSemantics,
) -> std::result::Result<(), &'static str> {
    read_two(operands, value, ValueKind::Reference, semantics)
}

fn unary(
    operands: &Operands,
    output: ValueKind,
    input: ValueKind,
    semantics: &mut InstructionSemantics,
) -> std::result::Result<(), &'static str> {
    write_first_read_second(operands, output, input, semantics)
}

fn binary(
    operands: &Operands,
    output: ValueKind,
    left: ValueKind,
    right: ValueKind,
    semantics: &mut InstructionSemantics,
) -> std::result::Result<(), &'static str> {
    write_first_read_two(operands, output, left, right, semantics)
}

fn two_address(
    operands: &Operands,
    first_kind: ValueKind,
    second_kind: ValueKind,
    semantics: &mut InstructionSemantics,
) -> std::result::Result<(), &'static str> {
    let (first, second) = two_registers(operands)?;
    semantics
        .reads
        .push(RegisterOperand::new(first, first_kind.clone()));
    semantics
        .reads
        .push(RegisterOperand::new(second, second_kind));
    semantics
        .writes
        .push(RegisterOperand::new(first, first_kind));
    Ok(())
}

fn literal_binary(
    operands: &Operands,
    semantics: &mut InstructionSemantics,
) -> std::result::Result<(), &'static str> {
    let Operands::RegistersLiteral { first, second, .. } = operands else {
        return Err("two registers and a literal");
    };
    semantics
        .reads
        .push(RegisterOperand::new(*second, ValueKind::Integer));
    semantics
        .writes
        .push(RegisterOperand::new(*first, ValueKind::Integer));
    Ok(())
}

fn read_invocation_words(
    operands: &Operands,
    semantics: &mut InstructionSemantics,
) -> std::result::Result<(), &'static str> {
    let registers = match operands {
        Operands::RegisterListIndex { registers, .. } => registers.clone(),
        Operands::RegisterRangeIndex { start, count, .. } => (0..u16::from(*count))
            .map(|delta| {
                start
                    .checked_add(delta)
                    .ok_or("a non-overflowing register range")
            })
            .collect::<std::result::Result<Vec<_>, _>>()?,
        _ => return Err("a register list or range"),
    };
    semantics.reads.extend(
        registers
            .into_iter()
            .map(|register| RegisterOperand::new(register, ValueKind::Single)),
    );
    Ok(())
}

fn read_words_and_produce(
    operands: &Operands,
    produced: ProducedValue,
    semantics: &mut InstructionSemantics,
) -> std::result::Result<(), &'static str> {
    read_invocation_words(operands, semantics)?;
    semantics.produced = Some(produced);
    Ok(())
}

fn first_register(operands: &Operands) -> std::result::Result<u16, &'static str> {
    match operands {
        Operands::Register(register)
        | Operands::RegisterLiteral { register, .. }
        | Operands::RegisterBranch { register, .. }
        | Operands::RegisterIndex { register, .. } => Ok(*register),
        _ => Err("one register"),
    }
}

fn two_registers(operands: &Operands) -> std::result::Result<(u16, u16), &'static str> {
    match operands {
        Operands::Registers { first, second }
        | Operands::RegistersLiteral { first, second, .. }
        | Operands::RegistersBranch { first, second, .. }
        | Operands::RegistersIndex { first, second, .. } => Ok((*first, *second)),
        _ => Err("two registers"),
    }
}

fn three_registers(operands: &Operands) -> std::result::Result<(u16, u16, u16), &'static str> {
    let Operands::ThreeRegisters {
        first,
        second,
        third,
    } = operands
    else {
        return Err("three registers");
    };
    Ok((*first, *second, *third))
}

fn operand_error(opcode: Opcode, operands: &Operands, offset: u32, expected: &str) -> Error {
    Error::invalid_instruction(
        offset,
        format!(
            "{} expects {expected}, found {operands:?}",
            opcode.mnemonic()
        ),
    )
}

#[allow(clippy::match_same_arms)]
const fn may_throw(opcode: Opcode) -> bool {
    use Opcode as O;
    matches!(
        opcode,
        O::ConstString
            | O::ConstStringJumbo
            | O::ConstClass
            | O::MonitorEnter
            | O::MonitorExit
            | O::CheckCast
            | O::InstanceOf
            | O::ArrayLength
            | O::NewInstance
            | O::NewArray
            | O::FilledNewArray
            | O::FilledNewArrayRange
            | O::FillArrayData
            | O::Throw
            | O::Aget
            | O::AgetWide
            | O::AgetObject
            | O::AgetBoolean
            | O::AgetByte
            | O::AgetChar
            | O::AgetShort
            | O::Aput
            | O::AputWide
            | O::AputObject
            | O::AputBoolean
            | O::AputByte
            | O::AputChar
            | O::AputShort
            | O::Iget
            | O::IgetWide
            | O::IgetObject
            | O::IgetBoolean
            | O::IgetByte
            | O::IgetChar
            | O::IgetShort
            | O::Iput
            | O::IputWide
            | O::IputObject
            | O::IputBoolean
            | O::IputByte
            | O::IputChar
            | O::IputShort
            | O::Sget
            | O::SgetWide
            | O::SgetObject
            | O::SgetBoolean
            | O::SgetByte
            | O::SgetChar
            | O::SgetShort
            | O::Sput
            | O::SputWide
            | O::SputObject
            | O::SputBoolean
            | O::SputByte
            | O::SputChar
            | O::SputShort
            | O::InvokeVirtual
            | O::InvokeSuper
            | O::InvokeDirect
            | O::InvokeStatic
            | O::InvokeInterface
            | O::InvokeVirtualRange
            | O::InvokeSuperRange
            | O::InvokeDirectRange
            | O::InvokeStaticRange
            | O::InvokeInterfaceRange
            | O::DivInt
            | O::RemInt
            | O::DivLong
            | O::RemLong
            | O::DivInt2Addr
            | O::RemInt2Addr
            | O::DivLong2Addr
            | O::RemLong2Addr
            | O::DivIntLit16
            | O::RemIntLit16
            | O::DivIntLit8
            | O::RemIntLit8
            | O::InvokePolymorphic
            | O::InvokePolymorphicRange
            | O::InvokeCustom
            | O::InvokeCustomRange
            | O::ConstMethodHandle
            | O::ConstMethodType
    )
}

//! Checked conversion between native JVM instructions and JVM LLIL.

use crate::bytecode::{self, Instruction as NativeInstruction, Opcode, Operand};
use crate::{Error, Result};

use super::{
    ArithmeticOperator as Arithmetic, ArrayAccess, ArrayElementKind as Element,
    BitwiseOperator as Bitwise, BranchCondition, Comparison, Constant, ConstantWidth, Conversion,
    FieldAccess, Instruction, Intrinsic, Invocation, LocalAccess, MonitorAction, NativeEncoding,
    Operation, Relation, ShiftOperator as Shift, StackOperation as Stack, Switch, SwitchCase,
    ValueKind,
};

/// Converts a checked native JVM instruction stream into JVM LLIL.
///
/// # Errors
///
/// Returns an error when the native stream has invalid layout, operands, or
/// control-flow targets.
pub fn lift_instructions(native: &[NativeInstruction]) -> Result<Vec<Instruction>> {
    bytecode::encode(native)?;
    lift_decoded_instructions(native)
}

/// Classifies a stream already checked by the native decoder.
pub(crate) fn lift_decoded_instructions(native: &[NativeInstruction]) -> Result<Vec<Instruction>> {
    native.iter().map(Instruction::from_native).collect()
}

/// Converts JVM LLIL back into a checked native instruction stream.
///
/// Each instruction's normalized operation must still agree with its exact
/// encoding sidecar. The complete stream is then passed through the native JVM
/// encoder, which verifies layout and control-flow targets.
///
/// # Errors
///
/// Returns an error for stale encoding provenance or an invalid native stream.
pub fn lower_instructions(llil: &[Instruction]) -> Result<Vec<NativeInstruction>> {
    let native = llil
        .iter()
        .map(Instruction::to_native)
        .collect::<Result<Vec<_>>>()?;
    bytecode::encode(&native)?;
    Ok(native)
}

impl NativeEncoding {
    /// Captures the exact encoding of one decoded native instruction.
    #[must_use]
    pub fn from_native(native: &NativeInstruction) -> Self {
        Self {
            opcode: native.opcode,
            wide: native.wide,
            size: native.size,
            operand: native.operand.clone(),
        }
    }
}

impl Instruction {
    /// Lifts one native JVM instruction into normalized LLIL.
    ///
    /// # Errors
    ///
    /// Returns an error if the supplied opcode/operand pair is not a valid
    /// decoded JVM instruction shape. Stream-level targets are checked by
    /// [`lift_instructions`].
    pub fn from_native(native: &NativeInstruction) -> Result<Self> {
        Ok(Self {
            offset: native.offset,
            operation: classify(native)?,
            encoding: NativeEncoding::from_native(native),
        })
    }

    /// Reconstructs one native JVM instruction from its exact encoding.
    ///
    /// # Errors
    ///
    /// Returns an error when the semantic operation no longer agrees with the
    /// encoding sidecar. Stream-level layout is checked by [`lower_instructions`].
    pub fn to_native(&self) -> Result<NativeInstruction> {
        let native = NativeInstruction {
            offset: self.offset,
            opcode: self.encoding.opcode,
            wide: self.encoding.wide,
            size: self.encoding.size,
            operand: self.encoding.operand.clone(),
        };
        let encoded_operation = classify(&native)?;
        if encoded_operation != self.operation {
            return Err(Error::invalid_bytecode(
                self.offset,
                format!(
                    "LLIL operation {:?} disagrees with native encoding {:?}",
                    self.operation, self.encoding
                ),
            ));
        }
        Ok(native)
    }

    /// Verifies that normalized semantics and native provenance still agree.
    ///
    /// # Errors
    ///
    /// Returns an error when the encoding represents a different operation.
    pub fn verify(&self) -> Result<()> {
        self.to_native().map(drop)
    }
}

#[allow(clippy::too_many_lines)]
fn classify(instruction: &NativeInstruction) -> Result<Operation> {
    use Opcode as O;

    let operation = match instruction.opcode {
        O::Nop => no_operand(instruction, Operation::Nop)?,
        O::AConstNull => no_operand(instruction, Operation::Constant(Constant::Null))?,
        O::IConstM1 => integer_constant(instruction, -1)?,
        O::IConst0 => integer_constant(instruction, 0)?,
        O::IConst1 => integer_constant(instruction, 1)?,
        O::IConst2 => integer_constant(instruction, 2)?,
        O::IConst3 => integer_constant(instruction, 3)?,
        O::IConst4 => integer_constant(instruction, 4)?,
        O::IConst5 => integer_constant(instruction, 5)?,
        O::LConst0 => no_operand(instruction, Operation::Constant(Constant::Long(0)))?,
        O::LConst1 => no_operand(instruction, Operation::Constant(Constant::Long(1)))?,
        O::FConst0 => float_constant(instruction, 0.0)?,
        O::FConst1 => float_constant(instruction, 1.0)?,
        O::FConst2 => float_constant(instruction, 2.0)?,
        O::DConst0 => double_constant(instruction, 0.0)?,
        O::DConst1 => double_constant(instruction, 1.0)?,
        O::BiPush => Operation::Constant(Constant::Integer(i32::from(byte(instruction)?))),
        O::SiPush => Operation::Constant(Constant::Integer(i32::from(short(instruction)?))),
        O::Ldc | O::LdcW => Operation::Constant(Constant::Pool {
            index: constant_index(instruction)?,
            width: ConstantWidth::Single,
        }),
        O::Ldc2W => Operation::Constant(Constant::Pool {
            index: constant_index(instruction)?,
            width: ConstantWidth::Double,
        }),
        O::ILoad => local(instruction, LocalAccess::Load, ValueKind::Integer)?,
        O::LLoad => local(instruction, LocalAccess::Load, ValueKind::Long)?,
        O::FLoad => local(instruction, LocalAccess::Load, ValueKind::Float)?,
        O::DLoad => local(instruction, LocalAccess::Load, ValueKind::Double)?,
        O::ALoad => local(instruction, LocalAccess::Load, ValueKind::Reference)?,
        O::ILoad0 | O::ILoad1 | O::ILoad2 | O::ILoad3 => implicit_local(
            instruction,
            LocalAccess::Load,
            ValueKind::Integer,
            O::ILoad0,
        )?,
        O::LLoad0 | O::LLoad1 | O::LLoad2 | O::LLoad3 => {
            implicit_local(instruction, LocalAccess::Load, ValueKind::Long, O::LLoad0)?
        }
        O::FLoad0 | O::FLoad1 | O::FLoad2 | O::FLoad3 => {
            implicit_local(instruction, LocalAccess::Load, ValueKind::Float, O::FLoad0)?
        }
        O::DLoad0 | O::DLoad1 | O::DLoad2 | O::DLoad3 => {
            implicit_local(instruction, LocalAccess::Load, ValueKind::Double, O::DLoad0)?
        }
        O::ALoad0 | O::ALoad1 | O::ALoad2 | O::ALoad3 => implicit_local(
            instruction,
            LocalAccess::Load,
            ValueKind::Reference,
            O::ALoad0,
        )?,
        O::IALoad => array(instruction, ArrayAccess::Load, Element::Integer)?,
        O::LALoad => array(instruction, ArrayAccess::Load, Element::Long)?,
        O::FALoad => array(instruction, ArrayAccess::Load, Element::Float)?,
        O::DALoad => array(instruction, ArrayAccess::Load, Element::Double)?,
        O::AALoad => array(instruction, ArrayAccess::Load, Element::Reference)?,
        O::BALoad => array(instruction, ArrayAccess::Load, Element::ByteOrBoolean)?,
        O::CALoad => array(instruction, ArrayAccess::Load, Element::Char)?,
        O::SALoad => array(instruction, ArrayAccess::Load, Element::Short)?,
        O::IStore => local(instruction, LocalAccess::Store, ValueKind::Integer)?,
        O::LStore => local(instruction, LocalAccess::Store, ValueKind::Long)?,
        O::FStore => local(instruction, LocalAccess::Store, ValueKind::Float)?,
        O::DStore => local(instruction, LocalAccess::Store, ValueKind::Double)?,
        O::AStore => local(
            instruction,
            LocalAccess::Store,
            ValueKind::ReferenceOrReturnAddress,
        )?,
        O::IStore0 | O::IStore1 | O::IStore2 | O::IStore3 => implicit_local(
            instruction,
            LocalAccess::Store,
            ValueKind::Integer,
            O::IStore0,
        )?,
        O::LStore0 | O::LStore1 | O::LStore2 | O::LStore3 => {
            implicit_local(instruction, LocalAccess::Store, ValueKind::Long, O::LStore0)?
        }
        O::FStore0 | O::FStore1 | O::FStore2 | O::FStore3 => implicit_local(
            instruction,
            LocalAccess::Store,
            ValueKind::Float,
            O::FStore0,
        )?,
        O::DStore0 | O::DStore1 | O::DStore2 | O::DStore3 => implicit_local(
            instruction,
            LocalAccess::Store,
            ValueKind::Double,
            O::DStore0,
        )?,
        O::AStore0 | O::AStore1 | O::AStore2 | O::AStore3 => implicit_local(
            instruction,
            LocalAccess::Store,
            ValueKind::ReferenceOrReturnAddress,
            O::AStore0,
        )?,
        O::IAStore => array(instruction, ArrayAccess::Store, Element::Integer)?,
        O::LAStore => array(instruction, ArrayAccess::Store, Element::Long)?,
        O::FAStore => array(instruction, ArrayAccess::Store, Element::Float)?,
        O::DAStore => array(instruction, ArrayAccess::Store, Element::Double)?,
        O::AAStore => array(instruction, ArrayAccess::Store, Element::Reference)?,
        O::BAStore => array(instruction, ArrayAccess::Store, Element::ByteOrBoolean)?,
        O::CAStore => array(instruction, ArrayAccess::Store, Element::Char)?,
        O::SAStore => array(instruction, ArrayAccess::Store, Element::Short)?,
        O::Pop => stack(instruction, Stack::Pop)?,
        O::Pop2 => stack(instruction, Stack::Pop2)?,
        O::Dup => stack(instruction, Stack::Dup)?,
        O::DupX1 => stack(instruction, Stack::DupX1)?,
        O::DupX2 => stack(instruction, Stack::DupX2)?,
        O::Dup2 => stack(instruction, Stack::Dup2)?,
        O::Dup2X1 => stack(instruction, Stack::Dup2X1)?,
        O::Dup2X2 => stack(instruction, Stack::Dup2X2)?,
        O::Swap => stack(instruction, Stack::Swap)?,
        O::IAdd => arithmetic(instruction, Arithmetic::Add, ValueKind::Integer)?,
        O::LAdd => arithmetic(instruction, Arithmetic::Add, ValueKind::Long)?,
        O::FAdd => arithmetic(instruction, Arithmetic::Add, ValueKind::Float)?,
        O::DAdd => arithmetic(instruction, Arithmetic::Add, ValueKind::Double)?,
        O::ISub => arithmetic(instruction, Arithmetic::Subtract, ValueKind::Integer)?,
        O::LSub => arithmetic(instruction, Arithmetic::Subtract, ValueKind::Long)?,
        O::FSub => arithmetic(instruction, Arithmetic::Subtract, ValueKind::Float)?,
        O::DSub => arithmetic(instruction, Arithmetic::Subtract, ValueKind::Double)?,
        O::IMul => arithmetic(instruction, Arithmetic::Multiply, ValueKind::Integer)?,
        O::LMul => arithmetic(instruction, Arithmetic::Multiply, ValueKind::Long)?,
        O::FMul => arithmetic(instruction, Arithmetic::Multiply, ValueKind::Float)?,
        O::DMul => arithmetic(instruction, Arithmetic::Multiply, ValueKind::Double)?,
        O::IDiv => arithmetic(instruction, Arithmetic::Divide, ValueKind::Integer)?,
        O::LDiv => arithmetic(instruction, Arithmetic::Divide, ValueKind::Long)?,
        O::FDiv => arithmetic(instruction, Arithmetic::Divide, ValueKind::Float)?,
        O::DDiv => arithmetic(instruction, Arithmetic::Divide, ValueKind::Double)?,
        O::IRem => arithmetic(instruction, Arithmetic::Remainder, ValueKind::Integer)?,
        O::LRem => arithmetic(instruction, Arithmetic::Remainder, ValueKind::Long)?,
        O::FRem => arithmetic(instruction, Arithmetic::Remainder, ValueKind::Float)?,
        O::DRem => arithmetic(instruction, Arithmetic::Remainder, ValueKind::Double)?,
        O::INeg => negate(instruction, ValueKind::Integer)?,
        O::LNeg => negate(instruction, ValueKind::Long)?,
        O::FNeg => negate(instruction, ValueKind::Float)?,
        O::DNeg => negate(instruction, ValueKind::Double)?,
        O::IShl => shift(instruction, Shift::Left, ValueKind::Integer)?,
        O::LShl => shift(instruction, Shift::Left, ValueKind::Long)?,
        O::IShr => shift(instruction, Shift::Right, ValueKind::Integer)?,
        O::LShr => shift(instruction, Shift::Right, ValueKind::Long)?,
        O::IUShr => shift(instruction, Shift::UnsignedRight, ValueKind::Integer)?,
        O::LUShr => shift(instruction, Shift::UnsignedRight, ValueKind::Long)?,
        O::IAnd => bitwise(instruction, Bitwise::And, ValueKind::Integer)?,
        O::LAnd => bitwise(instruction, Bitwise::And, ValueKind::Long)?,
        O::IOr => bitwise(instruction, Bitwise::Or, ValueKind::Integer)?,
        O::LOr => bitwise(instruction, Bitwise::Or, ValueKind::Long)?,
        O::IXor => bitwise(instruction, Bitwise::Xor, ValueKind::Integer)?,
        O::LXor => bitwise(instruction, Bitwise::Xor, ValueKind::Long)?,
        O::IInc => {
            let (index, amount) = increment(instruction)?;
            Operation::IncrementLocal { index, amount }
        }
        O::I2L => convert(instruction, Conversion::IntToLong)?,
        O::I2F => convert(instruction, Conversion::IntToFloat)?,
        O::I2D => convert(instruction, Conversion::IntToDouble)?,
        O::L2I => convert(instruction, Conversion::LongToInt)?,
        O::L2F => convert(instruction, Conversion::LongToFloat)?,
        O::L2D => convert(instruction, Conversion::LongToDouble)?,
        O::F2I => convert(instruction, Conversion::FloatToInt)?,
        O::F2L => convert(instruction, Conversion::FloatToLong)?,
        O::F2D => convert(instruction, Conversion::FloatToDouble)?,
        O::D2I => convert(instruction, Conversion::DoubleToInt)?,
        O::D2L => convert(instruction, Conversion::DoubleToLong)?,
        O::D2F => convert(instruction, Conversion::DoubleToFloat)?,
        O::I2B => convert(instruction, Conversion::IntToByte)?,
        O::I2C => convert(instruction, Conversion::IntToChar)?,
        O::I2S => convert(instruction, Conversion::IntToShort)?,
        O::LCmp => compare(instruction, Comparison::Long)?,
        O::FCmpL => compare(instruction, Comparison::FloatNanLow)?,
        O::FCmpG => compare(instruction, Comparison::FloatNanHigh)?,
        O::DCmpL => compare(instruction, Comparison::DoubleNanLow)?,
        O::DCmpG => compare(instruction, Comparison::DoubleNanHigh)?,
        O::IfEq => branch(instruction, BranchCondition::IntegerZero(Relation::Equal))?,
        O::IfNe => branch(
            instruction,
            BranchCondition::IntegerZero(Relation::NotEqual),
        )?,
        O::IfLt => branch(instruction, BranchCondition::IntegerZero(Relation::Less))?,
        O::IfGe => branch(
            instruction,
            BranchCondition::IntegerZero(Relation::GreaterOrEqual),
        )?,
        O::IfGt => branch(instruction, BranchCondition::IntegerZero(Relation::Greater))?,
        O::IfLe => branch(
            instruction,
            BranchCondition::IntegerZero(Relation::LessOrEqual),
        )?,
        O::IfICmpEq => branch(instruction, BranchCondition::IntegerPair(Relation::Equal))?,
        O::IfICmpNe => branch(
            instruction,
            BranchCondition::IntegerPair(Relation::NotEqual),
        )?,
        O::IfICmpLt => branch(instruction, BranchCondition::IntegerPair(Relation::Less))?,
        O::IfICmpGe => branch(
            instruction,
            BranchCondition::IntegerPair(Relation::GreaterOrEqual),
        )?,
        O::IfICmpGt => branch(instruction, BranchCondition::IntegerPair(Relation::Greater))?,
        O::IfICmpLe => branch(
            instruction,
            BranchCondition::IntegerPair(Relation::LessOrEqual),
        )?,
        O::IfACmpEq => branch(instruction, BranchCondition::ReferencePair(Relation::Equal))?,
        O::IfACmpNe => branch(
            instruction,
            BranchCondition::ReferencePair(Relation::NotEqual),
        )?,
        O::Goto | O::GotoW => Operation::Jump {
            target: branch_target(instruction)?,
        },
        O::Jsr | O::JsrW => Operation::SubroutineCall {
            target: branch_target(instruction)?,
        },
        O::Ret => Operation::SubroutineReturn {
            local: local_index(instruction)?,
        },
        O::TableSwitch => table_switch(instruction)?,
        O::LookupSwitch => lookup_switch(instruction)?,
        O::IReturn => method_return(instruction, Some(ValueKind::Integer))?,
        O::LReturn => method_return(instruction, Some(ValueKind::Long))?,
        O::FReturn => method_return(instruction, Some(ValueKind::Float))?,
        O::DReturn => method_return(instruction, Some(ValueKind::Double))?,
        O::AReturn => method_return(instruction, Some(ValueKind::Reference))?,
        O::Return => method_return(instruction, None)?,
        O::GetStatic => field(instruction, FieldAccess::GetStatic)?,
        O::PutStatic => field(instruction, FieldAccess::PutStatic)?,
        O::GetField => field(instruction, FieldAccess::GetInstance)?,
        O::PutField => field(instruction, FieldAccess::PutInstance)?,
        O::InvokeVirtual => invoke_constant(instruction, Invocation::Virtual)?,
        O::InvokeSpecial => invoke_constant(instruction, Invocation::Special)?,
        O::InvokeStatic => invoke_constant(instruction, Invocation::Static)?,
        O::InvokeInterface => match instruction.operand {
            Operand::InvokeInterface { index, .. } if !instruction.wide => Operation::Invoke {
                kind: Invocation::Interface,
                index,
            },
            _ => return Err(invalid_operand(instruction)),
        },
        O::InvokeDynamic => match instruction.operand {
            Operand::InvokeDynamic(index) if !instruction.wide => Operation::Invoke {
                kind: Invocation::Dynamic,
                index,
            },
            _ => return Err(invalid_operand(instruction)),
        },
        O::New => Operation::NewObject {
            index: constant_index(instruction)?,
        },
        O::NewArray => match instruction.operand {
            Operand::ArrayType(array_type) if !instruction.wide => {
                Operation::NewPrimitiveArray(array_type)
            }
            _ => return Err(invalid_operand(instruction)),
        },
        O::ANewArray => Operation::NewReferenceArray {
            index: constant_index(instruction)?,
        },
        O::ArrayLength => no_operand(instruction, Operation::ArrayLength)?,
        O::AThrow => no_operand(instruction, Operation::Throw)?,
        O::CheckCast => Operation::CheckCast {
            index: constant_index(instruction)?,
        },
        O::InstanceOf => Operation::InstanceOf {
            index: constant_index(instruction)?,
        },
        O::MonitorEnter => no_operand(instruction, Operation::Monitor(MonitorAction::Enter))?,
        O::MonitorExit => no_operand(instruction, Operation::Monitor(MonitorAction::Exit))?,
        O::Wide => return Err(invalid_operand(instruction)),
        O::MultiANewArray => match instruction.operand {
            Operand::MultiArray { index, dimensions } if !instruction.wide => {
                Operation::NewMultiArray { index, dimensions }
            }
            _ => return Err(invalid_operand(instruction)),
        },
        O::IfNull => branch(instruction, BranchCondition::ReferenceNull(Relation::Equal))?,
        O::IfNonNull => branch(
            instruction,
            BranchCondition::ReferenceNull(Relation::NotEqual),
        )?,
        O::Breakpoint => intrinsic(instruction, Intrinsic::Breakpoint)?,
        O::ImpDep1 => intrinsic(instruction, Intrinsic::ImplementationDependent1)?,
        O::ImpDep2 => intrinsic(instruction, Intrinsic::ImplementationDependent2)?,
    };
    Ok(operation)
}

fn no_operand(instruction: &NativeInstruction, operation: Operation) -> Result<Operation> {
    if matches!(instruction.operand, Operand::None) && !instruction.wide {
        Ok(operation)
    } else {
        Err(invalid_operand(instruction))
    }
}

fn integer_constant(instruction: &NativeInstruction, value: i32) -> Result<Operation> {
    no_operand(instruction, Operation::Constant(Constant::Integer(value)))
}

fn float_constant(instruction: &NativeInstruction, value: f32) -> Result<Operation> {
    no_operand(
        instruction,
        Operation::Constant(Constant::Float(value.to_bits())),
    )
}

fn double_constant(instruction: &NativeInstruction, value: f64) -> Result<Operation> {
    no_operand(
        instruction,
        Operation::Constant(Constant::Double(value.to_bits())),
    )
}

fn byte(instruction: &NativeInstruction) -> Result<i8> {
    match instruction.operand {
        Operand::Byte(value) if !instruction.wide => Ok(value),
        _ => Err(invalid_operand(instruction)),
    }
}

fn short(instruction: &NativeInstruction) -> Result<i16> {
    match instruction.operand {
        Operand::Short(value) if !instruction.wide => Ok(value),
        _ => Err(invalid_operand(instruction)),
    }
}

fn constant_index(instruction: &NativeInstruction) -> Result<u16> {
    match instruction.operand {
        Operand::Constant(index) if !instruction.wide => Ok(index),
        _ => Err(invalid_operand(instruction)),
    }
}

fn local_index(instruction: &NativeInstruction) -> Result<u16> {
    match instruction.operand {
        Operand::Local(index) => Ok(index),
        _ => Err(invalid_operand(instruction)),
    }
}

fn local(
    instruction: &NativeInstruction,
    access: LocalAccess,
    kind: ValueKind,
) -> Result<Operation> {
    Ok(Operation::Local {
        access,
        kind,
        index: local_index(instruction)?,
    })
}

fn implicit_local(
    instruction: &NativeInstruction,
    access: LocalAccess,
    kind: ValueKind,
    first: Opcode,
) -> Result<Operation> {
    let operation = Operation::Local {
        access,
        kind,
        index: u16::from(instruction.opcode.byte() - first.byte()),
    };
    no_operand(instruction, operation)
}

fn increment(instruction: &NativeInstruction) -> Result<(u16, i16)> {
    match instruction.operand {
        Operand::Increment { index, value } => Ok((index, value)),
        _ => Err(invalid_operand(instruction)),
    }
}

fn array(
    instruction: &NativeInstruction,
    access: ArrayAccess,
    element: Element,
) -> Result<Operation> {
    no_operand(instruction, Operation::Array { access, element })
}

fn stack(instruction: &NativeInstruction, operation: Stack) -> Result<Operation> {
    no_operand(instruction, Operation::Stack(operation))
}

fn arithmetic(
    instruction: &NativeInstruction,
    operator: Arithmetic,
    kind: ValueKind,
) -> Result<Operation> {
    no_operand(instruction, Operation::Arithmetic { operator, kind })
}

fn negate(instruction: &NativeInstruction, kind: ValueKind) -> Result<Operation> {
    no_operand(instruction, Operation::Negate(kind))
}

fn shift(instruction: &NativeInstruction, operator: Shift, kind: ValueKind) -> Result<Operation> {
    no_operand(instruction, Operation::Shift { operator, kind })
}

fn bitwise(
    instruction: &NativeInstruction,
    operator: Bitwise,
    kind: ValueKind,
) -> Result<Operation> {
    no_operand(instruction, Operation::Bitwise { operator, kind })
}

fn convert(instruction: &NativeInstruction, conversion: Conversion) -> Result<Operation> {
    no_operand(instruction, Operation::Convert(conversion))
}

fn compare(instruction: &NativeInstruction, comparison: Comparison) -> Result<Operation> {
    no_operand(instruction, Operation::Compare(comparison))
}

fn branch_target(instruction: &NativeInstruction) -> Result<i32> {
    match instruction.operand {
        Operand::Branch(target) if !instruction.wide => Ok(target),
        _ => Err(invalid_operand(instruction)),
    }
}

fn branch(instruction: &NativeInstruction, condition: BranchCondition) -> Result<Operation> {
    Ok(Operation::Branch {
        condition,
        target: branch_target(instruction)?,
    })
}

fn table_switch(instruction: &NativeInstruction) -> Result<Operation> {
    let Operand::TableSwitch {
        default,
        low,
        targets,
    } = &instruction.operand
    else {
        return Err(invalid_operand(instruction));
    };
    if instruction.wide {
        return Err(invalid_operand(instruction));
    }

    let cases = targets
        .iter()
        .enumerate()
        .map(|(delta, target)| {
            let delta = i32::try_from(delta).map_err(|_| invalid_operand(instruction))?;
            let key = low
                .checked_add(delta)
                .ok_or_else(|| invalid_operand(instruction))?;
            Ok(SwitchCase {
                key,
                target: *target,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(Operation::Switch(Switch {
        default: *default,
        cases,
    }))
}

fn lookup_switch(instruction: &NativeInstruction) -> Result<Operation> {
    let Operand::LookupSwitch { default, pairs } = &instruction.operand else {
        return Err(invalid_operand(instruction));
    };
    if instruction.wide {
        return Err(invalid_operand(instruction));
    }

    Ok(Operation::Switch(Switch {
        default: *default,
        cases: pairs
            .iter()
            .map(|(key, target)| SwitchCase {
                key: *key,
                target: *target,
            })
            .collect(),
    }))
}

fn method_return(instruction: &NativeInstruction, kind: Option<ValueKind>) -> Result<Operation> {
    no_operand(instruction, Operation::Return(kind))
}

fn field(instruction: &NativeInstruction, access: FieldAccess) -> Result<Operation> {
    Ok(Operation::Field {
        access,
        index: constant_index(instruction)?,
    })
}

fn invoke_constant(instruction: &NativeInstruction, kind: Invocation) -> Result<Operation> {
    Ok(Operation::Invoke {
        kind,
        index: constant_index(instruction)?,
    })
}

fn intrinsic(instruction: &NativeInstruction, intrinsic: Intrinsic) -> Result<Operation> {
    no_operand(instruction, Operation::Intrinsic(intrinsic))
}

fn invalid_operand(instruction: &NativeInstruction) -> Error {
    Error::invalid_bytecode(
        instruction.offset,
        format!(
            "{} has incompatible LLIL encoding operand {:?}",
            instruction.opcode.mnemonic(),
            instruction.operand
        ),
    )
}

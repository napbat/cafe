//! Checked conversion between native Dalvik instructions and DEX LLIL.

use crate::analysis::{InstructionSemantics, ValueKind, instruction_semantics};
use crate::instruction::{
    self, IndexKind, Instruction as NativeInstruction, InstructionData, Opcode,
    Operands as NativeOperands,
};
use crate::{Error, Result};

use super::{
    ArithmeticOperator as Arithmetic, ArrayAccess, ArrayElementKind as Element, Comparison,
    ConstantKind, Conversion, FieldAccess, Instruction, InstructionKind, Invocation, MonitorAction,
    NativeEncoding, Operand, Operation, OperationKind, Payload, Relation, UnaryOperator as Unary,
};

/// Converts a checked native Dalvik instruction stream into DEX LLIL.
///
/// # Errors
///
/// Returns an error when native layout, operands, branches, or payload links are
/// not encodable.
pub fn lift_instructions(native: &[NativeInstruction]) -> Result<Vec<Instruction>> {
    instruction::encode(native)?;
    native.iter().map(Instruction::from_native).collect()
}

/// Converts DEX LLIL back into a checked native instruction stream.
///
/// # Errors
///
/// Returns an error for stale semantic/encoding pairs or an invalid native
/// layout, branch, operand, or payload relationship.
pub fn lower_instructions(llil: &[Instruction]) -> Result<Vec<NativeInstruction>> {
    let native = llil
        .iter()
        .map(Instruction::to_native)
        .collect::<Result<Vec<_>>>()?;
    instruction::encode(&native)?;
    Ok(native)
}

impl NativeEncoding {
    /// Captures the exact encoding of one native DEX stream item.
    #[must_use]
    pub fn from_native(native: &NativeInstruction) -> Self {
        Self {
            data: native.data().clone(),
        }
    }
}

impl Instruction {
    /// Lifts one native DEX instruction or payload into LLIL.
    ///
    /// # Errors
    ///
    /// Returns an error if its opcode/operand shape has invalid Dalvik semantics.
    pub fn from_native(native: &NativeInstruction) -> Result<Self> {
        Ok(Self {
            offset: native.offset(),
            kind: classify(native)?,
            encoding: NativeEncoding::from_native(native),
        })
    }

    /// Reconstructs one native DEX stream item from its exact encoding.
    ///
    /// # Errors
    ///
    /// Returns an error when normalized semantics no longer agree with the
    /// retained encoding sidecar.
    pub fn to_native(&self) -> Result<NativeInstruction> {
        let native = native_instruction(self.offset, &self.encoding.data);
        let encoded_kind = classify(&native)?;
        if encoded_kind != self.kind {
            return Err(Error::invalid_instruction(
                self.offset,
                format!(
                    "DEX LLIL item {:?} disagrees with native encoding {:?}",
                    self.kind, self.encoding
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

fn native_instruction(offset: u32, data: &InstructionData) -> NativeInstruction {
    match data {
        InstructionData::Operation { opcode, operands } => {
            NativeInstruction::operation(offset, *opcode, operands.clone())
        }
        InstructionData::PackedSwitchPayload(payload) => {
            NativeInstruction::packed_switch(offset, payload.clone())
        }
        InstructionData::SparseSwitchPayload(payload) => {
            NativeInstruction::sparse_switch(offset, payload.clone())
        }
        InstructionData::ArrayDataPayload(payload) => {
            NativeInstruction::array_data(offset, payload.clone())
        }
    }
}

fn classify(native: &NativeInstruction) -> Result<InstructionKind> {
    let kind = match native.data() {
        InstructionData::Operation { opcode, operands } => {
            let semantics = instruction_semantics(native)?;
            InstructionKind::Operation(Operation {
                kind: operation_kind(*opcode),
                operands: normalized_operands(native.offset(), *opcode, operands, &semantics)?,
                semantics,
            })
        }
        InstructionData::PackedSwitchPayload(payload) => {
            InstructionKind::Payload(Payload::PackedSwitch(payload.clone()))
        }
        InstructionData::SparseSwitchPayload(payload) => {
            InstructionKind::Payload(Payload::SparseSwitch(payload.clone()))
        }
        InstructionData::ArrayDataPayload(payload) => {
            InstructionKind::Payload(Payload::ArrayData(payload.clone()))
        }
    };
    Ok(kind)
}

fn normalized_operands(
    offset: u32,
    opcode: Opcode,
    native: &NativeOperands,
    semantics: &InstructionSemantics,
) -> Result<Vec<Operand>> {
    let mut operands = Vec::with_capacity(
        semantics.writes.len() + semantics.reads.len() + non_register_operand_count(native),
    );
    operands.extend(semantics.writes.iter().cloned().map(Operand::Definition));
    operands.extend(semantics.reads.iter().cloned().map(Operand::Use));

    match native {
        NativeOperands::None
        | NativeOperands::Register(_)
        | NativeOperands::Registers { .. }
        | NativeOperands::ThreeRegisters { .. } => {}
        NativeOperands::RegisterLiteral { literal, .. }
        | NativeOperands::RegistersLiteral { literal, .. } => {
            operands.push(Operand::Literal(*literal));
        }
        NativeOperands::Branch { target }
        | NativeOperands::RegisterBranch { target, .. }
        | NativeOperands::RegistersBranch { target, .. } => {
            operands.push(Operand::Target(*target));
        }
        NativeOperands::RegisterIndex { index, .. }
        | NativeOperands::RegistersIndex { index, .. } => {
            push_reference(&mut operands, offset, opcode, *index)?;
        }
        NativeOperands::RegisterListIndex {
            index,
            secondary_index,
            ..
        }
        | NativeOperands::RegisterRangeIndex {
            index,
            secondary_index,
            ..
        } => {
            push_reference(&mut operands, offset, opcode, *index)?;
            if let Some(prototype) = secondary_index {
                operands.push(Operand::Reference {
                    kind: IndexKind::Prototype,
                    index: *prototype,
                });
            }
        }
    }
    Ok(operands)
}

const fn non_register_operand_count(operands: &NativeOperands) -> usize {
    match operands {
        NativeOperands::RegisterLiteral { .. }
        | NativeOperands::RegistersLiteral { .. }
        | NativeOperands::Branch { .. }
        | NativeOperands::RegisterBranch { .. }
        | NativeOperands::RegistersBranch { .. }
        | NativeOperands::RegisterIndex { .. }
        | NativeOperands::RegistersIndex { .. } => 1,
        NativeOperands::RegisterListIndex {
            secondary_index, ..
        }
        | NativeOperands::RegisterRangeIndex {
            secondary_index, ..
        } => {
            if secondary_index.is_some() {
                2
            } else {
                1
            }
        }
        NativeOperands::None
        | NativeOperands::Register(_)
        | NativeOperands::Registers { .. }
        | NativeOperands::ThreeRegisters { .. } => 0,
    }
}

fn push_reference(
    operands: &mut Vec<Operand>,
    offset: u32,
    opcode: Opcode,
    index: u32,
) -> Result<()> {
    let kind = opcode.index_kind().ok_or_else(|| {
        Error::invalid_instruction(
            offset,
            format!(
                "{} has an indexed operand but no index kind",
                opcode.mnemonic()
            ),
        )
    })?;
    operands.push(Operand::Reference { kind, index });
    Ok(())
}

#[allow(clippy::too_many_lines)]
const fn operation_kind(opcode: Opcode) -> OperationKind {
    use Opcode as O;

    match opcode {
        O::Nop => OperationKind::Nop,
        O::Move | O::MoveFrom16 | O::Move16 => OperationKind::Move(ValueKind::Single),
        O::MoveWide | O::MoveWideFrom16 | O::MoveWide16 => OperationKind::Move(ValueKind::Wide),
        O::MoveObject | O::MoveObjectFrom16 | O::MoveObject16 => {
            OperationKind::Move(ValueKind::Reference)
        }
        O::MoveResult => OperationKind::MoveResult(ValueKind::Single),
        O::MoveResultWide => OperationKind::MoveResult(ValueKind::Wide),
        O::MoveResultObject => OperationKind::MoveResult(ValueKind::Reference),
        O::MoveException => OperationKind::MoveException,
        O::ReturnVoid => OperationKind::Return(None),
        O::Return => OperationKind::Return(Some(ValueKind::Single)),
        O::ReturnWide => OperationKind::Return(Some(ValueKind::Wide)),
        O::ReturnObject => OperationKind::Return(Some(ValueKind::Reference)),
        O::Const4 | O::Const16 | O::Const | O::ConstHigh16 => {
            OperationKind::Constant(ConstantKind::Narrow)
        }
        O::ConstWide16 | O::ConstWide32 | O::ConstWide | O::ConstWideHigh16 => {
            OperationKind::Constant(ConstantKind::Wide)
        }
        O::ConstString | O::ConstStringJumbo => OperationKind::Constant(ConstantKind::String),
        O::ConstClass => OperationKind::Constant(ConstantKind::Class),
        O::MonitorEnter => OperationKind::Monitor(MonitorAction::Enter),
        O::MonitorExit => OperationKind::Monitor(MonitorAction::Exit),
        O::CheckCast => OperationKind::CheckCast,
        O::InstanceOf => OperationKind::InstanceOf,
        O::ArrayLength => OperationKind::ArrayLength,
        O::NewInstance => OperationKind::NewInstance,
        O::NewArray => OperationKind::NewArray,
        O::FilledNewArray | O::FilledNewArrayRange => OperationKind::FilledNewArray,
        O::FillArrayData => OperationKind::FillArrayData,
        O::Throw => OperationKind::Throw,
        O::Goto | O::Goto16 | O::Goto32 => OperationKind::Jump,
        O::PackedSwitch | O::SparseSwitch => OperationKind::Switch,
        O::CmplFloat => OperationKind::Compare(Comparison::FloatNanLow),
        O::CmpgFloat => OperationKind::Compare(Comparison::FloatNanHigh),
        O::CmplDouble => OperationKind::Compare(Comparison::DoubleNanLow),
        O::CmpgDouble => OperationKind::Compare(Comparison::DoubleNanHigh),
        O::CmpLong => OperationKind::Compare(Comparison::Long),
        O::IfEq => OperationKind::BranchPair(Relation::Equal),
        O::IfNe => OperationKind::BranchPair(Relation::NotEqual),
        O::IfLt => OperationKind::BranchPair(Relation::Less),
        O::IfGe => OperationKind::BranchPair(Relation::GreaterOrEqual),
        O::IfGt => OperationKind::BranchPair(Relation::Greater),
        O::IfLe => OperationKind::BranchPair(Relation::LessOrEqual),
        O::IfEqz => OperationKind::BranchZero(Relation::Equal),
        O::IfNez => OperationKind::BranchZero(Relation::NotEqual),
        O::IfLtz => OperationKind::BranchZero(Relation::Less),
        O::IfGez => OperationKind::BranchZero(Relation::GreaterOrEqual),
        O::IfGtz => OperationKind::BranchZero(Relation::Greater),
        O::IfLez => OperationKind::BranchZero(Relation::LessOrEqual),
        O::Aget => array(ArrayAccess::Get, Element::Single),
        O::AgetWide => array(ArrayAccess::Get, Element::Wide),
        O::AgetObject => array(ArrayAccess::Get, Element::Reference),
        O::AgetBoolean => array(ArrayAccess::Get, Element::Boolean),
        O::AgetByte => array(ArrayAccess::Get, Element::Byte),
        O::AgetChar => array(ArrayAccess::Get, Element::Char),
        O::AgetShort => array(ArrayAccess::Get, Element::Short),
        O::Aput => array(ArrayAccess::Put, Element::Single),
        O::AputWide => array(ArrayAccess::Put, Element::Wide),
        O::AputObject => array(ArrayAccess::Put, Element::Reference),
        O::AputBoolean => array(ArrayAccess::Put, Element::Boolean),
        O::AputByte => array(ArrayAccess::Put, Element::Byte),
        O::AputChar => array(ArrayAccess::Put, Element::Char),
        O::AputShort => array(ArrayAccess::Put, Element::Short),
        O::Iget => field(FieldAccess::GetInstance, Element::Single),
        O::IgetWide => field(FieldAccess::GetInstance, Element::Wide),
        O::IgetObject => field(FieldAccess::GetInstance, Element::Reference),
        O::IgetBoolean => field(FieldAccess::GetInstance, Element::Boolean),
        O::IgetByte => field(FieldAccess::GetInstance, Element::Byte),
        O::IgetChar => field(FieldAccess::GetInstance, Element::Char),
        O::IgetShort => field(FieldAccess::GetInstance, Element::Short),
        O::Iput => field(FieldAccess::PutInstance, Element::Single),
        O::IputWide => field(FieldAccess::PutInstance, Element::Wide),
        O::IputObject => field(FieldAccess::PutInstance, Element::Reference),
        O::IputBoolean => field(FieldAccess::PutInstance, Element::Boolean),
        O::IputByte => field(FieldAccess::PutInstance, Element::Byte),
        O::IputChar => field(FieldAccess::PutInstance, Element::Char),
        O::IputShort => field(FieldAccess::PutInstance, Element::Short),
        O::Sget => field(FieldAccess::GetStatic, Element::Single),
        O::SgetWide => field(FieldAccess::GetStatic, Element::Wide),
        O::SgetObject => field(FieldAccess::GetStatic, Element::Reference),
        O::SgetBoolean => field(FieldAccess::GetStatic, Element::Boolean),
        O::SgetByte => field(FieldAccess::GetStatic, Element::Byte),
        O::SgetChar => field(FieldAccess::GetStatic, Element::Char),
        O::SgetShort => field(FieldAccess::GetStatic, Element::Short),
        O::Sput => field(FieldAccess::PutStatic, Element::Single),
        O::SputWide => field(FieldAccess::PutStatic, Element::Wide),
        O::SputObject => field(FieldAccess::PutStatic, Element::Reference),
        O::SputBoolean => field(FieldAccess::PutStatic, Element::Boolean),
        O::SputByte => field(FieldAccess::PutStatic, Element::Byte),
        O::SputChar => field(FieldAccess::PutStatic, Element::Char),
        O::SputShort => field(FieldAccess::PutStatic, Element::Short),
        O::InvokeVirtual | O::InvokeVirtualRange => OperationKind::Invoke(Invocation::Virtual),
        O::InvokeSuper | O::InvokeSuperRange => OperationKind::Invoke(Invocation::Super),
        O::InvokeDirect | O::InvokeDirectRange => OperationKind::Invoke(Invocation::Direct),
        O::InvokeStatic | O::InvokeStaticRange => OperationKind::Invoke(Invocation::Static),
        O::InvokeInterface | O::InvokeInterfaceRange => {
            OperationKind::Invoke(Invocation::Interface)
        }
        O::NegInt => unary(Unary::Negate, ValueKind::Integer),
        O::NotInt => unary(Unary::Not, ValueKind::Integer),
        O::NegLong => unary(Unary::Negate, ValueKind::Long),
        O::NotLong => unary(Unary::Not, ValueKind::Long),
        O::NegFloat => unary(Unary::Negate, ValueKind::Float),
        O::NegDouble => unary(Unary::Negate, ValueKind::Double),
        O::IntToLong => OperationKind::Convert(Conversion::IntToLong),
        O::IntToFloat => OperationKind::Convert(Conversion::IntToFloat),
        O::IntToDouble => OperationKind::Convert(Conversion::IntToDouble),
        O::LongToInt => OperationKind::Convert(Conversion::LongToInt),
        O::LongToFloat => OperationKind::Convert(Conversion::LongToFloat),
        O::LongToDouble => OperationKind::Convert(Conversion::LongToDouble),
        O::FloatToInt => OperationKind::Convert(Conversion::FloatToInt),
        O::FloatToLong => OperationKind::Convert(Conversion::FloatToLong),
        O::FloatToDouble => OperationKind::Convert(Conversion::FloatToDouble),
        O::DoubleToInt => OperationKind::Convert(Conversion::DoubleToInt),
        O::DoubleToLong => OperationKind::Convert(Conversion::DoubleToLong),
        O::DoubleToFloat => OperationKind::Convert(Conversion::DoubleToFloat),
        O::IntToByte => OperationKind::Convert(Conversion::IntToByte),
        O::IntToChar => OperationKind::Convert(Conversion::IntToChar),
        O::IntToShort => OperationKind::Convert(Conversion::IntToShort),
        O::AddInt | O::AddInt2Addr | O::AddIntLit16 | O::AddIntLit8 => {
            binary(Arithmetic::Add, ValueKind::Integer)
        }
        O::SubInt | O::SubInt2Addr => binary(Arithmetic::Subtract, ValueKind::Integer),
        O::RsubInt | O::RsubIntLit8 => binary(Arithmetic::ReverseSubtract, ValueKind::Integer),
        O::MulInt | O::MulInt2Addr | O::MulIntLit16 | O::MulIntLit8 => {
            binary(Arithmetic::Multiply, ValueKind::Integer)
        }
        O::DivInt | O::DivInt2Addr | O::DivIntLit16 | O::DivIntLit8 => {
            binary(Arithmetic::Divide, ValueKind::Integer)
        }
        O::RemInt | O::RemInt2Addr | O::RemIntLit16 | O::RemIntLit8 => {
            binary(Arithmetic::Remainder, ValueKind::Integer)
        }
        O::AndInt | O::AndInt2Addr | O::AndIntLit16 | O::AndIntLit8 => {
            binary(Arithmetic::And, ValueKind::Integer)
        }
        O::OrInt | O::OrInt2Addr | O::OrIntLit16 | O::OrIntLit8 => {
            binary(Arithmetic::Or, ValueKind::Integer)
        }
        O::XorInt | O::XorInt2Addr | O::XorIntLit16 | O::XorIntLit8 => {
            binary(Arithmetic::Xor, ValueKind::Integer)
        }
        O::ShlInt | O::ShlInt2Addr | O::ShlIntLit8 => {
            binary(Arithmetic::ShiftLeft, ValueKind::Integer)
        }
        O::ShrInt | O::ShrInt2Addr | O::ShrIntLit8 => {
            binary(Arithmetic::ShiftRight, ValueKind::Integer)
        }
        O::UshrInt | O::UshrInt2Addr | O::UshrIntLit8 => {
            binary(Arithmetic::UnsignedShiftRight, ValueKind::Integer)
        }
        O::AddLong | O::AddLong2Addr => binary(Arithmetic::Add, ValueKind::Long),
        O::SubLong | O::SubLong2Addr => binary(Arithmetic::Subtract, ValueKind::Long),
        O::MulLong | O::MulLong2Addr => binary(Arithmetic::Multiply, ValueKind::Long),
        O::DivLong | O::DivLong2Addr => binary(Arithmetic::Divide, ValueKind::Long),
        O::RemLong | O::RemLong2Addr => binary(Arithmetic::Remainder, ValueKind::Long),
        O::AndLong | O::AndLong2Addr => binary(Arithmetic::And, ValueKind::Long),
        O::OrLong | O::OrLong2Addr => binary(Arithmetic::Or, ValueKind::Long),
        O::XorLong | O::XorLong2Addr => binary(Arithmetic::Xor, ValueKind::Long),
        O::ShlLong | O::ShlLong2Addr => binary(Arithmetic::ShiftLeft, ValueKind::Long),
        O::ShrLong | O::ShrLong2Addr => binary(Arithmetic::ShiftRight, ValueKind::Long),
        O::UshrLong | O::UshrLong2Addr => binary(Arithmetic::UnsignedShiftRight, ValueKind::Long),
        O::AddFloat | O::AddFloat2Addr => binary(Arithmetic::Add, ValueKind::Float),
        O::SubFloat | O::SubFloat2Addr => binary(Arithmetic::Subtract, ValueKind::Float),
        O::MulFloat | O::MulFloat2Addr => binary(Arithmetic::Multiply, ValueKind::Float),
        O::DivFloat | O::DivFloat2Addr => binary(Arithmetic::Divide, ValueKind::Float),
        O::RemFloat | O::RemFloat2Addr => binary(Arithmetic::Remainder, ValueKind::Float),
        O::AddDouble | O::AddDouble2Addr => binary(Arithmetic::Add, ValueKind::Double),
        O::SubDouble | O::SubDouble2Addr => binary(Arithmetic::Subtract, ValueKind::Double),
        O::MulDouble | O::MulDouble2Addr => binary(Arithmetic::Multiply, ValueKind::Double),
        O::DivDouble | O::DivDouble2Addr => binary(Arithmetic::Divide, ValueKind::Double),
        O::RemDouble | O::RemDouble2Addr => binary(Arithmetic::Remainder, ValueKind::Double),
        O::InvokePolymorphic | O::InvokePolymorphicRange => {
            OperationKind::Invoke(Invocation::Polymorphic)
        }
        O::InvokeCustom | O::InvokeCustomRange => OperationKind::Invoke(Invocation::Custom),
        O::ConstMethodHandle => OperationKind::Constant(ConstantKind::MethodHandle),
        O::ConstMethodType => OperationKind::Constant(ConstantKind::MethodType),
    }
}

const fn array(access: ArrayAccess, element: Element) -> OperationKind {
    OperationKind::Array { access, element }
}

const fn field(access: FieldAccess, value: Element) -> OperationKind {
    OperationKind::Field { access, value }
}

const fn unary(operator: Unary, kind: ValueKind) -> OperationKind {
    OperationKind::Unary { operator, kind }
}

const fn binary(operator: Arithmetic, kind: ValueKind) -> OperationKind {
    OperationKind::Binary { operator, kind }
}

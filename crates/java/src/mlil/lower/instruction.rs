//! Canonical JVM instruction selection and operand-stack scheduling.

use std::collections::BTreeMap;

use ::mlil::{
    ArrayAccess, BinaryOperator, BranchOperandKind, BranchPredicate, CallKind, Constant,
    Conversion, ElementType, FieldAccess, Function, Instruction, InstructionId, MonitorAction,
    Operation, Relation, ThreeWayComparison, UnaryOperator, ValueType,
};
use disassembler::cfglib::BlockId;
use disassembler::{Reference, ReferenceSymbol};

use crate::JavaReferenceResolver;
use crate::bytecode::{CodeBuilder, Label, LocalKind, Opcode, Operand};
use crate::classfile::{Constant as NativeConstant, ConstantPool};

use super::super::{Error, Result};
use super::arrays::{emit_allocation, emit_array_initialization};
use super::locals::{LocalAllocation, width};
use super::typing::{method_return_is_reference, zero_use_is_reference};

#[derive(Debug, Clone, Copy)]
pub(super) struct Emission {
    pub(super) start: Label,
    pub(super) end: Label,
    pub(super) throw_range: Option<(Label, Label)>,
    pub(super) maximum_stack: u16,
}

#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
pub(super) fn emit_instruction<R: JavaReferenceResolver>(
    builder: &mut CodeBuilder,
    instruction: &Instruction,
    allocation: &LocalAllocation,
    pool: &mut ConstantPool,
    resolver: &mut R,
    function: &Function,
    labels: &BTreeMap<BlockId, Label>,
    block: BlockId,
) -> Result<Emission> {
    let start = builder.new_label();
    builder.bind(start)?;
    let mut throw_range = None;
    match instruction.operation() {
        Operation::Nop => plain(builder, Opcode::Nop, Operand::None),
        Operation::Copy => copy(builder, instruction, allocation, function)?,
        Operation::ParallelCopy | Operation::TypeRefine => {
            parallel_copy(builder, instruction, allocation, function)?;
        }
        Operation::Discard => discard(builder, instruction, allocation, function)?,
        Operation::Constant(constant) => {
            let range = primary(builder, instruction, |builder| {
                emit_constant(builder, constant, instruction, pool, resolver)
            })?;
            throw_range = range;
            store_definitions(builder, instruction, allocation)?;
        }
        Operation::Unary(operator) => {
            load_uses(builder, instruction, allocation, function)?;
            throw_range = primary(builder, instruction, |builder| {
                emit_unary(builder, *operator, &instruction.use_types()[0]);
                Ok(())
            })?;
            store_definitions(builder, instruction, allocation)?;
        }
        Operation::Binary(operator) => {
            emit_binary_loads(builder, instruction, allocation, function, *operator)?;
            throw_range = primary(builder, instruction, |builder| {
                plain(
                    builder,
                    binary_opcode(*operator, &instruction.use_types()[0], instruction.id())?,
                    Operand::None,
                );
                Ok(())
            })?;
            store_definitions(builder, instruction, allocation)?;
        }
        Operation::Convert(conversion) => {
            load_uses(builder, instruction, allocation, function)?;
            throw_range = primary(builder, instruction, |builder| {
                plain(builder, conversion_opcode(*conversion), Operand::None);
                Ok(())
            })?;
            store_definitions(builder, instruction, allocation)?;
        }
        Operation::Compare(comparison) => {
            load_uses(builder, instruction, allocation, function)?;
            throw_range = primary(builder, instruction, |builder| {
                plain(builder, comparison_opcode(*comparison), Operand::None);
                Ok(())
            })?;
            store_definitions(builder, instruction, allocation)?;
        }
        Operation::Branch(predicate) => {
            load_uses(builder, instruction, allocation, function)?;
            let taken = target(
                function,
                block,
                |role| matches!(role, ::mlil::EdgeRole::BranchTrue),
                instruction.id(),
            )?;
            let fallback = target(
                function,
                block,
                |role| matches!(role, ::mlil::EdgeRole::BranchFalse),
                instruction.id(),
            )?;
            builder.emit_branch(branch_opcode(*predicate), labels[&taken])?;
            builder.emit_branch(Opcode::Goto, labels[&fallback])?;
        }
        Operation::Jump => {
            let target = target(
                function,
                block,
                |role| matches!(role, ::mlil::EdgeRole::Jump),
                instruction.id(),
            )?;
            builder.emit_branch(Opcode::Goto, labels[&target])?;
        }
        Operation::Switch(keys) => {
            load_uses(builder, instruction, allocation, function)?;
            let fallback = target(
                function,
                block,
                |role| matches!(role, ::mlil::EdgeRole::SwitchDefault),
                instruction.id(),
            )?;
            let mut pairs = Vec::with_capacity(keys.len());
            for &key in keys {
                let key_i32 = i32::try_from(key)
                    .map_err(|_| Error::lowering(instruction.id(), "JVM switch key exceeds i32"))?;
                let target = target(
                    function,
                    block,
                    |role| matches!(role, ::mlil::EdgeRole::SwitchCase(value) if *value == key),
                    instruction.id(),
                )?;
                pairs.push((key_i32, labels[&target]));
            }
            builder.emit_lookup_switch(labels[&fallback], pairs)?;
        }
        Operation::Return => {
            load_uses(builder, instruction, allocation, function)?;
            let opcode = match instruction.use_types().first() {
                None => Opcode::Return,
                Some(ValueType::Zero)
                    if method_return_is_reference(&function.source().symbol.signature)
                        == Some(true) =>
                {
                    Opcode::AReturn
                }
                Some(value) => return_opcode(value, instruction.id())?,
            };
            plain(builder, opcode, Operand::None);
        }
        Operation::Throw => {
            load_uses(builder, instruction, allocation, function)?;
            throw_range = primary(builder, instruction, |builder| {
                plain(builder, Opcode::AThrow, Operand::None);
                Ok(())
            })?;
        }
        Operation::Array { access, element } => {
            load_uses(builder, instruction, allocation, function)?;
            throw_range = primary(builder, instruction, |builder| {
                plain(builder, array_opcode(*access, *element), Operand::None);
                Ok(())
            })?;
            store_definitions(builder, instruction, allocation)?;
        }
        Operation::ArrayLength => {
            unary_managed(
                builder,
                instruction,
                allocation,
                function,
                Opcode::ArrayLength,
                &mut throw_range,
            )?;
        }
        Operation::Field { access, field } => {
            load_uses(builder, instruction, allocation, function)?;
            let index = resolve(field, instruction.id(), pool, resolver)?;
            throw_range = primary(builder, instruction, |builder| {
                plain(builder, field_opcode(*access), Operand::Constant(index));
                Ok(())
            })?;
            store_definitions(builder, instruction, allocation)?;
        }
        Operation::Call {
            kind,
            target: reference,
            descriptor,
        } => {
            load_uses(builder, instruction, allocation, function)?;
            let descriptor = descriptor.as_deref().ok_or_else(|| {
                Error::lowering(instruction.id(), "JVM call lacks an effective descriptor")
            })?;
            let effective_reference =
                effective_jvm_call_reference(*kind, reference, descriptor, instruction.id())?;
            let index = resolve(
                effective_reference.as_ref().unwrap_or(reference),
                instruction.id(),
                pool,
                resolver,
            )?;
            throw_range = primary(builder, instruction, |builder| {
                emit_call(
                    builder,
                    *kind,
                    reference.kind,
                    index,
                    descriptor,
                    instruction,
                )?;
                Ok(())
            })?;
            store_definitions(builder, instruction, allocation)?;
        }
        Operation::Allocate(kind) => {
            emit_allocation(
                builder,
                instruction,
                allocation,
                function,
                kind,
                pool,
                resolver,
                &mut throw_range,
            )?;
        }
        Operation::InitializeArray { array_type, values } => {
            throw_range = emit_array_initialization(
                builder,
                instruction,
                allocation,
                function,
                array_type,
                values,
                pool,
                resolver,
            )?;
        }
        Operation::CheckCast(reference) => {
            load_uses(builder, instruction, allocation, function)?;
            let index = resolve(reference, instruction.id(), pool, resolver)?;
            throw_range = primary(builder, instruction, |builder| {
                plain(builder, Opcode::CheckCast, Operand::Constant(index));
                Ok(())
            })?;
            store_definitions(builder, instruction, allocation)?;
        }
        Operation::InstanceOf(reference) => {
            load_uses(builder, instruction, allocation, function)?;
            let index = resolve(reference, instruction.id(), pool, resolver)?;
            throw_range = primary(builder, instruction, |builder| {
                plain(builder, Opcode::InstanceOf, Operand::Constant(index));
                Ok(())
            })?;
            store_definitions(builder, instruction, allocation)?;
        }
        Operation::Monitor(action) => {
            load_uses(builder, instruction, allocation, function)?;
            throw_range = primary(builder, instruction, |builder| {
                plain(
                    builder,
                    match action {
                        MonitorAction::Enter => Opcode::MonitorEnter,
                        MonitorAction::Exit => Opcode::MonitorExit,
                    },
                    Operand::None,
                );
                Ok(())
            })?;
        }
        Operation::CaughtException(_) => {
            let (&definition, value_type) = instruction
                .defs()
                .first()
                .zip(instruction.def_types().first())
                .ok_or_else(|| {
                    Error::lowering(instruction.id(), "caught exception has no definition")
                })?;
            let _ = builder.emit_store(
                local_kind(value_type, instruction.id())?,
                allocation.slot(definition),
            );
        }
        Operation::Intrinsic(name) => {
            return Err(Error::lowering(
                instruction.id(),
                format!("JVM backend does not encode MLIL intrinsic `{name}`"),
            ));
        }
    }
    let end = builder.new_label();
    builder.bind(end)?;
    let stack = instruction
        .use_types()
        .iter()
        .map(width)
        .try_fold(16u32, |total, value| total.checked_add(u32::from(value)))
        .ok_or_else(|| Error::lowering(instruction.id(), "JVM stack bound overflow"))?;
    let maximum_stack = u16::try_from(stack)
        .map_err(|_| Error::lowering(instruction.id(), "JVM stack bound exceeds u16"))?;
    Ok(Emission {
        start,
        end,
        throw_range,
        maximum_stack,
    })
}

const METHOD_HANDLE_NAME: &str = "java/lang/invoke/MethodHandle";
const METHOD_HANDLE_INVOKE_NAME: &str = "invoke";
const METHOD_HANDLE_INVOKE_EXACT_NAME: &str = "invokeExact";
const METHOD_HANDLE_DECLARED_DESCRIPTOR: &str = "([Ljava/lang/Object;)Ljava/lang/Object;";

fn effective_jvm_call_reference(
    kind: CallKind,
    reference: &Reference,
    descriptor: &str,
    instruction: InstructionId,
) -> Result<Option<Reference>> {
    if kind != CallKind::Polymorphic {
        return Ok(None);
    }
    let mut effective = reference.clone();
    let Some(ReferenceSymbol::Method {
        owner,
        name,
        descriptor: target_descriptor,
    }) = &mut effective.symbol
    else {
        return Err(Error::lowering(
            instruction,
            "signature-polymorphic call lacks a structured method identity",
        ));
    };
    let normalized_owner = owner
        .strip_prefix('L')
        .and_then(|value| value.strip_suffix(';'))
        .unwrap_or(owner);
    if normalized_owner != METHOD_HANDLE_NAME
        || !matches!(
            name.text.as_str(),
            METHOD_HANDLE_INVOKE_NAME | METHOD_HANDLE_INVOKE_EXACT_NAME
        )
        || target_descriptor != METHOD_HANDLE_DECLARED_DESCRIPTOR
    {
        return Err(Error::lowering(
            instruction,
            "JVM signature-polymorphic dispatch requires MethodHandle.invoke or invokeExact",
        ));
    }
    descriptor.clone_into(target_descriptor);
    effective.display = Some(format!("{}.{}{}", owner, name.text, descriptor));
    Ok(Some(effective))
}

fn copy(
    builder: &mut CodeBuilder,
    instruction: &Instruction,
    allocation: &LocalAllocation,
    function: &Function,
) -> Result<()> {
    load_uses(builder, instruction, allocation, function)?;
    store_definitions(builder, instruction, allocation)
}

fn parallel_copy(
    builder: &mut CodeBuilder,
    instruction: &Instruction,
    allocation: &LocalAllocation,
    function: &Function,
) -> Result<()> {
    load_uses(builder, instruction, allocation, function)?;
    store_definitions(builder, instruction, allocation)
}

fn discard(
    builder: &mut CodeBuilder,
    instruction: &Instruction,
    allocation: &LocalAllocation,
    function: &Function,
) -> Result<()> {
    for (position, (&variable, value_type)) in instruction
        .uses()
        .iter()
        .zip(instruction.use_types())
        .enumerate()
    {
        load_use(
            builder,
            instruction,
            allocation,
            function,
            position,
            variable,
        )?;
        plain(
            builder,
            if width(value_type) == 2 {
                Opcode::Pop2
            } else {
                Opcode::Pop
            },
            Operand::None,
        );
    }
    Ok(())
}

pub(super) fn load_uses(
    builder: &mut CodeBuilder,
    instruction: &Instruction,
    allocation: &LocalAllocation,
    function: &Function,
) -> Result<()> {
    for (position, &variable) in instruction.uses().iter().enumerate() {
        load_use(
            builder,
            instruction,
            allocation,
            function,
            position,
            variable,
        )?;
    }
    Ok(())
}

pub(super) fn load_use(
    builder: &mut CodeBuilder,
    instruction: &Instruction,
    allocation: &LocalAllocation,
    function: &Function,
    position: usize,
    variable: ::mlil::VariableId,
) -> Result<()> {
    let value_type = &instruction.use_types()[position];
    if matches!(value_type, ValueType::Zero)
        && zero_use_is_reference(function, instruction, position)?
    {
        plain(builder, Opcode::AConstNull, Operand::None);
    } else {
        let _ = builder.emit_load(
            local_kind(value_type, instruction.id())?,
            allocation.slot(variable),
        );
    }
    Ok(())
}

pub(super) fn store_definitions(
    builder: &mut CodeBuilder,
    instruction: &Instruction,
    allocation: &LocalAllocation,
) -> Result<()> {
    for (&variable, value_type) in instruction.defs().iter().zip(instruction.def_types()).rev() {
        let _ = builder.emit_store(
            local_kind(value_type, instruction.id())?,
            allocation.slot(variable),
        );
    }
    Ok(())
}

fn emit_binary_loads(
    builder: &mut CodeBuilder,
    instruction: &Instruction,
    allocation: &LocalAllocation,
    function: &Function,
    operator: BinaryOperator,
) -> Result<()> {
    if operator == BinaryOperator::ReverseSubtract {
        for position in [1usize, 0] {
            let variable = instruction.uses()[position];
            let value_type = &instruction.use_types()[position];
            let _ = builder.emit_load(
                local_kind(value_type, instruction.id())?,
                allocation.slot(variable),
            );
        }
    } else {
        load_uses(builder, instruction, allocation, function)?;
    }
    Ok(())
}

pub(super) fn primary(
    builder: &mut CodeBuilder,
    instruction: &Instruction,
    emit: impl FnOnce(&mut CodeBuilder) -> Result<()>,
) -> Result<Option<(Label, Label)>> {
    let start = builder.new_label();
    builder.bind(start)?;
    emit(builder)?;
    let end = builder.new_label();
    builder.bind(end)?;
    Ok(instruction.may_throw().then_some((start, end)))
}

pub(super) fn emit_constant<R: JavaReferenceResolver>(
    builder: &mut CodeBuilder,
    constant: &Constant,
    instruction: &Instruction,
    pool: &mut ConstantPool,
    resolver: &mut R,
) -> Result<()> {
    match constant {
        Constant::Null => plain(builder, Opcode::AConstNull, Operand::None),
        Constant::Integer(value) => emit_integer(builder, *value, pool)?,
        Constant::Long(value) => match value {
            0 => plain(builder, Opcode::LConst0, Operand::None),
            1 => plain(builder, Opcode::LConst1, Operand::None),
            _ => {
                let index = pool.intern(NativeConstant::Long(*value))?;
                let _ = builder.emit_ldc2(index);
            }
        },
        Constant::Float(bits) => {
            let value = f32::from_bits(*bits);
            let opcode = match *bits {
                0x0000_0000 => Some(Opcode::FConst0),
                0x3f80_0000 => Some(Opcode::FConst1),
                0x4000_0000 => Some(Opcode::FConst2),
                _ => None,
            };
            if let Some(opcode) = opcode {
                plain(builder, opcode, Operand::None);
            } else {
                let index = pool.intern(NativeConstant::Float(value))?;
                let _ = builder.emit_ldc(index);
            }
        }
        Constant::Double(bits) => {
            let value = f64::from_bits(*bits);
            let opcode = match *bits {
                0x0000_0000_0000_0000 => Some(Opcode::DConst0),
                0x3ff0_0000_0000_0000 => Some(Opcode::DConst1),
                _ => None,
            };
            if let Some(opcode) = opcode {
                plain(builder, opcode, Operand::None);
            } else {
                let index = pool.intern(NativeConstant::Double(value))?;
                let _ = builder.emit_ldc2(index);
            }
        }
        Constant::Reference(reference) => {
            let index = resolve(reference, instruction.id(), pool, resolver)?;
            if matches!(
                instruction.def_types().first(),
                Some(ValueType::Long | ValueType::Double | ValueType::Bits64)
            ) {
                let _ = builder.emit_ldc2(index);
            } else {
                let _ = builder.emit_ldc(index);
            }
        }
    }
    Ok(())
}

pub(super) fn emit_integer(
    builder: &mut CodeBuilder,
    value: i32,
    pool: &mut ConstantPool,
) -> Result<()> {
    match value {
        -1 => plain(builder, Opcode::IConstM1, Operand::None),
        0 => plain(builder, Opcode::IConst0, Operand::None),
        1 => plain(builder, Opcode::IConst1, Operand::None),
        2 => plain(builder, Opcode::IConst2, Operand::None),
        3 => plain(builder, Opcode::IConst3, Operand::None),
        4 => plain(builder, Opcode::IConst4, Operand::None),
        5 => plain(builder, Opcode::IConst5, Operand::None),
        value => {
            if let Ok(value) = i8::try_from(value) {
                plain(builder, Opcode::BiPush, Operand::Byte(value));
            } else if let Ok(value) = i16::try_from(value) {
                plain(builder, Opcode::SiPush, Operand::Short(value));
            } else {
                let index = pool.intern(NativeConstant::Integer(value))?;
                let _ = builder.emit_ldc(index);
            }
        }
    }
    Ok(())
}

fn emit_unary(builder: &mut CodeBuilder, operator: UnaryOperator, value_type: &ValueType) {
    match operator {
        UnaryOperator::Negate => plain(builder, negate_opcode(value_type), Operand::None),
        UnaryOperator::BitwiseNot => {
            if matches!(value_type, ValueType::Long | ValueType::Bits64) {
                plain(builder, Opcode::LConst1, Operand::None);
                plain(builder, Opcode::LNeg, Operand::None);
                plain(builder, Opcode::LXor, Operand::None);
            } else {
                plain(builder, Opcode::IConstM1, Operand::None);
                plain(builder, Opcode::IXor, Operand::None);
            }
        }
    }
}

fn unary_managed(
    builder: &mut CodeBuilder,
    instruction: &Instruction,
    allocation: &LocalAllocation,
    function: &Function,
    opcode: Opcode,
    throw_range: &mut Option<(Label, Label)>,
) -> Result<()> {
    load_uses(builder, instruction, allocation, function)?;
    *throw_range = primary(builder, instruction, |builder| {
        plain(builder, opcode, Operand::None);
        Ok(())
    })?;
    store_definitions(builder, instruction, allocation)
}

fn emit_call(
    builder: &mut CodeBuilder,
    kind: CallKind,
    reference_kind: disassembler::ReferenceKind,
    index: u16,
    _descriptor: &str,
    instruction: &Instruction,
) -> Result<()> {
    match kind {
        CallKind::Virtual | CallKind::Polymorphic => {
            plain(builder, Opcode::InvokeVirtual, Operand::Constant(index));
        }
        CallKind::Super | CallKind::Direct => {
            plain(builder, Opcode::InvokeSpecial, Operand::Constant(index));
        }
        CallKind::Static => plain(builder, Opcode::InvokeStatic, Operand::Constant(index)),
        CallKind::Interface => {
            let count = instruction
                .use_types()
                .iter()
                .map(width)
                .try_fold(0u16, u16::checked_add)
                .and_then(|value| u8::try_from(value).ok())
                .ok_or_else(|| {
                    Error::lowering(
                        instruction.id(),
                        "invokeinterface argument width exceeds u8",
                    )
                })?;
            plain(
                builder,
                Opcode::InvokeInterface,
                Operand::InvokeInterface { index, count },
            );
        }
        CallKind::Dynamic => plain(
            builder,
            Opcode::InvokeDynamic,
            Operand::InvokeDynamic(index),
        ),
    }
    if kind != CallKind::Dynamic
        && matches!(reference_kind, disassembler::ReferenceKind::DynamicCallSite)
    {
        return Err(Error::lowering(
            instruction.id(),
            "dynamic call-site reference requires dynamic dispatch",
        ));
    }
    Ok(())
}

fn resolve<R: JavaReferenceResolver>(
    reference: &disassembler::Reference,
    instruction: InstructionId,
    pool: &mut ConstantPool,
    resolver: &mut R,
) -> Result<u16> {
    resolver
        .resolve(reference, pool)
        .map_err(|source| Error::Reference {
            instruction,
            source,
        })
}

fn target(
    function: &Function,
    block: BlockId,
    predicate: impl Fn(&::mlil::EdgeRole) -> bool,
    instruction: InstructionId,
) -> Result<BlockId> {
    let matches = function
        .cfg()
        .successor_edges(block)
        .iter()
        .map(|edge| function.cfg().edge(*edge))
        .filter(|edge| predicate(&edge.payload().role))
        .map(disassembler::cfglib::Edge::target)
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [target] => Ok(*target),
        _ => Err(Error::lowering(
            instruction,
            format!(
                "expected one matching control-flow edge, found {}",
                matches.len()
            ),
        )),
    }
}

fn local_kind(value_type: &ValueType, instruction: InstructionId) -> Result<LocalKind> {
    match value_type {
        ValueType::Boolean | ValueType::Integer | ValueType::Bits32 | ValueType::Zero => {
            Ok(LocalKind::Integer)
        }
        ValueType::Long | ValueType::Bits64 => Ok(LocalKind::Long),
        ValueType::Float => Ok(LocalKind::Float),
        ValueType::Double => Ok(LocalKind::Double),
        ValueType::Null
        | ValueType::Reference(_)
        | ValueType::UninitializedThis(_)
        | ValueType::Uninitialized { .. } => Ok(LocalKind::Reference),
        ValueType::Unknown | ValueType::Conflict | ValueType::ReturnAddress => {
            Err(Error::lowering(
                instruction,
                format!("value type {value_type:?} has no canonical JVM local category"),
            ))
        }
    }
}

fn negate_opcode(value_type: &ValueType) -> Opcode {
    match value_type {
        ValueType::Long | ValueType::Bits64 => Opcode::LNeg,
        ValueType::Float => Opcode::FNeg,
        ValueType::Double => Opcode::DNeg,
        _ => Opcode::INeg,
    }
}

fn binary_opcode(
    operator: BinaryOperator,
    value_type: &ValueType,
    id: InstructionId,
) -> Result<Opcode> {
    use BinaryOperator as B;
    let category = match value_type {
        ValueType::Long | ValueType::Bits64 => 'l',
        ValueType::Float => 'f',
        ValueType::Double => 'd',
        _ => 'i',
    };
    let opcode = match (operator, category) {
        (B::Add, 'i') => Opcode::IAdd,
        (B::Add, 'l') => Opcode::LAdd,
        (B::Add, 'f') => Opcode::FAdd,
        (B::Add, 'd') => Opcode::DAdd,
        (B::Subtract | B::ReverseSubtract, 'i') => Opcode::ISub,
        (B::Subtract | B::ReverseSubtract, 'l') => Opcode::LSub,
        (B::Subtract | B::ReverseSubtract, 'f') => Opcode::FSub,
        (B::Subtract | B::ReverseSubtract, 'd') => Opcode::DSub,
        (B::Multiply, 'i') => Opcode::IMul,
        (B::Multiply, 'l') => Opcode::LMul,
        (B::Multiply, 'f') => Opcode::FMul,
        (B::Multiply, 'd') => Opcode::DMul,
        (B::Divide, 'i') => Opcode::IDiv,
        (B::Divide, 'l') => Opcode::LDiv,
        (B::Divide, 'f') => Opcode::FDiv,
        (B::Divide, 'd') => Opcode::DDiv,
        (B::Remainder, 'i') => Opcode::IRem,
        (B::Remainder, 'l') => Opcode::LRem,
        (B::Remainder, 'f') => Opcode::FRem,
        (B::Remainder, 'd') => Opcode::DRem,
        (B::And, 'i') => Opcode::IAnd,
        (B::And, 'l') => Opcode::LAnd,
        (B::Or, 'i') => Opcode::IOr,
        (B::Or, 'l') => Opcode::LOr,
        (B::Xor, 'i') => Opcode::IXor,
        (B::Xor, 'l') => Opcode::LXor,
        (B::ShiftLeft, 'i') => Opcode::IShl,
        (B::ShiftLeft, 'l') => Opcode::LShl,
        (B::ShiftRight, 'i') => Opcode::IShr,
        (B::ShiftRight, 'l') => Opcode::LShr,
        (B::UnsignedShiftRight, 'i') => Opcode::IUShr,
        (B::UnsignedShiftRight, 'l') => Opcode::LUShr,
        _ => {
            return Err(Error::lowering(
                id,
                "operator is incompatible with JVM value category",
            ));
        }
    };
    Ok(opcode)
}

const fn conversion_opcode(conversion: Conversion) -> Opcode {
    match conversion {
        Conversion::IntToLong => Opcode::I2L,
        Conversion::IntToFloat => Opcode::I2F,
        Conversion::IntToDouble => Opcode::I2D,
        Conversion::LongToInt => Opcode::L2I,
        Conversion::LongToFloat => Opcode::L2F,
        Conversion::LongToDouble => Opcode::L2D,
        Conversion::FloatToInt => Opcode::F2I,
        Conversion::FloatToLong => Opcode::F2L,
        Conversion::FloatToDouble => Opcode::F2D,
        Conversion::DoubleToInt => Opcode::D2I,
        Conversion::DoubleToLong => Opcode::D2L,
        Conversion::DoubleToFloat => Opcode::D2F,
        Conversion::IntToByte => Opcode::I2B,
        Conversion::IntToChar => Opcode::I2C,
        Conversion::IntToShort => Opcode::I2S,
    }
}

const fn comparison_opcode(comparison: ThreeWayComparison) -> Opcode {
    match comparison {
        ThreeWayComparison::Long => Opcode::LCmp,
        ThreeWayComparison::FloatNanLow => Opcode::FCmpL,
        ThreeWayComparison::FloatNanHigh => Opcode::FCmpG,
        ThreeWayComparison::DoubleNanLow => Opcode::DCmpL,
        ThreeWayComparison::DoubleNanHigh => Opcode::DCmpG,
    }
}

const fn branch_opcode(predicate: BranchPredicate) -> Opcode {
    match predicate.operands {
        BranchOperandKind::IntegerZero => relation_opcode(predicate.relation, false),
        BranchOperandKind::IntegerPair => relation_opcode(predicate.relation, true),
        BranchOperandKind::ReferencePair => {
            if matches!(predicate.relation, Relation::NotEqual) {
                Opcode::IfACmpNe
            } else {
                Opcode::IfACmpEq
            }
        }
        BranchOperandKind::ReferenceNull => {
            if matches!(predicate.relation, Relation::NotEqual) {
                Opcode::IfNonNull
            } else {
                Opcode::IfNull
            }
        }
        BranchOperandKind::Boolean => {
            if matches!(predicate.relation, Relation::NotEqual) {
                Opcode::IfEq
            } else {
                Opcode::IfNe
            }
        }
    }
}

const fn relation_opcode(relation: Relation, pair: bool) -> Opcode {
    match (relation, pair) {
        (Relation::Equal, false) => Opcode::IfEq,
        (Relation::NotEqual, false) => Opcode::IfNe,
        (Relation::Less, false) => Opcode::IfLt,
        (Relation::GreaterOrEqual, false) => Opcode::IfGe,
        (Relation::Greater, false) => Opcode::IfGt,
        (Relation::LessOrEqual, false) => Opcode::IfLe,
        (Relation::Equal, true) => Opcode::IfICmpEq,
        (Relation::NotEqual, true) => Opcode::IfICmpNe,
        (Relation::Less, true) => Opcode::IfICmpLt,
        (Relation::GreaterOrEqual, true) => Opcode::IfICmpGe,
        (Relation::Greater, true) => Opcode::IfICmpGt,
        (Relation::LessOrEqual, true) => Opcode::IfICmpLe,
    }
}

fn return_opcode(value_type: &ValueType, id: InstructionId) -> Result<Opcode> {
    match local_kind(value_type, id)? {
        LocalKind::Integer => Ok(Opcode::IReturn),
        LocalKind::Long => Ok(Opcode::LReturn),
        LocalKind::Float => Ok(Opcode::FReturn),
        LocalKind::Double => Ok(Opcode::DReturn),
        LocalKind::Reference => Ok(Opcode::AReturn),
    }
}

pub(super) const fn array_opcode(access: ArrayAccess, element: ElementType) -> Opcode {
    match (access, element) {
        (ArrayAccess::Get, ElementType::Bits32 | ElementType::Integer) => Opcode::IALoad,
        (ArrayAccess::Get, ElementType::Bits64 | ElementType::Long) => Opcode::LALoad,
        (ArrayAccess::Get, ElementType::Float) => Opcode::FALoad,
        (ArrayAccess::Get, ElementType::Double) => Opcode::DALoad,
        (ArrayAccess::Get, ElementType::Reference) => Opcode::AALoad,
        (
            ArrayAccess::Get,
            ElementType::Boolean | ElementType::Byte | ElementType::ByteOrBoolean,
        ) => Opcode::BALoad,
        (ArrayAccess::Get, ElementType::Char) => Opcode::CALoad,
        (ArrayAccess::Get, ElementType::Short) => Opcode::SALoad,
        (ArrayAccess::Put, ElementType::Bits32 | ElementType::Integer) => Opcode::IAStore,
        (ArrayAccess::Put, ElementType::Bits64 | ElementType::Long) => Opcode::LAStore,
        (ArrayAccess::Put, ElementType::Float) => Opcode::FAStore,
        (ArrayAccess::Put, ElementType::Double) => Opcode::DAStore,
        (ArrayAccess::Put, ElementType::Reference) => Opcode::AAStore,
        (
            ArrayAccess::Put,
            ElementType::Boolean | ElementType::Byte | ElementType::ByteOrBoolean,
        ) => Opcode::BAStore,
        (ArrayAccess::Put, ElementType::Char) => Opcode::CAStore,
        (ArrayAccess::Put, ElementType::Short) => Opcode::SAStore,
    }
}

const fn field_opcode(access: FieldAccess) -> Opcode {
    match access {
        FieldAccess::GetInstance => Opcode::GetField,
        FieldAccess::PutInstance => Opcode::PutField,
        FieldAccess::GetStatic => Opcode::GetStatic,
        FieldAccess::PutStatic => Opcode::PutStatic,
    }
}

pub(super) fn plain(builder: &mut CodeBuilder, opcode: Opcode, operand: Operand) {
    let _ = builder.emit(opcode, operand);
}

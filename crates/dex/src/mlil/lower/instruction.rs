//! Canonical Dalvik instruction selection and register scheduling.

use ::mlil::{
    AllocationKind, ArrayAccess, ArrayType, BinaryOperator, BranchOperandKind, CallKind, Constant,
    FieldAccess, Function, Instruction, InstructionId, MonitorAction, Operation, ValueType,
};
use disassembler::cfglib::BlockId;
use disassembler::{AddressRange, CodeAddress, ReferenceKind};

use crate::file::{DexFile, MethodIndex};
use crate::instruction::{ArrayDataPayload, IndexKind, Opcode, Operands};

use super::opcodes::{
    array_opcode, binary_opcode, branch_opcode, call_opcode, comparison_opcode, conversion_opcode,
    field_opcode, inverted_branch, return_opcode, unary_opcode,
};
use super::registers::{RegisterAllocation, width, words};
use super::{DexMlilReferenceResolver, Error, Planner, Result};

#[derive(Debug, Clone, Copy)]
pub(super) struct Emission {
    pub(super) start: u32,
    pub(super) end: u32,
    pub(super) throw_range: Option<AddressRange>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MoveKind {
    Narrow,
    Wide,
    Object,
}

#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
pub(super) fn emit_instruction<R: DexMlilReferenceResolver>(
    planner: &mut Planner,
    instruction: &Instruction,
    allocation: &RegisterAllocation,
    file: &DexFile,
    resolver: &mut R,
    function: &Function,
    block: BlockId,
) -> Result<Emission> {
    let start = planner.cursor();
    let mut throw_range = None;
    match instruction.operation() {
        Operation::Nop | Operation::Discard => plain(planner, Opcode::Nop, Operands::None)?,
        Operation::Copy => {
            let uses = stage_uses(planner, instruction, allocation)?;
            store_single(planner, instruction, allocation, uses[0])?;
        }
        Operation::ParallelCopy | Operation::TypeRefine => {
            let staged = stage_uses(planner, instruction, allocation)?;
            for ((&definition, value_type), &scratch) in instruction
                .defs()
                .iter()
                .zip(instruction.def_types())
                .zip(&staged)
            {
                move_value(
                    planner,
                    allocation.register(definition),
                    scratch,
                    move_kind(value_type, instruction.id())?,
                )?;
            }
        }
        Operation::Constant(constant) => {
            throw_range = primary(planner, instruction, |planner| {
                emit_constant(planner, constant, instruction, file, resolver)
            })?;
            store_single(planner, instruction, allocation, 0)?;
        }
        Operation::Unary(operator) => {
            stage_uses(planner, instruction, allocation)?;
            throw_range = primary(planner, instruction, |planner| {
                plain(
                    planner,
                    unary_opcode(*operator, &instruction.use_types()[0]),
                    Operands::Registers {
                        first: 0,
                        second: 0,
                    },
                )
            })?;
            store_single(planner, instruction, allocation, 0)?;
        }
        Operation::Binary(operator) => {
            let staged = stage_uses(planner, instruction, allocation)?;
            let (first, second) = if *operator == BinaryOperator::ReverseSubtract {
                (staged[1], staged[0])
            } else {
                (staged[0], staged[1])
            };
            throw_range = primary(planner, instruction, |planner| {
                plain(
                    planner,
                    binary_opcode(*operator, &instruction.use_types()[0], instruction.id())?,
                    Operands::ThreeRegisters {
                        first: 0,
                        second: first,
                        third: second,
                    },
                )
            })?;
            store_single(planner, instruction, allocation, 0)?;
        }
        Operation::Convert(conversion) => {
            stage_uses(planner, instruction, allocation)?;
            throw_range = primary(planner, instruction, |planner| {
                plain(
                    planner,
                    conversion_opcode(*conversion),
                    Operands::Registers {
                        first: 0,
                        second: 0,
                    },
                )
            })?;
            store_single(planner, instruction, allocation, 0)?;
        }
        Operation::Compare(comparison) => {
            let staged = stage_uses(planner, instruction, allocation)?;
            throw_range = primary(planner, instruction, |planner| {
                plain(
                    planner,
                    comparison_opcode(*comparison),
                    Operands::ThreeRegisters {
                        first: 0,
                        second: staged[0],
                        third: staged[1],
                    },
                )
            })?;
            store_single(planner, instruction, allocation, 0)?;
        }
        Operation::Branch(predicate) => {
            let staged = stage_uses(planner, instruction, allocation)?;
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
            let opcode = inverted_branch(branch_opcode(*predicate));
            let second = matches!(
                predicate.operands,
                BranchOperandKind::IntegerPair | BranchOperandKind::ReferencePair
            )
            .then_some(staged[1]);
            planner.conditional_skip(opcode, staged[0], second)?;
            planner.goto(taken)?;
            planner.goto(fallback)?;
        }
        Operation::Jump => {
            let target = target(
                function,
                block,
                |role| matches!(role, ::mlil::EdgeRole::Jump),
                instruction.id(),
            )?;
            planner.goto(target)?;
        }
        Operation::Switch(keys) => {
            let staged = stage_uses(planner, instruction, allocation)?;
            let fallback = target(
                function,
                block,
                |role| matches!(role, ::mlil::EdgeRole::SwitchDefault),
                instruction.id(),
            )?;
            let mut native_keys = Vec::with_capacity(keys.len());
            let mut targets = Vec::with_capacity(keys.len());
            for &key in keys {
                native_keys.push(i32::try_from(key).map_err(|_| {
                    Error::lowering(instruction.id(), "Dalvik switch key exceeds i32")
                })?);
                targets.push(target(
                    function,
                    block,
                    |role| matches!(role, ::mlil::EdgeRole::SwitchCase(value) if *value == key),
                    instruction.id(),
                )?);
            }
            planner.switch(staged[0], native_keys, targets)?;
            planner.goto(fallback)?;
        }
        Operation::Return => {
            let opcode = instruction
                .use_types()
                .first()
                .map_or(Opcode::ReturnVoid, return_opcode);
            let operands = if instruction.uses().is_empty() {
                Operands::None
            } else {
                let staged = stage_uses(planner, instruction, allocation)?;
                Operands::Register(staged[0])
            };
            plain(planner, opcode, operands)?;
        }
        Operation::Throw => {
            let staged = stage_uses(planner, instruction, allocation)?;
            throw_range = primary(planner, instruction, |planner| {
                plain(planner, Opcode::Throw, Operands::Register(staged[0]))
            })?;
        }
        Operation::Array { access, element } => {
            let staged = stage_uses(planner, instruction, allocation)?;
            throw_range = primary(planner, instruction, |planner| {
                let operands = match access {
                    ArrayAccess::Get => Operands::ThreeRegisters {
                        first: 0,
                        second: staged[0],
                        third: staged[1],
                    },
                    ArrayAccess::Put => Operands::ThreeRegisters {
                        first: staged[2],
                        second: staged[0],
                        third: staged[1],
                    },
                };
                plain(planner, array_opcode(*access, *element), operands)
            })?;
            if *access == ArrayAccess::Get {
                store_single(planner, instruction, allocation, 0)?;
            }
        }
        Operation::ArrayLength => {
            let staged = stage_uses(planner, instruction, allocation)?;
            throw_range = primary(planner, instruction, |planner| {
                plain(
                    planner,
                    Opcode::ArrayLength,
                    Operands::Registers {
                        first: 0,
                        second: staged[0],
                    },
                )
            })?;
            store_single(planner, instruction, allocation, 0)?;
        }
        Operation::Field { access, field } => {
            let staged = stage_uses(planner, instruction, allocation)?;
            let index = resolve(file, resolver, field, IndexKind::Field, instruction.id())?;
            require_u16_index(index, instruction.id(), "field")?;
            let opcode = field_opcode(*access, field, instruction);
            throw_range = primary(planner, instruction, |planner| {
                let operands = match access {
                    FieldAccess::GetInstance => Operands::RegistersIndex {
                        first: 0,
                        second: staged[0],
                        index,
                    },
                    FieldAccess::PutInstance => Operands::RegistersIndex {
                        first: staged[1],
                        second: staged[0],
                        index,
                    },
                    FieldAccess::GetStatic => Operands::RegisterIndex { register: 0, index },
                    FieldAccess::PutStatic => Operands::RegisterIndex {
                        register: staged[0],
                        index,
                    },
                };
                plain(planner, opcode, operands)
            })?;
            if matches!(access, FieldAccess::GetInstance | FieldAccess::GetStatic) {
                store_single(planner, instruction, allocation, 0)?;
            }
        }
        Operation::Call {
            kind,
            target: reference,
            descriptor,
        } => {
            stage_uses(planner, instruction, allocation)?;
            let descriptor = descriptor.as_deref().ok_or_else(|| {
                Error::lowering(
                    instruction.id(),
                    "Dalvik call lacks an effective descriptor",
                )
            })?;
            let expected = if *kind == CallKind::Dynamic {
                IndexKind::CallSite
            } else {
                IndexKind::Method
            };
            let index = resolve(file, resolver, reference, expected, instruction.id())?;
            if *kind == CallKind::Polymorphic {
                require_polymorphic_target(file, index, instruction.id())?;
            }
            require_u16_index(index, instruction.id(), "call")?;
            let count =
                u8::try_from(words(instruction.use_types(), instruction.id())?).map_err(|_| {
                    Error::lowering(instruction.id(), "Dalvik call argument width exceeds 255")
                })?;
            let secondary_index = if *kind == CallKind::Polymorphic {
                let index = resolver
                    .resolve_prototype(file, descriptor)
                    .map_err(|source| Error::Reference {
                        instruction: instruction.id(),
                        source,
                    })?
                    .get();
                require_u16_index(index, instruction.id(), "prototype")?;
                Some(index)
            } else {
                None
            };
            throw_range = primary(planner, instruction, |planner| {
                plain(
                    planner,
                    call_opcode(*kind),
                    Operands::RegisterRangeIndex {
                        start: 0,
                        count,
                        index,
                        secondary_index,
                    },
                )
            })?;
            emit_result(planner, instruction, allocation)?;
        }
        Operation::Allocate(kind) => {
            emit_allocation(
                planner,
                instruction,
                allocation,
                file,
                resolver,
                kind,
                &mut throw_range,
            )?;
        }
        Operation::InitializeArray { array_type, values } => {
            let staged = stage_uses(planner, instruction, allocation)?;
            let payload = array_payload(array_type, values, instruction.id())?;
            throw_range = primary(planner, instruction, |planner| {
                planner.fill_array(staged[0], payload).map(drop)
            })?;
        }
        Operation::CheckCast(reference) => {
            let staged = stage_uses(planner, instruction, allocation)?;
            let index = resolve(file, resolver, reference, IndexKind::Type, instruction.id())?;
            require_u16_index(index, instruction.id(), "type")?;
            throw_range = primary(planner, instruction, |planner| {
                plain(
                    planner,
                    Opcode::CheckCast,
                    Operands::RegisterIndex {
                        register: staged[0],
                        index,
                    },
                )
            })?;
            store_single(planner, instruction, allocation, staged[0])?;
        }
        Operation::InstanceOf(reference) => {
            let staged = stage_uses(planner, instruction, allocation)?;
            let index = resolve(file, resolver, reference, IndexKind::Type, instruction.id())?;
            require_u16_index(index, instruction.id(), "type")?;
            throw_range = primary(planner, instruction, |planner| {
                plain(
                    planner,
                    Opcode::InstanceOf,
                    Operands::RegistersIndex {
                        first: 0,
                        second: staged[0],
                        index,
                    },
                )
            })?;
            store_single(planner, instruction, allocation, 0)?;
        }
        Operation::Monitor(action) => {
            let staged = stage_uses(planner, instruction, allocation)?;
            throw_range = primary(planner, instruction, |planner| {
                plain(
                    planner,
                    match action {
                        MonitorAction::Enter => Opcode::MonitorEnter,
                        MonitorAction::Exit => Opcode::MonitorExit,
                    },
                    Operands::Register(staged[0]),
                )
            })?;
        }
        Operation::CaughtException(_) => {
            plain(planner, Opcode::MoveException, Operands::Register(0))?;
            store_single(planner, instruction, allocation, 0)?;
        }
        Operation::Intrinsic(name) => {
            return Err(Error::lowering(
                instruction.id(),
                format!("Dalvik backend does not encode MLIL intrinsic `{name}`"),
            ));
        }
    }
    Ok(Emission {
        start,
        end: planner.cursor(),
        throw_range,
    })
}

fn stage_uses(
    planner: &mut Planner,
    instruction: &Instruction,
    allocation: &RegisterAllocation,
) -> Result<Vec<u16>> {
    let mut cursor = 0u16;
    let mut staged = Vec::with_capacity(instruction.uses().len());
    for (&variable, value_type) in instruction.uses().iter().zip(instruction.use_types()) {
        staged.push(cursor);
        move_value(
            planner,
            cursor,
            allocation.register(variable),
            move_kind(value_type, instruction.id())?,
        )?;
        cursor = cursor.checked_add(width(value_type)).ok_or_else(|| {
            Error::lowering(instruction.id(), "Dalvik scratch allocation overflow")
        })?;
    }
    Ok(staged)
}

fn store_single(
    planner: &mut Planner,
    instruction: &Instruction,
    allocation: &RegisterAllocation,
    scratch: u16,
) -> Result<()> {
    let Some((&definition, value_type)) = instruction
        .defs()
        .first()
        .zip(instruction.def_types().first())
    else {
        return Ok(());
    };
    move_value(
        planner,
        allocation.register(definition),
        scratch,
        move_kind(value_type, instruction.id())?,
    )
}

fn emit_result(
    planner: &mut Planner,
    instruction: &Instruction,
    allocation: &RegisterAllocation,
) -> Result<()> {
    let Some(value_type) = instruction.def_types().first() else {
        return Ok(());
    };
    plain(
        planner,
        match move_kind(value_type, instruction.id())? {
            MoveKind::Narrow => Opcode::MoveResult,
            MoveKind::Wide => Opcode::MoveResultWide,
            MoveKind::Object => Opcode::MoveResultObject,
        },
        Operands::Register(0),
    )?;
    store_single(planner, instruction, allocation, 0)
}

fn move_value(planner: &mut Planner, destination: u16, source: u16, kind: MoveKind) -> Result<()> {
    if destination == source {
        return Ok(());
    }
    let opcode = match (kind, destination < 16 && source < 16, destination < 256) {
        (MoveKind::Narrow, true, _) => Opcode::Move,
        (MoveKind::Wide, true, _) => Opcode::MoveWide,
        (MoveKind::Object, true, _) => Opcode::MoveObject,
        (MoveKind::Narrow, false, true) => Opcode::MoveFrom16,
        (MoveKind::Wide, false, true) => Opcode::MoveWideFrom16,
        (MoveKind::Object, false, true) => Opcode::MoveObjectFrom16,
        (MoveKind::Narrow, false, false) => Opcode::Move16,
        (MoveKind::Wide, false, false) => Opcode::MoveWide16,
        (MoveKind::Object, false, false) => Opcode::MoveObject16,
    };
    plain(
        planner,
        opcode,
        Operands::Registers {
            first: destination,
            second: source,
        },
    )
}

fn primary(
    planner: &mut Planner,
    instruction: &Instruction,
    emit: impl FnOnce(&mut Planner) -> Result<()>,
) -> Result<Option<AddressRange>> {
    let start = planner.cursor();
    emit(planner)?;
    let end = planner.cursor();
    Ok(instruction.may_throw().then_some(AddressRange::new(
        CodeAddress::from(start),
        CodeAddress::from(end),
    )))
}

fn emit_constant<R: DexMlilReferenceResolver>(
    planner: &mut Planner,
    constant: &Constant,
    instruction: &Instruction,
    file: &DexFile,
    resolver: &mut R,
) -> Result<()> {
    match constant {
        Constant::Null => const_narrow(planner, 0),
        Constant::Integer(value) => const_narrow(planner, *value),
        Constant::Long(value) => const_wide(planner, *value),
        Constant::Float(bits) => const_narrow(planner, i32::from_ne_bytes(bits.to_ne_bytes())),
        Constant::Double(bits) => const_wide(planner, i64::from_ne_bytes(bits.to_ne_bytes())),
        Constant::Reference(reference) => {
            let (expected, opcode, jumbo) = match reference.kind {
                ReferenceKind::String => (IndexKind::String, Opcode::ConstString, true),
                ReferenceKind::Type => (IndexKind::Type, Opcode::ConstClass, false),
                ReferenceKind::MethodHandle => {
                    (IndexKind::MethodHandle, Opcode::ConstMethodHandle, false)
                }
                ReferenceKind::MethodPrototype => {
                    (IndexKind::Prototype, Opcode::ConstMethodType, false)
                }
                _ => {
                    return Err(Error::lowering(
                        instruction.id(),
                        "reference constant has no Dalvik const opcode",
                    ));
                }
            };
            let index = resolve(file, resolver, reference, expected, instruction.id())?;
            if jumbo && u16::try_from(index).is_err() {
                plain(
                    planner,
                    Opcode::ConstStringJumbo,
                    Operands::RegisterIndex { register: 0, index },
                )
            } else {
                require_u16_index(index, instruction.id(), "constant")?;
                plain(
                    planner,
                    opcode,
                    Operands::RegisterIndex { register: 0, index },
                )
            }
        }
    }
}

fn const_narrow(planner: &mut Planner, value: i32) -> Result<()> {
    plain(
        planner,
        Opcode::Const,
        Operands::RegisterLiteral {
            register: 0,
            literal: i64::from(value),
        },
    )
}

fn const_wide(planner: &mut Planner, value: i64) -> Result<()> {
    plain(
        planner,
        Opcode::ConstWide,
        Operands::RegisterLiteral {
            register: 0,
            literal: value,
        },
    )
}

fn array_payload(
    array_type: &ArrayType,
    values: &[Constant],
    instruction: InstructionId,
) -> Result<ArrayDataPayload> {
    let component = array_type
        .descriptor()
        .strip_prefix('[')
        .and_then(|descriptor| descriptor.as_bytes().first())
        .copied()
        .ok_or_else(|| Error::lowering(instruction, "array type has no component descriptor"))?;
    let element_width = match component {
        b'Z' | b'B' => 1,
        b'C' | b'S' => 2,
        b'I' | b'F' => 4,
        b'J' | b'D' => 8,
        _ => {
            return Err(Error::lowering(
                instruction,
                "Dalvik fill-array-data requires primitive element constants",
            ));
        }
    };
    let mut data = Vec::with_capacity(values.len().saturating_mul(usize::from(element_width)));
    for value in values {
        match (component, value) {
            (b'Z' | b'B', Constant::Integer(value)) => {
                data.extend_from_slice(&value.to_le_bytes()[..1]);
            }
            (b'C' | b'S', Constant::Integer(value)) => {
                data.extend_from_slice(&value.to_le_bytes()[..2]);
            }
            (b'I', Constant::Integer(value)) => data.extend_from_slice(&value.to_le_bytes()),
            (b'F', Constant::Float(value)) => data.extend_from_slice(&value.to_le_bytes()),
            (b'J', Constant::Long(value)) => data.extend_from_slice(&value.to_le_bytes()),
            (b'D', Constant::Double(value)) => data.extend_from_slice(&value.to_le_bytes()),
            _ => {
                return Err(Error::lowering(
                    instruction,
                    "fill-array constant kind disagrees with its semantic array type",
                ));
            }
        }
    }
    Ok(ArrayDataPayload {
        element_width,
        element_count: u32::try_from(values.len())
            .map_err(|_| Error::lowering(instruction, "fill-array element count exceeds u32"))?,
        data,
    })
}

#[allow(clippy::too_many_arguments)]
fn emit_allocation<R: DexMlilReferenceResolver>(
    planner: &mut Planner,
    instruction: &Instruction,
    allocation: &RegisterAllocation,
    file: &DexFile,
    resolver: &mut R,
    kind: &AllocationKind,
    throw_range: &mut Option<AddressRange>,
) -> Result<()> {
    let staged = stage_uses(planner, instruction, allocation)?;
    match kind {
        AllocationKind::Object(reference) => {
            let index = resolve(file, resolver, reference, IndexKind::Type, instruction.id())?;
            require_u16_index(index, instruction.id(), "type")?;
            *throw_range = primary(planner, instruction, |planner| {
                plain(
                    planner,
                    Opcode::NewInstance,
                    Operands::RegisterIndex { register: 0, index },
                )
            })?;
            store_single(planner, instruction, allocation, 0)?;
        }
        AllocationKind::Array {
            array_type,
            dimensions: 1,
        } => {
            let descriptor = array_type.descriptor();
            let index = resolver
                .resolve_type(file, descriptor)
                .map_err(|source| Error::Reference {
                    instruction: instruction.id(),
                    source,
                })?
                .get();
            require_u16_index(index, instruction.id(), "array type")?;
            *throw_range = primary(planner, instruction, |planner| {
                plain(
                    planner,
                    Opcode::NewArray,
                    Operands::RegistersIndex {
                        first: 0,
                        second: staged[0],
                        index,
                    },
                )
            })?;
            store_single(planner, instruction, allocation, 0)?;
        }
        AllocationKind::InitializedArray { array_type } => {
            let descriptor = array_type.descriptor();
            if matches!(
                descriptor
                    .strip_prefix('[')
                    .and_then(|value| value.as_bytes().first()),
                Some(b'J' | b'D')
            ) {
                return Err(Error::lowering(
                    instruction.id(),
                    "Dalvik filled-new-array cannot contain wide primitive elements",
                ));
            }
            let index = resolver
                .resolve_type(file, descriptor)
                .map_err(|source| Error::Reference {
                    instruction: instruction.id(),
                    source,
                })?
                .get();
            require_u16_index(index, instruction.id(), "array type")?;
            let count =
                u8::try_from(words(instruction.use_types(), instruction.id())?).map_err(|_| {
                    Error::lowering(instruction.id(), "filled-array operand width exceeds 255")
                })?;
            *throw_range = primary(planner, instruction, |planner| {
                plain(
                    planner,
                    Opcode::FilledNewArrayRange,
                    Operands::RegisterRangeIndex {
                        start: 0,
                        count,
                        index,
                        secondary_index: None,
                    },
                )
            })?;
            emit_result(planner, instruction, allocation)?;
        }
        AllocationKind::Array { .. } => {
            return Err(Error::lowering(
                instruction.id(),
                "Dalvik array allocation requires one indexed array descriptor dimension",
            ));
        }
    }
    Ok(())
}

fn resolve<R: DexMlilReferenceResolver>(
    file: &DexFile,
    resolver: &mut R,
    reference: &disassembler::Reference,
    expected: IndexKind,
    instruction: InstructionId,
) -> Result<u32> {
    resolver
        .resolve(file, reference, expected)
        .map_err(|source| Error::Reference {
            instruction,
            source,
        })
}

fn require_u16_index(index: u32, instruction: InstructionId, kind: &str) -> Result<()> {
    u16::try_from(index)
        .map(drop)
        .map_err(|_| Error::lowering(instruction, format!("Dalvik {kind} index exceeds u16")))
}

const METHOD_HANDLE_DESCRIPTOR: &str = "Ljava/lang/invoke/MethodHandle;";
const METHOD_HANDLE_INVOKE_NAME: &str = "invoke";
const METHOD_HANDLE_INVOKE_EXACT_NAME: &str = "invokeExact";
const METHOD_HANDLE_DECLARED_DESCRIPTOR: &str = "([Ljava/lang/Object;)Ljava/lang/Object;";

fn require_polymorphic_target(
    file: &DexFile,
    index: u32,
    instruction: InstructionId,
) -> Result<()> {
    let method = file.resolve_method(MethodIndex::new(index))?;
    if method.owner == METHOD_HANDLE_DESCRIPTOR
        && matches!(
            method.name,
            METHOD_HANDLE_INVOKE_NAME | METHOD_HANDLE_INVOKE_EXACT_NAME
        )
        && method.signature == METHOD_HANDLE_DECLARED_DESCRIPTOR
    {
        Ok(())
    } else {
        Err(Error::lowering(
            instruction,
            "Dalvik signature-polymorphic dispatch requires MethodHandle.invoke or invokeExact",
        ))
    }
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

fn move_kind(value_type: &ValueType, instruction: InstructionId) -> Result<MoveKind> {
    match value_type {
        ValueType::Boolean
        | ValueType::Integer
        | ValueType::Float
        | ValueType::Bits32
        | ValueType::Zero => Ok(MoveKind::Narrow),
        ValueType::Long | ValueType::Double | ValueType::Bits64 => Ok(MoveKind::Wide),
        ValueType::Null
        | ValueType::Reference(_)
        | ValueType::UninitializedThis(_)
        | ValueType::Uninitialized { .. } => Ok(MoveKind::Object),
        ValueType::Unknown | ValueType::Conflict | ValueType::ReturnAddress => {
            Err(Error::lowering(
                instruction,
                format!("value type {value_type:?} has no canonical Dalvik register category"),
            ))
        }
    }
}

fn plain(planner: &mut Planner, opcode: Opcode, operands: Operands) -> Result<()> {
    planner.plain(opcode, operands).map(drop)
}

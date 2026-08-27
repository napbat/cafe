//! Semantic lifting of individual Dalvik LLIL operations.

use ::mlil::{
    AllocationKind, ArrayAccess, ArrayType, BinaryOperator, BranchOperandKind, BranchPredicate,
    CallKind, Constant, Conversion, ElementType, FieldAccess, MonitorAction, Operation, Relation,
    ThreeWayComparison, TypedVariable, UnaryOperator, ValueType, VariableRole,
};
use disassembler::{Reference, ReferenceSymbol};

use crate::analysis::{RegisterFrame, RegisterType, ValueKind};
use crate::file::DexFile;
use crate::instruction::Instruction as NativeInstruction;
use crate::llil::{self, InstructionKind, Operand, OperationKind, Payload};

use super::reference;
use super::state::{StateVariables, VariableAllocator, value_kind};
use super::{Error, Result};

pub(crate) struct Step {
    pub(crate) operation: Operation,
    pub(crate) uses: Vec<TypedVariable>,
    pub(crate) defs: Vec<TypedVariable>,
}

pub(crate) struct LiftedInstruction {
    pub(crate) steps: Vec<Step>,
    pub(crate) throw_step: usize,
}

#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
pub(crate) fn lift_instruction(
    builder: &mut (impl VariableAllocator + ?Sized),
    variables: &StateVariables,
    file: &DexFile,
    native: &NativeInstruction,
    instruction: &llil::Instruction,
    entry: Option<&RegisterFrame>,
    exit: Option<&RegisterFrame>,
    body: &llil::Body,
) -> Result<LiftedInstruction> {
    let InstructionKind::Operation(operation) = &instruction.kind else {
        return Err(Error::unsupported(
            instruction.offset,
            "payload passed to executable MLIL lifting",
        ));
    };
    let uses = register_uses(variables, operation, entry);
    let defs = register_defs(variables, operation, exit);
    let result = match &operation.kind {
        OperationKind::Nop => one(Operation::Nop, vec![], vec![]),
        OperationKind::Move(_) => one(Operation::Copy, uses, defs),
        OperationKind::MoveResult(kind) => {
            let result_type = defs.first().map_or_else(
                || value_kind(kind),
                |definition| definition.value_type.clone(),
            );
            one(
                Operation::Copy,
                vec![TypedVariable::new(variables.result, result_type)],
                defs,
            )
        }
        OperationKind::MoveException => {
            let exception_type = defs
                .first()
                .map_or(ValueType::Reference(None), |definition| {
                    definition.value_type.clone()
                });
            one(
                Operation::Copy,
                vec![TypedVariable::new(variables.exception, exception_type)],
                defs,
            )
        }
        OperationKind::Return(_) => one(Operation::Return, uses, vec![]),
        OperationKind::Constant(kind) => {
            let constant = lift_constant(file, native, operation, *kind)?;
            one(Operation::Constant(constant), vec![], defs)
        }
        OperationKind::Monitor(action) => one(
            Operation::Monitor(match action {
                llil::MonitorAction::Enter => MonitorAction::Enter,
                llil::MonitorAction::Exit => MonitorAction::Exit,
            }),
            uses,
            vec![],
        ),
        OperationKind::CheckCast => one(
            Operation::CheckCast(reference::type_reference(
                file,
                native,
                reference_index(operation, instruction.offset)?,
            )?),
            uses,
            defs,
        ),
        OperationKind::InstanceOf => one(
            Operation::InstanceOf(reference::type_reference(
                file,
                native,
                reference_index(operation, instruction.offset)?,
            )?),
            uses,
            defs,
        ),
        OperationKind::ArrayLength => one(Operation::ArrayLength, uses, defs),
        OperationKind::NewInstance => one(
            Operation::Allocate(AllocationKind::Object(reference::type_reference(
                file,
                native,
                reference_index(operation, instruction.offset)?,
            )?)),
            vec![],
            defs,
        ),
        OperationKind::NewArray => {
            let source = reference::type_reference(
                file,
                native,
                reference_index(operation, instruction.offset)?,
            )?;
            one(
                Operation::Allocate(AllocationKind::Array {
                    array_type: array_type_from_reference(source, instruction.offset)?,
                    dimensions: 1,
                }),
                uses,
                defs,
            )
        }
        OperationKind::FilledNewArray => {
            let array_type = reference::type_reference(
                file,
                native,
                reference_index(operation, instruction.offset)?,
            )?;
            let semantic_type = array_type_from_reference(array_type.clone(), instruction.offset)?;
            one(
                Operation::Allocate(AllocationKind::InitializedArray {
                    array_type: semantic_type.clone(),
                }),
                uses,
                vec![TypedVariable::new(
                    variables.result,
                    ValueType::Reference(Some(semantic_type.descriptor().to_owned())),
                )],
            )
        }
        OperationKind::FillArrayData => {
            let Payload::ArrayData(payload) =
                payload(body, target(operation, instruction.offset)?)?
            else {
                return Err(Error::unsupported(
                    instruction.offset,
                    "fill-array-data target is not an array payload",
                ));
            };
            let array_type = array_type_from_use(&uses, instruction.offset)?;
            let values = decode_array_values(&array_type, payload, instruction.offset)?;
            one(
                Operation::InitializeArray { array_type, values },
                uses,
                vec![],
            )
        }
        OperationKind::Throw => one(Operation::Throw, uses, vec![]),
        OperationKind::Jump => one(Operation::Jump, vec![], vec![]),
        OperationKind::Switch => {
            let keys = switch_keys(
                payload(body, target(operation, instruction.offset)?)?,
                instruction.offset,
            )?;
            one(Operation::Switch(keys), uses, vec![])
        }
        OperationKind::Compare(comparison) => {
            one(Operation::Compare(comparison_kind(*comparison)), uses, defs)
        }
        OperationKind::BranchPair(relation) => {
            let operand_kind = if uses.iter().any(|value| value.value_type.is_reference())
                && uses.iter().all(|value| {
                    value.value_type.is_reference() || matches!(&value.value_type, ValueType::Zero)
                }) {
                BranchOperandKind::ReferencePair
            } else {
                BranchOperandKind::IntegerPair
            };
            one(
                Operation::Branch(BranchPredicate {
                    relation: relation_kind(*relation),
                    operands: operand_kind,
                }),
                uses,
                vec![],
            )
        }
        OperationKind::BranchZero(relation) => {
            let operand_kind = if uses[0].value_type.is_reference() {
                BranchOperandKind::ReferenceNull
            } else {
                BranchOperandKind::IntegerZero
            };
            one(
                Operation::Branch(BranchPredicate {
                    relation: relation_kind(*relation),
                    operands: operand_kind,
                }),
                uses,
                vec![],
            )
        }
        OperationKind::Array { access, element } => {
            let access = match access {
                llil::ArrayAccess::Get => ArrayAccess::Get,
                llil::ArrayAccess::Put => ArrayAccess::Put,
            };
            one(
                Operation::Array {
                    access,
                    element: element_kind(*element),
                },
                ordered_array_uses(access, uses),
                defs,
            )
        }
        OperationKind::Field { access, .. } => {
            let access = match access {
                llil::FieldAccess::GetInstance => FieldAccess::GetInstance,
                llil::FieldAccess::PutInstance => FieldAccess::PutInstance,
                llil::FieldAccess::GetStatic => FieldAccess::GetStatic,
                llil::FieldAccess::PutStatic => FieldAccess::PutStatic,
            };
            one(
                Operation::Field {
                    access,
                    field: reference::field_reference(
                        file,
                        native,
                        reference_index(operation, instruction.offset)?,
                    )?
                    .0,
                },
                ordered_field_uses(access, uses),
                defs,
            )
        }
        OperationKind::Invoke(invocation) => lift_call(
            variables,
            file,
            native,
            operation,
            *invocation,
            uses,
            entry,
            exit,
        )?,
        OperationKind::Unary { operator, .. } => one(
            Operation::Unary(match operator {
                llil::UnaryOperator::Negate => UnaryOperator::Negate,
                llil::UnaryOperator::Not => UnaryOperator::BitwiseNot,
            }),
            uses,
            defs,
        ),
        OperationKind::Convert(conversion) => {
            one(Operation::Convert(conversion_kind(*conversion)), uses, defs)
        }
        OperationKind::Binary { operator, .. } => lift_binary(
            builder,
            operation,
            *operator,
            uses,
            defs,
            instruction.offset,
        )?,
    };
    Ok(result)
}

#[allow(clippy::too_many_arguments)]
fn lift_call(
    variables: &StateVariables,
    file: &DexFile,
    native: &NativeInstruction,
    operation: &llil::Operation,
    invocation: llil::Invocation,
    mut uses: Vec<TypedVariable>,
    entry: Option<&RegisterFrame>,
    exit: Option<&RegisterFrame>,
) -> Result<LiftedInstruction> {
    let (target, descriptor) = reference::call_reference(
        file,
        native,
        invocation,
        reference_index(operation, native.offset())?,
    )?;
    if entry.is_none() {
        uses = type_unreachable_call_uses(invocation, &descriptor, uses, native.offset())?;
    }
    let mut steps = vec![Step {
        operation: Operation::Call {
            kind: match invocation {
                llil::Invocation::Virtual => CallKind::Virtual,
                llil::Invocation::Super => CallKind::Super,
                llil::Invocation::Direct => CallKind::Direct,
                llil::Invocation::Static => CallKind::Static,
                llil::Invocation::Interface => CallKind::Interface,
                llil::Invocation::Polymorphic => CallKind::Polymorphic,
                llil::Invocation::Custom => CallKind::Dynamic,
            },
            target,
            descriptor: Some(descriptor.clone()),
        },
        uses,
        defs: return_type(&descriptor).map_or_else(Vec::new, |value_type| {
            vec![TypedVariable::new(variables.result, value_type)]
        }),
    }];
    if let Some(refinement) = initialization_refinement(variables, entry, exit) {
        steps.push(refinement);
    }
    Ok(LiftedInstruction {
        steps,
        throw_step: 0,
    })
}

fn array_type_from_reference(reference: Reference, offset: u32) -> Result<ArrayType> {
    let Some(ReferenceSymbol::Type(descriptor)) = &reference.symbol else {
        return Err(Error::unsupported(
            offset,
            "array type operand lacks a structured descriptor",
        ));
    };
    if !descriptor.starts_with('[') {
        return Err(Error::unsupported(
            offset,
            "array type operand is not an array descriptor",
        ));
    }
    Ok(ArrayType::new(descriptor.clone()).with_source_reference(reference))
}

fn array_type_from_use(uses: &[TypedVariable], offset: u32) -> Result<ArrayType> {
    let Some(TypedVariable {
        value_type: ValueType::Reference(Some(descriptor)),
        ..
    }) = uses.first()
    else {
        return Err(Error::unsupported(
            offset,
            "fill-array-data lacks an exact semantic array type",
        ));
    };
    if !descriptor.starts_with('[') {
        return Err(Error::unsupported(
            offset,
            "fill-array-data operand is not an array descriptor",
        ));
    }
    Ok(ArrayType::new(descriptor.clone()))
}

fn decode_array_values(
    array_type: &ArrayType,
    payload: &crate::instruction::ArrayDataPayload,
    offset: u32,
) -> Result<Vec<Constant>> {
    let component = array_type
        .descriptor()
        .strip_prefix('[')
        .and_then(|descriptor| descriptor.as_bytes().first())
        .copied()
        .ok_or_else(|| Error::unsupported(offset, "array descriptor has no component type"))?;
    let expected_width = match component {
        b'Z' | b'B' => 1,
        b'C' | b'S' => 2,
        b'I' | b'F' => 4,
        b'J' | b'D' => 8,
        _ => {
            return Err(Error::unsupported(
                offset,
                "fill-array-data requires a primitive array type",
            ));
        }
    };
    if payload.element_width != expected_width {
        return Err(Error::unsupported(
            offset,
            "fill-array-data width disagrees with its semantic array type",
        ));
    }
    let width = usize::from(expected_width);
    let expected_len = usize::try_from(payload.element_count)
        .ok()
        .and_then(|count| count.checked_mul(width));
    if expected_len != Some(payload.data.len()) {
        return Err(Error::unsupported(
            offset,
            "fill-array-data element count disagrees with its payload",
        ));
    }
    payload
        .data
        .chunks_exact(width)
        .map(|bytes| {
            Ok(match component {
                b'Z' | b'B' => Constant::Integer(i32::from(i8::from_le_bytes([bytes[0]]))),
                b'C' => Constant::Integer(i32::from(u16::from_le_bytes([bytes[0], bytes[1]]))),
                b'S' => Constant::Integer(i32::from(i16::from_le_bytes([bytes[0], bytes[1]]))),
                b'I' => {
                    Constant::Integer(i32::from_le_bytes(bytes.try_into().map_err(|_| {
                        Error::unsupported(offset, "invalid 32-bit array element")
                    })?))
                }
                b'F' => {
                    Constant::Float(u32::from_le_bytes(bytes.try_into().map_err(|_| {
                        Error::unsupported(offset, "invalid float array element")
                    })?))
                }
                b'J' => {
                    Constant::Long(i64::from_le_bytes(bytes.try_into().map_err(|_| {
                        Error::unsupported(offset, "invalid 64-bit array element")
                    })?))
                }
                b'D' => {
                    Constant::Double(u64::from_le_bytes(bytes.try_into().map_err(|_| {
                        Error::unsupported(offset, "invalid double array element")
                    })?))
                }
                _ => unreachable!("component was checked above"),
            })
        })
        .collect()
}

fn initialization_refinement(
    variables: &StateVariables,
    entry: Option<&RegisterFrame>,
    exit: Option<&RegisterFrame>,
) -> Option<Step> {
    let (entry, exit) = (entry?, exit?);
    let pairs = entry
        .registers()
        .iter()
        .zip(exit.registers())
        .enumerate()
        .filter(|(_, (before, after))| initialized(before, after))
        .map(|(index, _)| {
            let register = u16::try_from(index).expect("DEX register index fits u16");
            let fallback = ValueKind::Reference;
            (
                variables.register(Some(entry), register, &fallback),
                variables.register(Some(exit), register, &fallback),
            )
        });
    let (uses, defs): (Vec<_>, Vec<_>) = pairs.unzip();
    (!uses.is_empty()).then_some(Step {
        operation: Operation::TypeRefine,
        uses,
        defs,
    })
}

fn initialized(before: &RegisterType, after: &RegisterType) -> bool {
    matches!(
        (before, after),
        (
            RegisterType::Reference(
                crate::analysis::ReferenceType::Uninitialized { .. }
                    | crate::analysis::ReferenceType::UninitializedThis { .. }
            ),
            RegisterType::Reference(
                crate::analysis::ReferenceType::Any | crate::analysis::ReferenceType::Descriptor(_)
            )
        )
    )
}

fn lift_binary(
    builder: &mut (impl VariableAllocator + ?Sized),
    operation: &llil::Operation,
    operator: llil::ArithmeticOperator,
    mut uses: Vec<TypedVariable>,
    defs: Vec<TypedVariable>,
    offset: u32,
) -> Result<LiftedInstruction> {
    let mut steps = Vec::new();
    if let Some(literal) = operation.operands.iter().find_map(|operand| match operand {
        Operand::Literal(value) => Some(*value),
        _ => None,
    }) {
        let value = i32::try_from(literal)
            .map_err(|_| Error::unsupported(offset, "narrow binary literal does not fit i32"))?;
        let temporary = builder.declare_variable(VariableRole::Temporary, None)?;
        steps.push(Step {
            operation: Operation::Constant(Constant::Integer(value)),
            uses: vec![],
            defs: vec![TypedVariable::new(temporary, ValueType::Integer)],
        });
        uses.push(TypedVariable::new(temporary, ValueType::Integer));
    }
    steps.push(Step {
        operation: Operation::Binary(binary_kind(operator)),
        uses,
        defs,
    });
    Ok(LiftedInstruction {
        throw_step: steps.len() - 1,
        steps,
    })
}

fn register_uses(
    variables: &StateVariables,
    operation: &llil::Operation,
    frame: Option<&RegisterFrame>,
) -> Vec<TypedVariable> {
    operation
        .operands
        .iter()
        .filter_map(|operand| match operand {
            Operand::Use(operand)
                if !frame.is_some_and(|frame| {
                    matches!(
                        frame.register(operand.register),
                        Some(RegisterType::WideContinuation)
                    )
                }) =>
            {
                Some(variables.register(frame, operand.register, &operand.kind))
            }
            _ => None,
        })
        .collect()
}

fn register_defs(
    variables: &StateVariables,
    operation: &llil::Operation,
    frame: Option<&RegisterFrame>,
) -> Vec<TypedVariable> {
    operation
        .operands
        .iter()
        .filter_map(|operand| match operand {
            Operand::Definition(operand) => {
                Some(variables.register(frame, operand.register, &operand.kind))
            }
            _ => None,
        })
        .collect()
}

fn lift_constant(
    file: &DexFile,
    native: &NativeInstruction,
    operation: &llil::Operation,
    kind: llil::ConstantKind,
) -> Result<Constant> {
    Ok(match kind {
        llil::ConstantKind::Narrow => {
            Constant::Integer(i32::try_from(literal(operation, native.offset())?).map_err(
                |_| Error::unsupported(native.offset(), "narrow DEX literal does not fit i32"),
            )?)
        }
        llil::ConstantKind::Wide => Constant::Long(literal(operation, native.offset())?),
        llil::ConstantKind::String
        | llil::ConstantKind::Class
        | llil::ConstantKind::MethodHandle
        | llil::ConstantKind::MethodType => Constant::Reference(reference::constant_reference(
            file,
            native,
            reference_index(operation, native.offset())?,
        )?),
    })
}

fn one(
    operation: Operation,
    uses: Vec<TypedVariable>,
    defs: Vec<TypedVariable>,
) -> LiftedInstruction {
    LiftedInstruction {
        steps: vec![Step {
            operation,
            uses,
            defs,
        }],
        throw_step: 0,
    }
}

fn literal(operation: &llil::Operation, offset: u32) -> Result<i64> {
    operation
        .operands
        .iter()
        .find_map(|operand| match operand {
            Operand::Literal(value) => Some(*value),
            _ => None,
        })
        .ok_or_else(|| Error::unsupported(offset, "operation lacks its literal operand"))
}

fn reference_index(operation: &llil::Operation, offset: u32) -> Result<u32> {
    operation
        .operands
        .iter()
        .find_map(|operand| match operand {
            Operand::Reference { index, .. } => Some(*index),
            _ => None,
        })
        .ok_or_else(|| Error::unsupported(offset, "operation lacks its identifier index"))
}

fn target(operation: &llil::Operation, offset: u32) -> Result<u32> {
    operation
        .operands
        .iter()
        .find_map(|operand| match operand {
            Operand::Target(target) => Some(*target),
            _ => None,
        })
        .ok_or_else(|| Error::unsupported(offset, "operation lacks its target operand"))
}

fn ordered_array_uses(access: ArrayAccess, mut uses: Vec<TypedVariable>) -> Vec<TypedVariable> {
    if access == ArrayAccess::Put && uses.len() == 3 {
        uses.rotate_left(1);
    }
    uses
}

fn ordered_field_uses(access: FieldAccess, mut uses: Vec<TypedVariable>) -> Vec<TypedVariable> {
    if access == FieldAccess::PutInstance && uses.len() == 2 {
        uses.swap(0, 1);
    }
    uses
}

fn payload(body: &llil::Body, target: u32) -> Result<&Payload> {
    body.instructions
        .iter()
        .find(|instruction| instruction.offset == target)
        .and_then(|instruction| match &instruction.kind {
            InstructionKind::Payload(payload) => Some(payload),
            InstructionKind::Operation(_) => None,
        })
        .ok_or(Error::MissingTarget {
            source_offset: target,
            target,
        })
}

fn switch_keys(payload: &Payload, offset: u32) -> Result<Vec<i64>> {
    Ok(match payload {
        Payload::PackedSwitch(payload) => (0..payload.targets.len())
            .map(|position| {
                i64::from(payload.first_key)
                    + i64::try_from(position).expect("DEX payload length fits i64")
            })
            .collect(),
        Payload::SparseSwitch(payload) => payload.keys.iter().map(|&key| i64::from(key)).collect(),
        Payload::ArrayData(_) => {
            return Err(Error::unsupported(
                offset,
                "switch target is an array payload",
            ));
        }
    })
}

fn return_type(descriptor: &str) -> Option<ValueType> {
    let Some(return_descriptor) = descriptor.split_once(')').map(|(_, value)| value) else {
        return Some(ValueType::Unknown);
    };
    Some(match return_descriptor.as_bytes().first() {
        Some(b'V') | None => return None,
        Some(b'J') => ValueType::Long,
        Some(b'F') => ValueType::Float,
        Some(b'D') => ValueType::Double,
        Some(b'L' | b'[') => ValueType::Reference(Some(return_descriptor.to_owned())),
        Some(_) => ValueType::Integer,
    })
}

fn type_unreachable_call_uses(
    invocation: llil::Invocation,
    descriptor: &str,
    uses: Vec<TypedVariable>,
    offset: u32,
) -> Result<Vec<TypedVariable>> {
    let parameters = parameter_types(descriptor)
        .ok_or_else(|| Error::unsupported(offset, "invocation has an invalid method descriptor"))?;
    let has_receiver = !matches!(
        invocation,
        llil::Invocation::Static | llil::Invocation::Custom
    );
    let expected_words =
        usize::from(has_receiver) + parameters.iter().map(|(_, width)| width).sum::<usize>();
    if uses.len() != expected_words {
        return Err(Error::unsupported(
            offset,
            "invocation register words disagree with its method descriptor",
        ));
    }
    let mut words = uses.into_iter();
    let mut typed = Vec::with_capacity(parameters.len() + usize::from(has_receiver));
    if has_receiver {
        let mut receiver = words.next().expect("validated invocation receiver word");
        receiver.value_type = ValueType::Reference(None);
        typed.push(receiver);
    }
    for (value_type, width) in parameters {
        let mut value = words.next().expect("validated invocation parameter word");
        value.value_type = value_type;
        typed.push(value);
        for _ in 1..width {
            let _ = words.next().expect("validated wide continuation word");
        }
    }
    Ok(typed)
}

fn parameter_types(descriptor: &str) -> Option<Vec<(ValueType, usize)>> {
    let bytes = descriptor.as_bytes();
    if bytes.first() != Some(&b'(') {
        return None;
    }
    let mut cursor = 1usize;
    let mut parameters = Vec::new();
    while bytes.get(cursor) != Some(&b')') {
        let start = cursor;
        let mut array = false;
        while bytes.get(cursor) == Some(&b'[') {
            array = true;
            cursor += 1;
        }
        let width = match bytes.get(cursor)? {
            b'L' => {
                cursor += 1;
                while bytes.get(cursor) != Some(&b';') {
                    cursor += 1;
                    let _ = bytes.get(cursor)?;
                }
                cursor += 1;
                1
            }
            b'J' | b'D' if !array => {
                cursor += 1;
                2
            }
            b'Z' | b'B' | b'C' | b'S' | b'I' | b'J' | b'F' | b'D' => {
                cursor += 1;
                1
            }
            _ => return None,
        };
        let value_type = if array {
            ValueType::Reference(Some(descriptor[start..cursor].to_owned()))
        } else {
            match bytes[start] {
                b'J' => ValueType::Long,
                b'F' => ValueType::Float,
                b'D' => ValueType::Double,
                b'L' => ValueType::Reference(Some(descriptor[start..cursor].to_owned())),
                _ => ValueType::Integer,
            }
        };
        parameters.push((value_type, width));
    }
    bytes.get(cursor + 1).map(|_| parameters)
}

fn relation_kind(relation: llil::Relation) -> Relation {
    match relation {
        llil::Relation::Equal => Relation::Equal,
        llil::Relation::NotEqual => Relation::NotEqual,
        llil::Relation::Less => Relation::Less,
        llil::Relation::GreaterOrEqual => Relation::GreaterOrEqual,
        llil::Relation::Greater => Relation::Greater,
        llil::Relation::LessOrEqual => Relation::LessOrEqual,
    }
}

fn comparison_kind(comparison: llil::Comparison) -> ThreeWayComparison {
    match comparison {
        llil::Comparison::FloatNanLow => ThreeWayComparison::FloatNanLow,
        llil::Comparison::FloatNanHigh => ThreeWayComparison::FloatNanHigh,
        llil::Comparison::DoubleNanLow => ThreeWayComparison::DoubleNanLow,
        llil::Comparison::DoubleNanHigh => ThreeWayComparison::DoubleNanHigh,
        llil::Comparison::Long => ThreeWayComparison::Long,
    }
}

fn conversion_kind(conversion: llil::Conversion) -> Conversion {
    match conversion {
        llil::Conversion::IntToLong => Conversion::IntToLong,
        llil::Conversion::IntToFloat => Conversion::IntToFloat,
        llil::Conversion::IntToDouble => Conversion::IntToDouble,
        llil::Conversion::LongToInt => Conversion::LongToInt,
        llil::Conversion::LongToFloat => Conversion::LongToFloat,
        llil::Conversion::LongToDouble => Conversion::LongToDouble,
        llil::Conversion::FloatToInt => Conversion::FloatToInt,
        llil::Conversion::FloatToLong => Conversion::FloatToLong,
        llil::Conversion::FloatToDouble => Conversion::FloatToDouble,
        llil::Conversion::DoubleToInt => Conversion::DoubleToInt,
        llil::Conversion::DoubleToLong => Conversion::DoubleToLong,
        llil::Conversion::DoubleToFloat => Conversion::DoubleToFloat,
        llil::Conversion::IntToByte => Conversion::IntToByte,
        llil::Conversion::IntToChar => Conversion::IntToChar,
        llil::Conversion::IntToShort => Conversion::IntToShort,
    }
}

fn binary_kind(operator: llil::ArithmeticOperator) -> BinaryOperator {
    match operator {
        llil::ArithmeticOperator::Add => BinaryOperator::Add,
        llil::ArithmeticOperator::Subtract => BinaryOperator::Subtract,
        llil::ArithmeticOperator::ReverseSubtract => BinaryOperator::ReverseSubtract,
        llil::ArithmeticOperator::Multiply => BinaryOperator::Multiply,
        llil::ArithmeticOperator::Divide => BinaryOperator::Divide,
        llil::ArithmeticOperator::Remainder => BinaryOperator::Remainder,
        llil::ArithmeticOperator::And => BinaryOperator::And,
        llil::ArithmeticOperator::Or => BinaryOperator::Or,
        llil::ArithmeticOperator::Xor => BinaryOperator::Xor,
        llil::ArithmeticOperator::ShiftLeft => BinaryOperator::ShiftLeft,
        llil::ArithmeticOperator::ShiftRight => BinaryOperator::ShiftRight,
        llil::ArithmeticOperator::UnsignedShiftRight => BinaryOperator::UnsignedShiftRight,
    }
}

fn element_kind(element: llil::ArrayElementKind) -> ElementType {
    match element {
        llil::ArrayElementKind::Single => ElementType::Bits32,
        llil::ArrayElementKind::Wide => ElementType::Bits64,
        llil::ArrayElementKind::Reference => ElementType::Reference,
        llil::ArrayElementKind::Boolean => ElementType::Boolean,
        llil::ArrayElementKind::Byte => ElementType::Byte,
        llil::ArrayElementKind::Char => ElementType::Char,
        llil::ArrayElementKind::Short => ElementType::Short,
    }
}

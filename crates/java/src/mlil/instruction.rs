//! Semantic lifting of individual verified JVM LLIL instructions.

use ::mlil::{
    AllocationKind, ArrayAccess, ArrayType, BinaryOperator, BranchOperandKind, BranchPredicate,
    CallKind, Constant, Conversion, ElementType, FieldAccess, FunctionBuilder, MonitorAction,
    Operation, Relation, ThreeWayComparison, TypedVariable, UnaryOperator, ValueType,
};

use crate::analysis::{FrameState, FrameValue};
use crate::bytecode::{ArrayType as NativeArrayType, Instruction as NativeInstruction};
use crate::classfile::ConstantPool;
use crate::descriptor::{ReturnType, parse_method};
use crate::llil::{
    self, ArithmeticOperator, ArrayElementKind, BitwiseOperator, BranchCondition, LocalAccess,
    StackOperation,
};

use super::reference;
use super::state::StateVariables;
use super::{Error, Result};

pub(super) struct Step {
    pub(super) operation: Operation,
    pub(super) uses: Vec<TypedVariable>,
    pub(super) defs: Vec<TypedVariable>,
}

pub(super) struct LiftedInstruction {
    pub(super) steps: Vec<Step>,
    pub(super) throw_step: usize,
}

#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
pub(super) fn lift_instruction(
    builder: &mut FunctionBuilder,
    variables: &StateVariables,
    pool: &ConstantPool,
    native: &NativeInstruction,
    instruction: &llil::Instruction,
    entry: &FrameState,
    exit: &FrameState,
    owner: &str,
) -> Result<LiftedInstruction> {
    use llil::Operation as L;

    let lifted = match &instruction.operation {
        L::Nop => one(Operation::Nop, vec![], vec![]),
        L::Constant(constant) => {
            let constant = lift_constant(pool, native, constant)?;
            one(
                Operation::Constant(constant),
                vec![],
                vec![top_def(variables, exit, owner, native.offset)?],
            )
        }
        L::Local { access, index, .. } => match access {
            LocalAccess::Load => one(
                Operation::Copy,
                vec![variables.local(entry, *index, owner)],
                vec![top_def(variables, exit, owner, native.offset)?],
            ),
            LocalAccess::Store => one(
                Operation::Copy,
                vec![top_use(variables, entry, owner, native.offset)?],
                vec![variables.local(exit, *index, owner)],
            ),
        },
        L::IncrementLocal { index, amount } => {
            let temporary = builder.declare_variable(::mlil::VariableRole::Temporary, None)?;
            LiftedInstruction {
                steps: vec![
                    Step {
                        operation: Operation::Constant(Constant::Integer(i32::from(*amount))),
                        uses: vec![],
                        defs: vec![TypedVariable::new(temporary, ValueType::Integer)],
                    },
                    Step {
                        operation: Operation::Binary(BinaryOperator::Add),
                        uses: vec![
                            variables.local(entry, *index, owner),
                            TypedVariable::new(temporary, ValueType::Integer),
                        ],
                        defs: vec![variables.local(exit, *index, owner)],
                    },
                ],
                throw_step: 1,
            }
        }
        L::Array { access, element } => lift_array(
            variables,
            *access,
            *element,
            entry,
            exit,
            owner,
            native.offset,
        )?,
        L::Stack(operation) => {
            lift_stack(variables, *operation, entry, exit, owner, native.offset)?
        }
        L::Arithmetic { operator, .. } => binary(
            variables,
            Operation::Binary(arithmetic(*operator)),
            entry,
            exit,
            owner,
            native.offset,
        )?,
        L::Negate(_) => unary(
            variables,
            Operation::Unary(UnaryOperator::Negate),
            entry,
            exit,
            owner,
            native.offset,
        )?,
        L::Shift { operator, .. } => binary(
            variables,
            Operation::Binary(shift(*operator)),
            entry,
            exit,
            owner,
            native.offset,
        )?,
        L::Bitwise { operator, .. } => binary(
            variables,
            Operation::Binary(bitwise(*operator)),
            entry,
            exit,
            owner,
            native.offset,
        )?,
        L::Convert(conversion) => unary(
            variables,
            Operation::Convert(conversion_kind(*conversion)),
            entry,
            exit,
            owner,
            native.offset,
        )?,
        L::Compare(comparison) => binary(
            variables,
            Operation::Compare(comparison_kind(*comparison)),
            entry,
            exit,
            owner,
            native.offset,
        )?,
        L::Branch { condition, .. } => {
            let (predicate, count) = branch(*condition);
            one(
                Operation::Branch(predicate),
                top_uses(variables, entry, count, owner, native.offset)?,
                vec![],
            )
        }
        L::Jump { .. } => one(Operation::Jump, vec![], vec![]),
        L::SubroutineCall { .. } | L::SubroutineReturn { .. } => {
            return Err(Error::unsupported(
                native.offset,
                "legacy jsr/ret subroutine semantics",
            ));
        }
        L::Switch(table) => one(
            Operation::Switch(table.cases.iter().map(|case| i64::from(case.key)).collect()),
            vec![top_use(variables, entry, owner, native.offset)?],
            vec![],
        ),
        L::Return(kind) => {
            let uses = if kind.is_some() {
                vec![top_use(variables, entry, owner, native.offset)?]
            } else {
                Vec::new()
            };
            one(Operation::Return, uses, vec![])
        }
        L::Field { access, index } => {
            lift_field(variables, pool, native, *access, *index, entry, exit, owner)?
        }
        L::Invoke { kind, index } => {
            lift_call(variables, pool, native, *kind, *index, entry, exit, owner)?
        }
        L::NewObject { index } => one(
            Operation::Allocate(AllocationKind::Object(reference::class_reference(
                pool, native, *index,
            )?)),
            vec![],
            vec![top_def(variables, exit, owner, native.offset)?],
        ),
        L::NewPrimitiveArray(array_type) => one(
            Operation::Allocate(AllocationKind::Array {
                array_type: ArrayType::new(primitive_array_descriptor(*array_type)),
                dimensions: 1,
            }),
            vec![top_use(variables, entry, owner, native.offset)?],
            vec![top_def(variables, exit, owner, native.offset)?],
        ),
        L::NewReferenceArray { index } => {
            let definition = top_def(variables, exit, owner, native.offset)?;
            let source = reference::class_reference(pool, native, *index)?;
            one(
                Operation::Allocate(AllocationKind::Array {
                    array_type: allocated_array_type(&definition, Some(source), native.offset)?,
                    dimensions: 1,
                }),
                vec![top_use(variables, entry, owner, native.offset)?],
                vec![definition],
            )
        }
        L::ArrayLength => unary(
            variables,
            Operation::ArrayLength,
            entry,
            exit,
            owner,
            native.offset,
        )?,
        L::Throw => one(
            Operation::Throw,
            vec![top_use(variables, entry, owner, native.offset)?],
            vec![],
        ),
        L::CheckCast { index } => unary(
            variables,
            Operation::CheckCast(reference::class_reference(pool, native, *index)?),
            entry,
            exit,
            owner,
            native.offset,
        )?,
        L::InstanceOf { index } => unary(
            variables,
            Operation::InstanceOf(reference::class_reference(pool, native, *index)?),
            entry,
            exit,
            owner,
            native.offset,
        )?,
        L::Monitor(action) => one(
            Operation::Monitor(match action {
                llil::MonitorAction::Enter => MonitorAction::Enter,
                llil::MonitorAction::Exit => MonitorAction::Exit,
            }),
            vec![top_use(variables, entry, owner, native.offset)?],
            vec![],
        ),
        L::NewMultiArray { index, dimensions } => {
            let definition = top_def(variables, exit, owner, native.offset)?;
            let source = reference::class_reference(pool, native, *index)?;
            one(
                Operation::Allocate(AllocationKind::Array {
                    array_type: allocated_array_type(&definition, Some(source), native.offset)?,
                    dimensions: *dimensions,
                }),
                top_uses(
                    variables,
                    entry,
                    usize::from(*dimensions),
                    owner,
                    native.offset,
                )?,
                vec![definition],
            )
        }
        L::Intrinsic(intrinsic) => {
            return Err(Error::unsupported(
                native.offset,
                format!("reserved JVM intrinsic {intrinsic:?}"),
            ));
        }
    };
    Ok(lifted)
}

fn lift_constant(
    pool: &ConstantPool,
    native: &NativeInstruction,
    constant: &llil::Constant,
) -> Result<Constant> {
    Ok(match constant {
        llil::Constant::Null => Constant::Null,
        llil::Constant::Integer(value) => Constant::Integer(*value),
        llil::Constant::Long(value) => Constant::Long(*value),
        llil::Constant::Float(value) => Constant::Float(*value),
        llil::Constant::Double(value) => Constant::Double(*value),
        llil::Constant::Pool { index, .. } => reference::loadable_constant(pool, native, *index)?,
    })
}

fn lift_array(
    variables: &StateVariables,
    access: llil::ArrayAccess,
    element: ArrayElementKind,
    entry: &FrameState,
    exit: &FrameState,
    owner: &str,
    offset: usize,
) -> Result<LiftedInstruction> {
    let operation = Operation::Array {
        access: match access {
            llil::ArrayAccess::Load => ArrayAccess::Get,
            llil::ArrayAccess::Store => ArrayAccess::Put,
        },
        element: element_kind(element),
    };
    Ok(match access {
        llil::ArrayAccess::Load => {
            let base = stack_base(entry, 2, offset)?;
            one(
                operation,
                stack_range(variables, entry, base, 2, owner),
                vec![variables.stack(exit, base, owner)],
            )
        }
        llil::ArrayAccess::Store => one(
            operation,
            top_uses(variables, entry, 3, owner, offset)?,
            vec![],
        ),
    })
}

#[allow(clippy::too_many_arguments)]
fn lift_field(
    variables: &StateVariables,
    pool: &ConstantPool,
    native: &NativeInstruction,
    access: llil::FieldAccess,
    index: u16,
    entry: &FrameState,
    exit: &FrameState,
    owner: &str,
) -> Result<LiftedInstruction> {
    let (field, _) = reference::field(pool, native, index)?;
    let operation = Operation::Field {
        access: match access {
            llil::FieldAccess::GetStatic => FieldAccess::GetStatic,
            llil::FieldAccess::PutStatic => FieldAccess::PutStatic,
            llil::FieldAccess::GetInstance => FieldAccess::GetInstance,
            llil::FieldAccess::PutInstance => FieldAccess::PutInstance,
        },
        field,
    };
    Ok(match access {
        llil::FieldAccess::GetStatic => one(
            operation,
            vec![],
            vec![top_def(variables, exit, owner, native.offset)?],
        ),
        llil::FieldAccess::PutStatic => one(
            operation,
            vec![top_use(variables, entry, owner, native.offset)?],
            vec![],
        ),
        llil::FieldAccess::GetInstance => {
            unary(variables, operation, entry, exit, owner, native.offset)?
        }
        llil::FieldAccess::PutInstance => one(
            operation,
            top_uses(variables, entry, 2, owner, native.offset)?,
            vec![],
        ),
    })
}

#[allow(clippy::too_many_arguments)]
fn lift_call(
    variables: &StateVariables,
    pool: &ConstantPool,
    native: &NativeInstruction,
    invocation: llil::Invocation,
    index: u16,
    entry: &FrameState,
    exit: &FrameState,
    owner: &str,
) -> Result<LiftedInstruction> {
    let (mut target, descriptor) = reference::method(pool, native, index)?;
    let kind = call_kind(invocation, &target, owner, native.offset)?;
    if kind == CallKind::Polymorphic {
        canonicalize_polymorphic_target(&mut target, native.offset)?;
    }
    let parsed = parse_method(&descriptor)?;
    let receiver_count = usize::from(!matches!(
        invocation,
        llil::Invocation::Static | llil::Invocation::Dynamic
    ));
    let argument_count = parsed.parameters.len() + receiver_count;
    let base = stack_base(entry, argument_count, native.offset)?;
    let uses = stack_range(variables, entry, base, argument_count, owner);
    let defs = match parsed.return_type {
        ReturnType::Void => vec![],
        ReturnType::Type(_) => vec![top_def(variables, exit, owner, native.offset)?],
    };
    let mut steps = vec![Step {
        operation: Operation::Call {
            kind,
            target,
            descriptor: Some(descriptor),
        },
        uses,
        defs,
    }];
    if let Some(refinement) = initialization_refinement(variables, entry, exit, base, owner) {
        steps.push(refinement);
    }
    Ok(LiftedInstruction {
        steps,
        throw_step: 0,
    })
}

fn call_kind(
    invocation: llil::Invocation,
    target: &disassembler::Reference,
    current_owner: &str,
    offset: usize,
) -> Result<CallKind> {
    Ok(match invocation {
        llil::Invocation::Virtual if is_method_handle_polymorphic(target) => CallKind::Polymorphic,
        llil::Invocation::Virtual => CallKind::Virtual,
        llil::Invocation::Special => {
            let Some(disassembler::ReferenceSymbol::Method { owner, name, .. }) = &target.symbol
            else {
                return Err(Error::unsupported(
                    offset,
                    "invokespecial target lacks a structured method identity",
                ));
            };
            if name.text == "<init>" || same_object_type(owner, current_owner) {
                CallKind::Direct
            } else {
                CallKind::Super
            }
        }
        llil::Invocation::Static => CallKind::Static,
        llil::Invocation::Interface => CallKind::Interface,
        llil::Invocation::Dynamic => CallKind::Dynamic,
    })
}

const METHOD_HANDLE_NAME: &str = "java/lang/invoke/MethodHandle";
const METHOD_HANDLE_INVOKE_NAME: &str = "invoke";
const METHOD_HANDLE_INVOKE_EXACT_NAME: &str = "invokeExact";
const METHOD_HANDLE_DECLARED_DESCRIPTOR: &str = "([Ljava/lang/Object;)Ljava/lang/Object;";

fn is_method_handle_polymorphic(target: &disassembler::Reference) -> bool {
    matches!(
        &target.symbol,
        Some(disassembler::ReferenceSymbol::Method { owner, name, .. })
            if same_object_type(owner, METHOD_HANDLE_NAME)
                && matches!(
                    name.text.as_str(),
                    METHOD_HANDLE_INVOKE_NAME | METHOD_HANDLE_INVOKE_EXACT_NAME
                )
    )
}

fn canonicalize_polymorphic_target(
    target: &mut disassembler::Reference,
    offset: usize,
) -> Result<()> {
    let Some(disassembler::ReferenceSymbol::Method {
        owner,
        name,
        descriptor,
    }) = &mut target.symbol
    else {
        return Err(Error::unsupported(
            offset,
            "signature-polymorphic target lacks a structured method identity",
        ));
    };
    METHOD_HANDLE_DECLARED_DESCRIPTOR.clone_into(descriptor);
    target.display = Some(format!("{}.{}{}", owner, name.text, descriptor));
    Ok(())
}

fn same_object_type(left: &str, right: &str) -> bool {
    fn internal_name(value: &str) -> &str {
        value
            .strip_prefix('L')
            .and_then(|value| value.strip_suffix(';'))
            .unwrap_or(value)
    }
    internal_name(left) == internal_name(right)
}

fn initialization_refinement(
    variables: &StateVariables,
    entry: &FrameState,
    exit: &FrameState,
    preserved_stack: usize,
    owner: &str,
) -> Option<Step> {
    let local_pairs = entry
        .locals()
        .iter()
        .zip(exit.locals())
        .enumerate()
        .filter(|(_, (before, after))| before != after && initialized(before, after))
        .map(|(index, _)| {
            (
                variables.local(
                    entry,
                    u16::try_from(index).expect("JVM local index fits u16"),
                    owner,
                ),
                variables.local(
                    exit,
                    u16::try_from(index).expect("JVM local index fits u16"),
                    owner,
                ),
            )
        });
    let stack_pairs = (0..preserved_stack)
        .filter(|&index| {
            entry.stack()[index] != exit.stack()[index]
                && initialized(&entry.stack()[index], &exit.stack()[index])
        })
        .map(|index| {
            (
                variables.stack(entry, index, owner),
                variables.stack(exit, index, owner),
            )
        });
    let (uses, defs): (Vec<_>, Vec<_>) = local_pairs.chain(stack_pairs).unzip();
    (!uses.is_empty()).then_some(Step {
        operation: Operation::TypeRefine,
        uses,
        defs,
    })
}

fn initialized(before: &FrameValue, after: &FrameValue) -> bool {
    matches!(
        (before, after),
        (
            FrameValue::UninitializedThis | FrameValue::Uninitialized { .. },
            FrameValue::Reference(_)
        )
    )
}

fn lift_stack(
    variables: &StateVariables,
    operation: StackOperation,
    entry: &FrameState,
    exit: &FrameState,
    owner: &str,
    offset: usize,
) -> Result<LiftedInstruction> {
    let input_count = entry.stack().len();
    let output_tokens = stack_permutation(operation, entry, offset)?;
    if output_tokens.len() < input_count {
        return Ok(one(
            Operation::Discard,
            stack_range(
                variables,
                entry,
                output_tokens.len(),
                input_count - output_tokens.len(),
                owner,
            ),
            vec![],
        ));
    }
    let unchanged = output_tokens
        .iter()
        .enumerate()
        .take_while(|(position, source)| *position == **source)
        .count();
    let uses = output_tokens[unchanged..]
        .iter()
        .map(|&source| variables.stack(entry, source, owner))
        .collect();
    let defs = (unchanged..output_tokens.len())
        .map(|position| variables.stack(exit, position, owner))
        .collect();
    Ok(one(Operation::ParallelCopy, uses, defs))
}

#[allow(clippy::too_many_lines)]
fn stack_permutation(
    operation: StackOperation,
    frame: &FrameState,
    offset: usize,
) -> Result<Vec<usize>> {
    let values = frame.stack();
    let count = values.len();
    let prefix = |suffix: &[usize]| {
        (0..count - suffix_input_count(suffix, count))
            .chain(suffix.iter().copied())
            .collect()
    };
    let result = match operation {
        StackOperation::Pop => (0..stack_base(frame, 1, offset)?).collect(),
        StackOperation::Pop2 => {
            let removed = if category_two(values.last(), offset)? {
                1
            } else {
                2
            };
            (0..stack_base(frame, removed, offset)?).collect()
        }
        StackOperation::Dup => prefix(&[count - 1, count - 1]),
        StackOperation::DupX1 => prefix(&[count - 1, count - 2, count - 1]),
        StackOperation::DupX2 => {
            if category_two(values.get(count.wrapping_sub(2)), offset)? {
                prefix(&[count - 1, count - 2, count - 1])
            } else {
                prefix(&[count - 1, count - 3, count - 2, count - 1])
            }
        }
        StackOperation::Dup2 => {
            if category_two(values.last(), offset)? {
                prefix(&[count - 1, count - 1])
            } else {
                prefix(&[count - 2, count - 1, count - 2, count - 1])
            }
        }
        StackOperation::Dup2X1 => {
            if category_two(values.last(), offset)? {
                prefix(&[count - 1, count - 2, count - 1])
            } else {
                prefix(&[count - 2, count - 1, count - 3, count - 2, count - 1])
            }
        }
        StackOperation::Dup2X2 => {
            let top_wide = category_two(values.last(), offset)?;
            let second_wide = if top_wide {
                category_two(values.get(count.wrapping_sub(2)), offset)?
            } else {
                category_two(values.get(count.wrapping_sub(3)), offset)?
            };
            match (top_wide, second_wide) {
                (true, true) => prefix(&[count - 1, count - 2, count - 1]),
                (true, false) => prefix(&[count - 1, count - 3, count - 2, count - 1]),
                (false, true) => prefix(&[count - 2, count - 1, count - 3, count - 2, count - 1]),
                (false, false) => prefix(&[
                    count - 2,
                    count - 1,
                    count - 4,
                    count - 3,
                    count - 2,
                    count - 1,
                ]),
            }
        }
        StackOperation::Swap => prefix(&[count - 1, count - 2]),
    };
    Ok(result)
}

fn suffix_input_count(suffix: &[usize], total: usize) -> usize {
    suffix
        .iter()
        .min()
        .map_or(0, |first| total.saturating_sub(*first))
}

fn category_two(value: Option<&FrameValue>, offset: usize) -> Result<bool> {
    value
        .map(|value| matches!(value, FrameValue::Long | FrameValue::Double))
        .ok_or_else(|| Error::unsupported(offset, "stack operation underflowed its frame"))
}

fn unary(
    variables: &StateVariables,
    operation: Operation,
    entry: &FrameState,
    exit: &FrameState,
    owner: &str,
    offset: usize,
) -> Result<LiftedInstruction> {
    let position = stack_base(entry, 1, offset)?;
    Ok(one(
        operation,
        vec![variables.stack(entry, position, owner)],
        vec![variables.stack(exit, position, owner)],
    ))
}

fn binary(
    variables: &StateVariables,
    operation: Operation,
    entry: &FrameState,
    exit: &FrameState,
    owner: &str,
    offset: usize,
) -> Result<LiftedInstruction> {
    let position = stack_base(entry, 2, offset)?;
    Ok(one(
        operation,
        stack_range(variables, entry, position, 2, owner),
        vec![variables.stack(exit, position, owner)],
    ))
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

fn top_use(
    variables: &StateVariables,
    frame: &FrameState,
    owner: &str,
    offset: usize,
) -> Result<TypedVariable> {
    let position = stack_base(frame, 1, offset)?;
    Ok(variables.stack(frame, position, owner))
}

fn top_def(
    variables: &StateVariables,
    frame: &FrameState,
    owner: &str,
    offset: usize,
) -> Result<TypedVariable> {
    top_use(variables, frame, owner, offset)
}

fn top_uses(
    variables: &StateVariables,
    frame: &FrameState,
    count: usize,
    owner: &str,
    offset: usize,
) -> Result<Vec<TypedVariable>> {
    let start = stack_base(frame, count, offset)?;
    Ok(stack_range(variables, frame, start, count, owner))
}

fn stack_range(
    variables: &StateVariables,
    frame: &FrameState,
    start: usize,
    count: usize,
    owner: &str,
) -> Vec<TypedVariable> {
    (start..start + count)
        .map(|position| variables.stack(frame, position, owner))
        .collect()
}

fn stack_base(frame: &FrameState, count: usize, offset: usize) -> Result<usize> {
    frame.stack().len().checked_sub(count).ok_or_else(|| {
        Error::unsupported(
            offset,
            format!(
                "semantic operation needs {count} stack values but its frame has {}",
                frame.stack().len()
            ),
        )
    })
}

fn arithmetic(operator: ArithmeticOperator) -> BinaryOperator {
    match operator {
        ArithmeticOperator::Add => BinaryOperator::Add,
        ArithmeticOperator::Subtract => BinaryOperator::Subtract,
        ArithmeticOperator::Multiply => BinaryOperator::Multiply,
        ArithmeticOperator::Divide => BinaryOperator::Divide,
        ArithmeticOperator::Remainder => BinaryOperator::Remainder,
    }
}

fn shift(operator: llil::ShiftOperator) -> BinaryOperator {
    match operator {
        llil::ShiftOperator::Left => BinaryOperator::ShiftLeft,
        llil::ShiftOperator::Right => BinaryOperator::ShiftRight,
        llil::ShiftOperator::UnsignedRight => BinaryOperator::UnsignedShiftRight,
    }
}

fn bitwise(operator: BitwiseOperator) -> BinaryOperator {
    match operator {
        BitwiseOperator::And => BinaryOperator::And,
        BitwiseOperator::Or => BinaryOperator::Or,
        BitwiseOperator::Xor => BinaryOperator::Xor,
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

fn comparison_kind(comparison: llil::Comparison) -> ThreeWayComparison {
    match comparison {
        llil::Comparison::Long => ThreeWayComparison::Long,
        llil::Comparison::FloatNanLow => ThreeWayComparison::FloatNanLow,
        llil::Comparison::FloatNanHigh => ThreeWayComparison::FloatNanHigh,
        llil::Comparison::DoubleNanLow => ThreeWayComparison::DoubleNanLow,
        llil::Comparison::DoubleNanHigh => ThreeWayComparison::DoubleNanHigh,
    }
}

fn branch(condition: BranchCondition) -> (BranchPredicate, usize) {
    let (relation, operands, count) = match condition {
        BranchCondition::IntegerZero(relation) => (relation, BranchOperandKind::IntegerZero, 1),
        BranchCondition::IntegerPair(relation) => (relation, BranchOperandKind::IntegerPair, 2),
        BranchCondition::ReferencePair(relation) => (relation, BranchOperandKind::ReferencePair, 2),
        BranchCondition::ReferenceNull(relation) => (relation, BranchOperandKind::ReferenceNull, 1),
    };
    (
        BranchPredicate {
            relation: relation_kind(relation),
            operands,
        },
        count,
    )
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

fn element_kind(element: ArrayElementKind) -> ElementType {
    match element {
        ArrayElementKind::Integer => ElementType::Integer,
        ArrayElementKind::Long => ElementType::Long,
        ArrayElementKind::Float => ElementType::Float,
        ArrayElementKind::Double => ElementType::Double,
        ArrayElementKind::Reference => ElementType::Reference,
        ArrayElementKind::ByteOrBoolean => ElementType::ByteOrBoolean,
        ArrayElementKind::Char => ElementType::Char,
        ArrayElementKind::Short => ElementType::Short,
    }
}

const fn primitive_array_descriptor(array_type: NativeArrayType) -> &'static str {
    match array_type {
        NativeArrayType::Boolean => "[Z",
        NativeArrayType::Char => "[C",
        NativeArrayType::Float => "[F",
        NativeArrayType::Double => "[D",
        NativeArrayType::Byte => "[B",
        NativeArrayType::Short => "[S",
        NativeArrayType::Int => "[I",
        NativeArrayType::Long => "[J",
    }
}

fn allocated_array_type(
    definition: &TypedVariable,
    source: Option<disassembler::Reference>,
    offset: usize,
) -> Result<ArrayType> {
    let ValueType::Reference(Some(descriptor)) = &definition.value_type else {
        return Err(Error::unsupported(
            offset,
            "array allocation lacks an exact result descriptor",
        ));
    };
    if !descriptor.starts_with('[') {
        return Err(Error::unsupported(
            offset,
            "array allocation result is not an array descriptor",
        ));
    }
    let array_type = ArrayType::new(descriptor.clone());
    Ok(source.map_or(array_type.clone(), |reference| {
        array_type.with_source_reference(reference)
    }))
}

//! Adapters from semantic MLIL instructions into cfglib data-flow analyses.

use std::collections::BTreeMap;

use crate::{
    BinaryOperator, Constant, Conversion, Instruction, Operation, ThreeWayComparison,
    UnaryOperator, VariableId,
};

/// Pure MLIL operator retained in recovered expression trees.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ExpressionOperator(Operation);

impl ExpressionOperator {
    /// Returns the underlying semantic MLIL operation.
    #[must_use]
    pub const fn operation(&self) -> &Operation {
        &self.0
    }
}

pub(crate) fn is_copy(operation: &Operation) -> bool {
    matches!(operation, Operation::Copy)
}

pub(crate) fn expression_operator(operation: &Operation) -> Option<ExpressionOperator> {
    matches!(
        operation,
        Operation::Copy
            | Operation::Unary(_)
            | Operation::Binary(_)
            | Operation::Convert(_)
            | Operation::Compare(_)
            | Operation::InstanceOf(_)
    )
    .then(|| ExpressionOperator(operation.clone()))
}

pub(crate) fn constant(operation: &Operation) -> Option<Constant> {
    let Operation::Constant(constant) = operation else {
        return None;
    };
    Some(constant.clone())
}

pub(crate) fn fold_constant(
    instruction: &Instruction,
    known: &BTreeMap<VariableId, Constant>,
) -> Option<(VariableId, Constant)> {
    let destination = *instruction.defs().first()?;
    let value = match instruction.operation() {
        Operation::Constant(value) => value.clone(),
        Operation::Copy => known.get(instruction.uses().first()?)?.clone(),
        Operation::Unary(operator) => {
            fold_unary(*operator, known.get(instruction.uses().first()?)?)?
        }
        Operation::Binary(operator) => fold_binary(
            *operator,
            known.get(instruction.uses().first()?)?,
            known.get(instruction.uses().get(1)?)?,
        )?,
        Operation::Convert(conversion) => {
            fold_conversion(*conversion, known.get(instruction.uses().first()?)?)?
        }
        Operation::Compare(comparison) => fold_comparison(
            *comparison,
            known.get(instruction.uses().first()?)?,
            known.get(instruction.uses().get(1)?)?,
        )?,
        _ => return None,
    };
    Some((destination, value))
}

pub(crate) fn callee(operation: &Operation) -> Option<String> {
    let Operation::Call { target, .. } = operation else {
        return None;
    };
    Some(
        target
            .display
            .clone()
            .unwrap_or_else(|| format!("{:?}#{}", target.kind, target.index)),
    )
}

fn fold_unary(operator: UnaryOperator, value: &Constant) -> Option<Constant> {
    match (operator, value) {
        (UnaryOperator::Negate, Constant::Integer(value)) => {
            Some(Constant::Integer(value.wrapping_neg()))
        }
        (UnaryOperator::Negate, Constant::Long(value)) => {
            Some(Constant::Long(value.wrapping_neg()))
        }
        (UnaryOperator::Negate, Constant::Float(value)) => {
            Some(Constant::Float((-f32::from_bits(*value)).to_bits()))
        }
        (UnaryOperator::Negate, Constant::Double(value)) => {
            Some(Constant::Double((-f64::from_bits(*value)).to_bits()))
        }
        (UnaryOperator::BitwiseNot, Constant::Integer(value)) => Some(Constant::Integer(!value)),
        (UnaryOperator::BitwiseNot, Constant::Long(value)) => Some(Constant::Long(!value)),
        _ => None,
    }
}

fn fold_binary(operator: BinaryOperator, left: &Constant, right: &Constant) -> Option<Constant> {
    match (left, right) {
        (Constant::Integer(left), Constant::Integer(right)) => {
            fold_integer(operator, *left, *right).map(Constant::Integer)
        }
        (Constant::Long(left), Constant::Long(right)) => {
            fold_long(operator, *left, *right).map(Constant::Long)
        }
        (Constant::Float(left), Constant::Float(right)) => {
            fold_float(operator, f32::from_bits(*left), f32::from_bits(*right))
                .map(f32::to_bits)
                .map(Constant::Float)
        }
        (Constant::Double(left), Constant::Double(right)) => {
            fold_double(operator, f64::from_bits(*left), f64::from_bits(*right))
                .map(f64::to_bits)
                .map(Constant::Double)
        }
        (Constant::Long(left), Constant::Integer(right)) => {
            fold_long(operator, *left, i64::from(*right)).map(Constant::Long)
        }
        _ => None,
    }
}

fn fold_integer(operator: BinaryOperator, left: i32, right: i32) -> Option<i32> {
    Some(match operator {
        BinaryOperator::Add => left.wrapping_add(right),
        BinaryOperator::Subtract => left.wrapping_sub(right),
        BinaryOperator::ReverseSubtract => right.wrapping_sub(left),
        BinaryOperator::Multiply => left.wrapping_mul(right),
        BinaryOperator::Divide if right != 0 => left.wrapping_div(right),
        BinaryOperator::Remainder if right != 0 => left.wrapping_rem(right),
        BinaryOperator::And => left & right,
        BinaryOperator::Or => left | right,
        BinaryOperator::Xor => left ^ right,
        BinaryOperator::ShiftLeft => left.wrapping_shl((right & 0x1f).cast_unsigned()),
        BinaryOperator::ShiftRight => left.wrapping_shr((right & 0x1f).cast_unsigned()),
        BinaryOperator::UnsignedShiftRight => {
            (left.cast_unsigned() >> (right & 0x1f)).cast_signed()
        }
        BinaryOperator::Divide | BinaryOperator::Remainder => return None,
    })
}

fn fold_long(operator: BinaryOperator, left: i64, right: i64) -> Option<i64> {
    Some(match operator {
        BinaryOperator::Add => left.wrapping_add(right),
        BinaryOperator::Subtract => left.wrapping_sub(right),
        BinaryOperator::ReverseSubtract => right.wrapping_sub(left),
        BinaryOperator::Multiply => left.wrapping_mul(right),
        BinaryOperator::Divide if right != 0 => left.wrapping_div(right),
        BinaryOperator::Remainder if right != 0 => left.wrapping_rem(right),
        BinaryOperator::And => left & right,
        BinaryOperator::Or => left | right,
        BinaryOperator::Xor => left ^ right,
        BinaryOperator::ShiftLeft => left.wrapping_shl(
            u32::try_from(right.cast_unsigned() & 0x3f).expect("masked long shift fits u32"),
        ),
        BinaryOperator::ShiftRight => left.wrapping_shr(
            u32::try_from(right.cast_unsigned() & 0x3f).expect("masked long shift fits u32"),
        ),
        BinaryOperator::UnsignedShiftRight => {
            (left.cast_unsigned() >> (right & 0x3f)).cast_signed()
        }
        BinaryOperator::Divide | BinaryOperator::Remainder => return None,
    })
}

fn fold_float(operator: BinaryOperator, left: f32, right: f32) -> Option<f32> {
    Some(match operator {
        BinaryOperator::Add => left + right,
        BinaryOperator::Subtract => left - right,
        BinaryOperator::Multiply => left * right,
        BinaryOperator::Divide => left / right,
        BinaryOperator::Remainder => left % right,
        _ => return None,
    })
}

fn fold_double(operator: BinaryOperator, left: f64, right: f64) -> Option<f64> {
    Some(match operator {
        BinaryOperator::Add => left + right,
        BinaryOperator::Subtract => left - right,
        BinaryOperator::Multiply => left * right,
        BinaryOperator::Divide => left / right,
        BinaryOperator::Remainder => left % right,
        _ => return None,
    })
}

#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss
)]
fn fold_conversion(conversion: Conversion, value: &Constant) -> Option<Constant> {
    Some(match (conversion, value) {
        (Conversion::IntToLong, Constant::Integer(value)) => Constant::Long(i64::from(*value)),
        (Conversion::IntToFloat, Constant::Integer(value)) => {
            Constant::Float((*value as f32).to_bits())
        }
        (Conversion::IntToDouble, Constant::Integer(value)) => {
            Constant::Double(f64::from(*value).to_bits())
        }
        (Conversion::LongToInt, Constant::Long(value)) => Constant::Integer(*value as i32),
        (Conversion::LongToFloat, Constant::Long(value)) => {
            Constant::Float((*value as f32).to_bits())
        }
        (Conversion::LongToDouble, Constant::Long(value)) => {
            Constant::Double((*value as f64).to_bits())
        }
        (Conversion::FloatToInt, Constant::Float(value)) => {
            Constant::Integer(f32::from_bits(*value) as i32)
        }
        (Conversion::FloatToLong, Constant::Float(value)) => {
            Constant::Long(f32::from_bits(*value) as i64)
        }
        (Conversion::FloatToDouble, Constant::Float(value)) => {
            Constant::Double(f64::from(f32::from_bits(*value)).to_bits())
        }
        (Conversion::DoubleToInt, Constant::Double(value)) => {
            Constant::Integer(f64::from_bits(*value) as i32)
        }
        (Conversion::DoubleToLong, Constant::Double(value)) => {
            Constant::Long(f64::from_bits(*value) as i64)
        }
        (Conversion::DoubleToFloat, Constant::Double(value)) => {
            Constant::Float((f64::from_bits(*value) as f32).to_bits())
        }
        (Conversion::IntToByte, Constant::Integer(value)) => {
            Constant::Integer(i32::from(*value as i8))
        }
        (Conversion::IntToChar, Constant::Integer(value)) => {
            Constant::Integer(i32::from(*value as u16))
        }
        (Conversion::IntToShort, Constant::Integer(value)) => {
            Constant::Integer(i32::from(*value as i16))
        }
        _ => return None,
    })
}

fn fold_comparison(
    comparison: ThreeWayComparison,
    left: &Constant,
    right: &Constant,
) -> Option<Constant> {
    let result = match (comparison, left, right) {
        (ThreeWayComparison::Long, Constant::Long(left), Constant::Long(right)) => {
            match left.cmp(right) {
                std::cmp::Ordering::Less => -1,
                std::cmp::Ordering::Equal => 0,
                std::cmp::Ordering::Greater => 1,
            }
        }
        (ThreeWayComparison::FloatNanLow, Constant::Float(left), Constant::Float(right)) => {
            compare_float(
                f64::from(f32::from_bits(*left)),
                f64::from(f32::from_bits(*right)),
                -1,
            )
        }
        (ThreeWayComparison::FloatNanHigh, Constant::Float(left), Constant::Float(right)) => {
            compare_float(
                f64::from(f32::from_bits(*left)),
                f64::from(f32::from_bits(*right)),
                1,
            )
        }
        (ThreeWayComparison::DoubleNanLow, Constant::Double(left), Constant::Double(right)) => {
            compare_float(f64::from_bits(*left), f64::from_bits(*right), -1)
        }
        (ThreeWayComparison::DoubleNanHigh, Constant::Double(left), Constant::Double(right)) => {
            compare_float(f64::from_bits(*left), f64::from_bits(*right), 1)
        }
        _ => return None,
    };
    Some(Constant::Integer(result))
}

fn compare_float(left: f64, right: f64, nan: i32) -> i32 {
    if left.is_nan() || right.is_nan() {
        nan
    } else if left < right {
        -1
    } else {
        i32::from(left > right)
    }
}

//! Category-aware JVM operand-stack manipulation instructions.

use crate::bytecode::Opcode;
use crate::{Error, Result};

use super::model::{FrameState, FrameValue};

pub(super) fn apply_stack_manipulation(
    opcode: Opcode,
    frame: &mut FrameState,
    offset: usize,
) -> Result<()> {
    match opcode {
        Opcode::Pop => {
            let value = take_top(frame, offset)?;
            require_category_one(&value, offset)
        }
        Opcode::Pop2 => pop_two_slots(frame, offset),
        Opcode::Dup => dup(frame, offset),
        Opcode::DupX1 => dup_x1(frame, offset),
        Opcode::DupX2 => dup_x2(frame, offset),
        Opcode::Dup2 => dup2(frame, offset),
        Opcode::Dup2X1 => dup2_x1(frame, offset),
        Opcode::Dup2X2 => dup2_x2(frame, offset),
        Opcode::Swap => swap(frame, offset),
        _ => Err(Error::invalid_bytecode(
            offset,
            "opcode is not an operand-stack manipulation",
        )),
    }
}

pub(super) fn take_top(frame: &mut FrameState, offset: usize) -> Result<FrameValue> {
    frame
        .stack
        .pop()
        .ok_or_else(|| Error::invalid_bytecode(offset, "operand stack underflow"))
}

fn pop_two_slots(frame: &mut FrameState, offset: usize) -> Result<()> {
    let first = take_top(frame, offset)?;
    if first.is_category_two() {
        return Ok(());
    }
    require_category_one(&first, offset)?;
    let second = take_top(frame, offset)?;
    require_category_one(&second, offset)
}

fn dup(frame: &mut FrameState, offset: usize) -> Result<()> {
    let value = take_top(frame, offset)?;
    require_category_one(&value, offset)?;
    frame.stack.extend([value.clone(), value]);
    Ok(())
}

fn dup_x1(frame: &mut FrameState, offset: usize) -> Result<()> {
    let value1 = category_one(frame, offset)?;
    let value2 = category_one(frame, offset)?;
    frame.stack.extend([value1.clone(), value2, value1]);
    Ok(())
}

fn dup_x2(frame: &mut FrameState, offset: usize) -> Result<()> {
    let value1 = category_one(frame, offset)?;
    let value2 = take_top(frame, offset)?;
    if value2.is_category_two() {
        frame.stack.extend([value1.clone(), value2, value1]);
    } else {
        require_category_one(&value2, offset)?;
        let value3 = category_one(frame, offset)?;
        frame.stack.extend([value1.clone(), value3, value2, value1]);
    }
    Ok(())
}

fn dup2(frame: &mut FrameState, offset: usize) -> Result<()> {
    let value1 = take_top(frame, offset)?;
    if value1.is_category_two() {
        frame.stack.extend([value1.clone(), value1]);
    } else {
        require_category_one(&value1, offset)?;
        let value2 = category_one(frame, offset)?;
        frame
            .stack
            .extend([value2.clone(), value1.clone(), value2, value1]);
    }
    Ok(())
}

fn dup2_x1(frame: &mut FrameState, offset: usize) -> Result<()> {
    let value1 = take_top(frame, offset)?;
    if value1.is_category_two() {
        let value2 = category_one(frame, offset)?;
        frame.stack.extend([value1.clone(), value2, value1]);
    } else {
        require_category_one(&value1, offset)?;
        let value2 = category_one(frame, offset)?;
        let value3 = category_one(frame, offset)?;
        frame
            .stack
            .extend([value2.clone(), value1.clone(), value3, value2, value1]);
    }
    Ok(())
}

fn dup2_x2(frame: &mut FrameState, offset: usize) -> Result<()> {
    let value1 = take_top(frame, offset)?;
    if value1.is_category_two() {
        let value2 = take_top(frame, offset)?;
        if value2.is_category_two() {
            frame.stack.extend([value1.clone(), value2, value1]);
        } else {
            require_category_one(&value2, offset)?;
            let value3 = category_one(frame, offset)?;
            frame.stack.extend([value1.clone(), value3, value2, value1]);
        }
    } else {
        require_category_one(&value1, offset)?;
        let value2 = category_one(frame, offset)?;
        let value3 = take_top(frame, offset)?;
        if value3.is_category_two() {
            frame
                .stack
                .extend([value2.clone(), value1.clone(), value3, value2, value1]);
        } else {
            require_category_one(&value3, offset)?;
            let value4 = category_one(frame, offset)?;
            frame.stack.extend([
                value2.clone(),
                value1.clone(),
                value4,
                value3,
                value2,
                value1,
            ]);
        }
    }
    Ok(())
}

fn swap(frame: &mut FrameState, offset: usize) -> Result<()> {
    let value1 = category_one(frame, offset)?;
    let value2 = category_one(frame, offset)?;
    frame.stack.extend([value1, value2]);
    Ok(())
}

fn category_one(frame: &mut FrameState, offset: usize) -> Result<FrameValue> {
    let value = take_top(frame, offset)?;
    require_category_one(&value, offset)?;
    Ok(value)
}

fn require_category_one(value: &FrameValue, offset: usize) -> Result<()> {
    if value.is_category_two() {
        Err(Error::invalid_bytecode(
            offset,
            "stack manipulation requires a category-one value",
        ))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dup2_x2_supports_both_category_two_form() {
        let mut frame = FrameState {
            locals: Vec::new(),
            stack: vec![FrameValue::Long, FrameValue::Double],
        };
        apply_stack_manipulation(Opcode::Dup2X2, &mut frame, 0).unwrap();
        assert_eq!(
            frame.stack,
            vec![FrameValue::Double, FrameValue::Long, FrameValue::Double]
        );
    }

    fn transformed(opcode: Opcode, stack: Vec<FrameValue>) -> Vec<FrameValue> {
        let mut frame = FrameState {
            locals: Vec::new(),
            stack,
        };
        apply_stack_manipulation(opcode, &mut frame, 0).unwrap();
        frame.stack
    }

    #[test]
    fn every_duplication_form_respects_value_categories() {
        use FrameValue::{Double as D, Float as F, Integer as I, Long as L};

        assert_eq!(transformed(Opcode::Dup, vec![I]), vec![I, I]);
        assert_eq!(transformed(Opcode::DupX1, vec![I, F]), vec![F, I, F]);
        assert_eq!(transformed(Opcode::DupX2, vec![L, I]), vec![I, L, I]);
        assert_eq!(transformed(Opcode::DupX2, vec![I, F, I]), vec![I, I, F, I]);
        assert_eq!(transformed(Opcode::Dup2, vec![L]), vec![L, L]);
        assert_eq!(transformed(Opcode::Dup2, vec![I, F]), vec![I, F, I, F]);
        assert_eq!(transformed(Opcode::Dup2X1, vec![I, D]), vec![D, I, D]);
        assert_eq!(
            transformed(Opcode::Dup2X1, vec![I, F, I]),
            vec![F, I, I, F, I]
        );
        assert_eq!(transformed(Opcode::Dup2X2, vec![I, F, D]), vec![D, I, F, D]);
        assert_eq!(
            transformed(Opcode::Dup2X2, vec![D, I, F]),
            vec![I, F, D, I, F]
        );
        assert_eq!(
            transformed(Opcode::Dup2X2, vec![I, F, I, F]),
            vec![I, F, I, F, I, F]
        );
        assert_eq!(transformed(Opcode::Swap, vec![I, F]), vec![F, I]);
    }

    #[test]
    fn invalid_category_forms_are_rejected() {
        let mut frame = FrameState {
            locals: Vec::new(),
            stack: vec![FrameValue::Long],
        };
        assert!(apply_stack_manipulation(Opcode::Dup, &mut frame, 7).is_err());

        let mut frame = FrameState {
            locals: Vec::new(),
            stack: vec![FrameValue::Integer, FrameValue::Double],
        };
        assert!(apply_stack_manipulation(Opcode::Swap, &mut frame, 9).is_err());
    }
}

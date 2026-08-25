//! Dalvik register allocation and scratch-frame sizing.

use ::mlil::{Function, Operation, ValueType, VariableId, VariableRole};

use super::super::{Error, Result};

pub(super) const MINIMUM_SCRATCH_WORDS: u16 = 8;

pub(super) struct RegisterAllocation {
    registers: Vec<u16>,
    registers_size: u16,
    ins_size: u16,
    outs_size: u16,
}

impl RegisterAllocation {
    pub(super) fn compute(function: &Function) -> Result<Self> {
        let mut widths = vec![1u16; function.variables().len()];
        let (scratch_words, outs_size) = measure_frame(function, &mut widths)?;
        u8::try_from(outs_size).map_err(|_| {
            Error::lowering(
                ::mlil::InstructionId::from_raw(0),
                "Dalvik outgoing argument width exceeds 255 words",
            )
        })?;

        apply_parameter_widths(function, &mut widths)?;
        let mut registers = vec![u16::MAX; function.variables().len()];
        let mut cursor = u32::from(scratch_words);
        for variable in function
            .variables()
            .iter()
            .filter(|variable| !matches!(variable.role, VariableRole::Parameter(_)))
        {
            registers[variable.id.index()] = checked_register(cursor, variable.id)?;
            cursor = cursor
                .checked_add(u32::from(widths[variable.id.index()]))
                .ok_or_else(|| allocation_error(variable.id, "register allocation overflow"))?;
        }
        let parameter_start = cursor;
        let mut parameters = function
            .variables()
            .iter()
            .filter_map(|variable| match variable.role {
                VariableRole::Parameter(ordinal) => Some((ordinal, variable.id)),
                _ => None,
            })
            .collect::<Vec<_>>();
        parameters.sort_unstable_by_key(|(ordinal, _)| *ordinal);
        for (_, variable) in parameters {
            registers[variable.index()] = checked_register(cursor, variable)?;
            cursor = cursor
                .checked_add(u32::from(widths[variable.index()]))
                .ok_or_else(|| allocation_error(variable, "parameter allocation overflow"))?;
        }
        let registers_size = u16::try_from(cursor).map_err(|_| {
            Error::lowering(
                ::mlil::InstructionId::from_raw(0),
                "Dalvik register frame exceeds 65,535 words",
            )
        })?;
        let ins_size = u16::try_from(cursor - parameter_start).map_err(|_| {
            Error::lowering(
                ::mlil::InstructionId::from_raw(0),
                "Dalvik incoming register width exceeds u16",
            )
        })?;
        Ok(Self {
            registers,
            registers_size,
            ins_size,
            outs_size,
        })
    }

    pub(super) fn register(&self, variable: VariableId) -> u16 {
        self.registers[variable.index()]
    }

    pub(super) const fn registers_size(&self) -> u16 {
        self.registers_size
    }

    pub(super) const fn ins_size(&self) -> u16 {
        self.ins_size
    }

    pub(super) const fn outs_size(&self) -> u16 {
        self.outs_size
    }
}

fn measure_frame(function: &Function, widths: &mut [u16]) -> Result<(u16, u16)> {
    let mut scratch_words = MINIMUM_SCRATCH_WORDS;
    let mut outs_size = 0u16;
    for block in function.cfg().blocks() {
        for instruction in block.instructions() {
            for (&variable, value_type) in instruction
                .uses()
                .iter()
                .zip(instruction.use_types())
                .chain(instruction.defs().iter().zip(instruction.def_types()))
            {
                widths[variable.index()] = widths[variable.index()].max(width(value_type));
            }
            let use_words = words(instruction.use_types(), instruction.id())?;
            if matches!(
                instruction.operation(),
                Operation::ParallelCopy | Operation::TypeRefine
            ) {
                scratch_words = scratch_words.max(use_words);
            }
            if matches!(instruction.operation(), Operation::Call { .. }) {
                outs_size = outs_size.max(use_words);
                scratch_words = scratch_words.max(use_words);
            }
            if matches!(
                instruction.operation(),
                Operation::Allocate(::mlil::AllocationKind::InitializedArray { .. })
            ) {
                scratch_words = scratch_words.max(use_words);
            }
            if let Operation::Allocate(::mlil::AllocationKind::Array { dimensions, .. }) =
                instruction.operation()
            {
                let dimensions = u16::from(*dimensions);
                if dimensions > 1 {
                    let persistent = dimensions
                        .checked_mul(3)
                        .and_then(|words| words.checked_add(MINIMUM_SCRATCH_WORDS - 2))
                        .ok_or_else(|| {
                            Error::lowering(
                                instruction.id(),
                                "Dalvik multidimensional-array scratch size overflowed",
                            )
                        })?;
                    scratch_words = scratch_words.max(persistent);
                }
            }
        }
    }
    Ok((scratch_words, outs_size))
}

pub(super) const fn width(value_type: &ValueType) -> u16 {
    if matches!(
        value_type,
        ValueType::Long | ValueType::Double | ValueType::Bits64
    ) {
        2
    } else {
        1
    }
}

pub(super) fn words(types: &[ValueType], instruction: ::mlil::InstructionId) -> Result<u16> {
    types.iter().try_fold(0u16, |total, value| {
        total
            .checked_add(width(value))
            .ok_or_else(|| Error::lowering(instruction, "Dalvik operand width exceeds u16"))
    })
}

fn apply_parameter_widths(function: &Function, widths: &mut [u16]) -> Result<()> {
    let mut parameters = function
        .variables()
        .iter()
        .filter_map(|variable| match variable.role {
            VariableRole::Parameter(ordinal) => Some((ordinal, variable.id)),
            _ => None,
        })
        .collect::<Vec<_>>();
    parameters.sort_unstable_by_key(|(ordinal, _)| *ordinal);
    let descriptor_widths =
        parameter_widths(&function.source().symbol.signature).ok_or_else(|| {
            Error::lowering(
                ::mlil::InstructionId::from_raw(0),
                "MLIL source has an invalid method descriptor",
            )
        })?;
    let receiver = match parameters.len().checked_sub(descriptor_widths.len()) {
        Some(0) => false,
        Some(1) => true,
        _ => {
            return Err(Error::lowering(
                ::mlil::InstructionId::from_raw(0),
                "parameter variables disagree with the method descriptor",
            ));
        }
    };
    for (position, (_, variable)) in parameters.into_iter().enumerate() {
        let width = if receiver && position == 0 {
            1
        } else {
            descriptor_widths[position - usize::from(receiver)]
        };
        widths[variable.index()] = width;
    }
    Ok(())
}

fn parameter_widths(descriptor: &str) -> Option<Vec<u16>> {
    let bytes = descriptor.as_bytes();
    if bytes.first() != Some(&b'(') {
        return None;
    }
    let mut cursor = 1usize;
    let mut widths = Vec::new();
    while bytes.get(cursor) != Some(&b')') {
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
                    bytes.get(cursor)?;
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
        widths.push(width);
    }
    (bytes.get(cursor + 1).is_some()).then_some(widths)
}

fn checked_register(value: u32, variable: VariableId) -> Result<u16> {
    u16::try_from(value).map_err(|_| allocation_error(variable, "register index exceeds u16"))
}

fn allocation_error(variable: VariableId, message: &str) -> Error {
    Error::lowering(
        ::mlil::InstructionId::from_raw(0),
        format!("cannot allocate {variable}: {message}"),
    )
}

//! JVM local-slot allocation for mutable MLIL variables.

use ::mlil::{Function, SourceStorage, ValueType, VariableId, VariableRole};

use super::super::{Error, Result};

pub(super) struct LocalAllocation {
    slots: Vec<u16>,
    max_locals: u16,
}

impl LocalAllocation {
    pub(super) fn compute(function: &Function) -> Result<Self> {
        let mut widths = vec![1u16; function.variables().len()];
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
            }
        }

        let mut slots = vec![u16::MAX; function.variables().len()];
        let mut next = allocate_parameters(function, &mut widths, &mut slots)?;
        for variable in function.variables() {
            if slots[variable.id.index()] != u16::MAX {
                continue;
            }
            let Some(native) = variable.native else {
                continue;
            };
            let SourceStorage::JvmLocal(slot) = native.storage else {
                continue;
            };
            slots[variable.id.index()] = slot;
            next = next.max(u32::from(slot) + u32::from(widths[variable.id.index()]));
        }
        for variable in function.variables() {
            if slots[variable.id.index()] != u16::MAX {
                continue;
            }
            let slot = u16::try_from(next).map_err(|_| {
                Error::lowering(
                    InstructionPlaceholder::id(),
                    "JVM local allocation exceeds u16",
                )
            })?;
            slots[variable.id.index()] = slot;
            next = next
                .checked_add(u32::from(widths[variable.id.index()]))
                .ok_or_else(|| {
                    Error::lowering(
                        InstructionPlaceholder::id(),
                        "JVM local allocation overflow",
                    )
                })?;
        }
        let max_locals = u16::try_from(next).map_err(|_| {
            Error::lowering(
                InstructionPlaceholder::id(),
                "JVM local allocation exceeds 65,535 slots",
            )
        })?;
        Ok(Self { slots, max_locals })
    }

    pub(super) fn slot(&self, variable: VariableId) -> u16 {
        self.slots[variable.index()]
    }

    pub(super) const fn max_locals(&self) -> u16 {
        self.max_locals
    }
}

fn allocate_parameters(function: &Function, widths: &mut [u16], slots: &mut [u16]) -> Result<u32> {
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
                InstructionPlaceholder::id(),
                "MLIL source has an invalid method descriptor",
            )
        })?;
    let receiver = match parameters.len().checked_sub(descriptor_widths.len()) {
        Some(0) => false,
        Some(1) => true,
        _ => {
            return Err(Error::lowering(
                InstructionPlaceholder::id(),
                "parameter variables disagree with the method descriptor",
            ));
        }
    };
    let mut cursor = 0u32;
    for (position, (_, variable)) in parameters.into_iter().enumerate() {
        let width = if receiver && position == 0 {
            1
        } else {
            descriptor_widths[position - usize::from(receiver)]
        };
        widths[variable.index()] = width;
        slots[variable.index()] = u16::try_from(cursor).map_err(|_| {
            Error::lowering(
                InstructionPlaceholder::id(),
                "JVM parameter local exceeds u16",
            )
        })?;
        cursor = cursor.checked_add(u32::from(width)).ok_or_else(|| {
            Error::lowering(
                InstructionPlaceholder::id(),
                "JVM parameter local allocation overflow",
            )
        })?;
    }
    Ok(cursor)
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

struct InstructionPlaceholder;

impl InstructionPlaceholder {
    const fn id() -> ::mlil::InstructionId {
        ::mlil::InstructionId::from_raw(0)
    }
}

//! Bounds-checked instruction decoding and target validation.

use std::collections::{BTreeMap, BTreeSet};

use crate::{Error, Result};

use super::{
    ArrayDataPayload, Instruction, InstructionData, InstructionFormat, Opcode, Operands,
    PackedSwitchPayload, SparseSwitchPayload,
};

const PACKED_SWITCH_PAYLOAD: u16 = 0x0100;
const SPARSE_SWITCH_PAYLOAD: u16 = 0x0200;
const ARRAY_DATA_PAYLOAD: u16 = 0x0300;

/// Decodes and validates an entire Dalvik instruction stream.
///
/// Relative branch and switch deltas become absolute code-unit offsets. All
/// targets are checked against item boundaries, and payload references must
/// select the payload kind required by their referring opcode.
///
/// # Errors
///
/// Returns an error for truncated encodings, undefined opcodes, invalid
/// reserved bits, overflowing addresses, malformed payloads, or bad targets.
pub fn decode(code_units: &[u16]) -> Result<Vec<Instruction>> {
    let mut decoded = Vec::new();
    let mut cursor = 0usize;
    while cursor < code_units.len() {
        let offset = u32::try_from(cursor).map_err(|_| {
            Error::invalid_instruction(u32::MAX, "instruction stream exceeds DEX address space")
        })?;
        let word = code_units[cursor];
        let instruction = match word {
            PACKED_SWITCH_PAYLOAD => decode_packed_switch(code_units, cursor, offset)?,
            SPARSE_SWITCH_PAYLOAD => decode_sparse_switch(code_units, cursor, offset)?,
            ARRAY_DATA_PAYLOAD => decode_array_data(code_units, cursor, offset)?,
            _ => decode_operation(code_units, cursor, offset)?,
        };
        let width = instruction.code_units().ok_or_else(|| {
            Error::invalid_instruction(offset, "instruction width exceeds DEX address space")
        })?;
        let width = usize::try_from(width).map_err(|_| {
            Error::invalid_instruction(offset, "instruction width does not fit this platform")
        })?;
        cursor = cursor
            .checked_add(width)
            .ok_or_else(|| Error::invalid_instruction(offset, "instruction cursor overflowed"))?;
        decoded.push(instruction);
    }

    validate_targets(&mut decoded, code_units.len())?;
    Ok(decoded)
}

fn decode_operation(code: &[u16], cursor: usize, offset: u32) -> Result<Instruction> {
    let first = code[cursor];
    let [opcode_byte, high] = first.to_le_bytes();
    let opcode = Opcode::from_byte(opcode_byte).ok_or_else(|| {
        Error::invalid_instruction(offset, format!("undefined opcode 0x{opcode_byte:02x}"))
    })?;
    let width = usize::try_from(opcode.format().code_units()).map_err(|_| {
        Error::invalid_instruction(offset, "instruction width does not fit this platform")
    })?;
    let words = take(code, cursor, width, offset)?;
    let operands = decode_operands(opcode, high, words, offset)?;
    Ok(Instruction::operation(offset, opcode, operands))
}

#[allow(clippy::too_many_lines)]
fn decode_operands(opcode: Opcode, high: u8, words: &[u16], offset: u32) -> Result<Operands> {
    let nibbles = || (u16::from(high & 0x0f), u16::from(high >> 4));
    let target8 = || relative_target(offset, i64::from(i8::from_ne_bytes([high])));
    let target16 = || relative_target(offset, i64::from(signed16(words[1])));
    let target32 = || relative_target(offset, i64::from(signed32(words[1], words[2])));

    let operands = match opcode.format() {
        InstructionFormat::F10x => {
            require_zero(high, offset, "format 10x reserved byte")?;
            Operands::None
        }
        InstructionFormat::F12x => {
            let (first, second) = nibbles();
            Operands::Registers { first, second }
        }
        InstructionFormat::F11n => Operands::RegisterLiteral {
            register: u16::from(high & 0x0f),
            literal: i64::from(sign_nibble(high >> 4)),
        },
        InstructionFormat::F11x => Operands::Register(u16::from(high)),
        InstructionFormat::F10t => Operands::Branch { target: target8()? },
        InstructionFormat::F20t => {
            require_zero(high, offset, "format 20t reserved byte")?;
            Operands::Branch {
                target: target16()?,
            }
        }
        InstructionFormat::F22x => Operands::Registers {
            first: u16::from(high),
            second: words[1],
        },
        InstructionFormat::F21t | InstructionFormat::F31t => Operands::RegisterBranch {
            register: u16::from(high),
            target: if opcode.format() == InstructionFormat::F21t {
                target16()?
            } else {
                target32()?
            },
        },
        InstructionFormat::F21s => Operands::RegisterLiteral {
            register: u16::from(high),
            literal: i64::from(signed16(words[1])),
        },
        InstructionFormat::F21h => {
            let shift = if opcode == Opcode::ConstWideHigh16 {
                48
            } else {
                16
            };
            Operands::RegisterLiteral {
                register: u16::from(high),
                literal: i64::from(signed16(words[1])) << shift,
            }
        }
        InstructionFormat::F21c => Operands::RegisterIndex {
            register: u16::from(high),
            index: u32::from(words[1]),
        },
        InstructionFormat::F23x => {
            let [second, third] = words[1].to_le_bytes();
            Operands::ThreeRegisters {
                first: u16::from(high),
                second: u16::from(second),
                third: u16::from(third),
            }
        }
        InstructionFormat::F22t => {
            let (first, second) = nibbles();
            Operands::RegistersBranch {
                first,
                second,
                target: target16()?,
            }
        }
        InstructionFormat::F22s => {
            let (first, second) = nibbles();
            Operands::RegistersLiteral {
                first,
                second,
                literal: i64::from(signed16(words[1])),
            }
        }
        InstructionFormat::F22c => {
            let (first, second) = nibbles();
            Operands::RegistersIndex {
                first,
                second,
                index: u32::from(words[1]),
            }
        }
        InstructionFormat::F22b => {
            let [second, literal] = words[1].to_le_bytes();
            Operands::RegistersLiteral {
                first: u16::from(high),
                second: u16::from(second),
                literal: i64::from(i8::from_ne_bytes([literal])),
            }
        }
        InstructionFormat::F30t => {
            require_zero(high, offset, "format 30t reserved byte")?;
            Operands::Branch {
                target: target32()?,
            }
        }
        InstructionFormat::F32x => {
            require_zero(high, offset, "format 32x reserved byte")?;
            Operands::Registers {
                first: words[1],
                second: words[2],
            }
        }
        InstructionFormat::F31i => Operands::RegisterLiteral {
            register: u16::from(high),
            literal: i64::from(signed32(words[1], words[2])),
        },
        InstructionFormat::F31c => Operands::RegisterIndex {
            register: u16::from(high),
            index: unsigned32(words[1], words[2]),
        },
        InstructionFormat::F35c | InstructionFormat::F45cc => {
            decode_register_list(opcode, high, words, offset)?
        }
        InstructionFormat::F3rc | InstructionFormat::F4rcc => Operands::RegisterRangeIndex {
            start: words[2],
            count: high,
            index: u32::from(words[1]),
            secondary_index: (opcode.format() == InstructionFormat::F4rcc)
                .then(|| u32::from(words[3])),
        },
        InstructionFormat::F51l => Operands::RegisterLiteral {
            register: u16::from(high),
            literal: signed64(words[1], words[2], words[3], words[4]),
        },
    };
    Ok(operands)
}

fn decode_register_list(opcode: Opcode, high: u8, words: &[u16], offset: u32) -> Result<Operands> {
    let count = usize::from(high >> 4);
    if count > 5 {
        return Err(Error::invalid_instruction(
            offset,
            format!("format 35c/45cc register count {count} exceeds five"),
        ));
    }
    let extra = u16::from(high & 0x0f);
    let [first, second] = words[2].to_le_bytes();
    let candidates = [
        u16::from(first & 0x0f),
        u16::from(first >> 4),
        u16::from(second & 0x0f),
        u16::from(second >> 4),
        extra,
    ];
    let registers = candidates[..count].to_vec();
    Ok(Operands::RegisterListIndex {
        registers,
        index: u32::from(words[1]),
        secondary_index: (opcode.format() == InstructionFormat::F45cc).then(|| u32::from(words[3])),
    })
}

fn decode_packed_switch(code: &[u16], cursor: usize, offset: u32) -> Result<Instruction> {
    require_payload_alignment(offset)?;
    let head = take(code, cursor, 4, offset)?;
    let count = usize::from(head[1]);
    let width = count
        .checked_mul(2)
        .and_then(|value| value.checked_add(4))
        .ok_or_else(|| Error::invalid_instruction(offset, "packed-switch size overflowed"))?;
    let words = take(code, cursor, width, offset)?;
    let first_key = signed32(words[2], words[3]);
    let targets = words[4..]
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| u32::from_ne_bytes(signed32(pair[0], pair[1]).to_ne_bytes()))
        .collect();
    Ok(Instruction::packed_switch(
        offset,
        PackedSwitchPayload { first_key, targets },
    ))
}

fn decode_sparse_switch(code: &[u16], cursor: usize, offset: u32) -> Result<Instruction> {
    require_payload_alignment(offset)?;
    let head = take(code, cursor, 2, offset)?;
    let count = usize::from(head[1]);
    let width = count
        .checked_mul(4)
        .and_then(|value| value.checked_add(2))
        .ok_or_else(|| Error::invalid_instruction(offset, "sparse-switch size overflowed"))?;
    let words = take(code, cursor, width, offset)?;
    let key_end = 2 + count * 2;
    let keys = words[2..key_end]
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| signed32(pair[0], pair[1]))
        .collect();
    let targets = words[key_end..]
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| u32::from_ne_bytes(signed32(pair[0], pair[1]).to_ne_bytes()))
        .collect();
    Ok(Instruction::sparse_switch(
        offset,
        SparseSwitchPayload { keys, targets },
    ))
}

fn decode_array_data(code: &[u16], cursor: usize, offset: u32) -> Result<Instruction> {
    require_payload_alignment(offset)?;
    let head = take(code, cursor, 4, offset)?;
    let element_width = head[1];
    if element_width == 0 {
        return Err(Error::invalid_instruction(
            offset,
            "array-data element width is zero",
        ));
    }
    let element_count = unsigned32(head[2], head[3]);
    let byte_count = usize::from(element_width)
        .checked_mul(usize::try_from(element_count).map_err(|_| {
            Error::invalid_instruction(offset, "array-data size does not fit this platform")
        })?)
        .ok_or_else(|| Error::invalid_instruction(offset, "array-data size overflowed"))?;
    let data_words = byte_count.checked_add(1).ok_or_else(|| {
        Error::invalid_instruction(offset, "array-data padding calculation overflowed")
    })? / 2;
    let width = data_words
        .checked_add(4)
        .ok_or_else(|| Error::invalid_instruction(offset, "array-data size overflowed"))?;
    let words = take(code, cursor, width, offset)?;
    let mut data = Vec::with_capacity(byte_count);
    for word in &words[4..] {
        data.extend_from_slice(&word.to_le_bytes());
    }
    if byte_count % 2 == 1 && data.get(byte_count).copied() != Some(0) {
        return Err(Error::invalid_instruction(
            offset,
            "array-data alignment padding is nonzero",
        ));
    }
    data.truncate(byte_count);
    Ok(Instruction::array_data(
        offset,
        ArrayDataPayload {
            element_width,
            element_count,
            data,
        },
    ))
}

fn validate_targets(instructions: &mut [Instruction], stream_len: usize) -> Result<()> {
    let stream_len = u32::try_from(stream_len).map_err(|_| {
        Error::invalid_instruction(u32::MAX, "instruction stream exceeds DEX address space")
    })?;
    let mut operations = BTreeSet::new();
    let mut items = BTreeMap::new();
    for (index, instruction) in instructions.iter().enumerate() {
        items.insert(instruction.offset(), index);
        if matches!(instruction.data(), InstructionData::Operation { .. }) {
            operations.insert(instruction.offset());
        }
    }

    let mut payload_bases = BTreeMap::new();
    for instruction in instructions.iter() {
        let InstructionData::Operation { opcode, operands } = instruction.data() else {
            continue;
        };
        let Some(target) = operand_target(operands) else {
            continue;
        };
        match opcode {
            Opcode::PackedSwitch | Opcode::SparseSwitch | Opcode::FillArrayData => {
                let Some(target_index) = items.get(&target).copied() else {
                    return Err(bad_target(instruction.offset(), target, stream_len));
                };
                let target_data = instructions[target_index].data();
                let valid = matches!(
                    (opcode, target_data),
                    (
                        Opcode::PackedSwitch,
                        InstructionData::PackedSwitchPayload(_)
                    ) | (
                        Opcode::SparseSwitch,
                        InstructionData::SparseSwitchPayload(_)
                    ) | (Opcode::FillArrayData, InstructionData::ArrayDataPayload(_))
                );
                if !valid {
                    return Err(Error::invalid_instruction(
                        instruction.offset(),
                        format!(
                            "{} target {target} selects the wrong payload kind",
                            opcode.mnemonic()
                        ),
                    ));
                }
                if let Some(previous) = payload_bases.insert(target, instruction.offset())
                    && previous != instruction.offset()
                    && *opcode != Opcode::FillArrayData
                {
                    return Err(Error::invalid_instruction(
                        instruction.offset(),
                        format!("switch payload at {target} has more than one referring base"),
                    ));
                }
            }
            _ if !operations.contains(&target) => {
                return Err(bad_target(instruction.offset(), target, stream_len));
            }
            _ => {}
        }
    }

    resolve_payload_targets(instructions, &payload_bases, &operations, stream_len)
}

fn resolve_payload_targets(
    instructions: &mut [Instruction],
    payload_bases: &BTreeMap<u32, u32>,
    operations: &BTreeSet<u32>,
    stream_len: u32,
) -> Result<()> {
    for instruction in instructions {
        let item_offset = instruction.offset();
        let base = payload_bases.get(&item_offset).copied();
        if base.is_none()
            && matches!(
                instruction.data(),
                InstructionData::PackedSwitchPayload(_)
                    | InstructionData::SparseSwitchPayload(_)
                    | InstructionData::ArrayDataPayload(_)
            )
        {
            return Err(Error::invalid_instruction(
                item_offset,
                "payload has no referring instruction",
            ));
        }
        let Some(base) = base else {
            continue;
        };
        match instruction.data_mut() {
            InstructionData::PackedSwitchPayload(payload) => {
                resolve_switch_targets(base, &mut payload.targets, operations, stream_len)?;
            }
            InstructionData::SparseSwitchPayload(payload) => {
                if payload.keys.len() != payload.targets.len() {
                    return Err(Error::invalid_instruction(
                        item_offset,
                        "sparse-switch key and target counts differ",
                    ));
                }
                if payload.keys.windows(2).any(|pair| pair[0] >= pair[1]) {
                    return Err(Error::invalid_instruction(
                        item_offset,
                        "sparse-switch keys are not strictly increasing",
                    ));
                }
                resolve_switch_targets(base, &mut payload.targets, operations, stream_len)?;
            }
            InstructionData::Operation { .. } | InstructionData::ArrayDataPayload(_) => {}
        }
    }
    Ok(())
}

fn resolve_switch_targets(
    base: u32,
    targets: &mut [u32],
    operations: &BTreeSet<u32>,
    stream_len: u32,
) -> Result<()> {
    for target in targets {
        let delta = i64::from(i32::from_ne_bytes(target.to_ne_bytes()));
        let absolute = relative_target(base, delta)?;
        if !operations.contains(&absolute) {
            return Err(bad_target(base, absolute, stream_len));
        }
        *target = absolute;
    }
    Ok(())
}

fn operand_target(operands: &Operands) -> Option<u32> {
    match operands {
        Operands::Branch { target }
        | Operands::RegisterBranch { target, .. }
        | Operands::RegistersBranch { target, .. } => Some(*target),
        Operands::None
        | Operands::Register(_)
        | Operands::Registers { .. }
        | Operands::ThreeRegisters { .. }
        | Operands::RegisterLiteral { .. }
        | Operands::RegistersLiteral { .. }
        | Operands::RegisterIndex { .. }
        | Operands::RegistersIndex { .. }
        | Operands::RegisterListIndex { .. }
        | Operands::RegisterRangeIndex { .. } => None,
    }
}

fn relative_target(offset: u32, delta: i64) -> Result<u32> {
    let target = i64::from(offset).checked_add(delta).ok_or_else(|| {
        Error::invalid_instruction(offset, "relative target calculation overflowed")
    })?;
    u32::try_from(target).map_err(|_| {
        Error::invalid_instruction(
            offset,
            format!("relative target {target} is outside DEX code"),
        )
    })
}

fn bad_target(source: u32, target: u32, stream_len: u32) -> Error {
    let reason = if target >= stream_len {
        "outside the instruction stream"
    } else {
        "not an instruction boundary of the required kind"
    };
    Error::invalid_instruction(source, format!("target {target} is {reason}"))
}

fn require_payload_alignment(offset: u32) -> Result<()> {
    if offset.is_multiple_of(2) {
        Ok(())
    } else {
        Err(Error::invalid_instruction(
            offset,
            "payload is not aligned to a four-byte boundary",
        ))
    }
}

fn require_zero(value: u8, offset: u32, field: &str) -> Result<()> {
    if value == 0 {
        Ok(())
    } else {
        Err(Error::invalid_instruction(
            offset,
            format!("{field} is nonzero"),
        ))
    }
}

fn take(code: &[u16], cursor: usize, count: usize, offset: u32) -> Result<&[u16]> {
    let end = cursor
        .checked_add(count)
        .ok_or_else(|| Error::invalid_instruction(offset, "instruction length overflowed"))?;
    code.get(cursor..end).ok_or_else(|| {
        Error::invalid_instruction(
            offset,
            format!("truncated instruction: needs {count} code units"),
        )
    })
}

fn signed16(word: u16) -> i16 {
    i16::from_ne_bytes(word.to_ne_bytes())
}

fn unsigned32(low: u16, high: u16) -> u32 {
    u32::from(low) | (u32::from(high) << 16)
}

fn signed32(low: u16, high: u16) -> i32 {
    i32::from_ne_bytes(unsigned32(low, high).to_ne_bytes())
}

fn signed64(first: u16, second: u16, third: u16, fourth: u16) -> i64 {
    let bits = u64::from(first)
        | (u64::from(second) << 16)
        | (u64::from(third) << 32)
        | (u64::from(fourth) << 48);
    i64::from_ne_bytes(bits.to_ne_bytes())
}

const fn sign_nibble(value: u8) -> i8 {
    let shifted = value << 4;
    i8::from_ne_bytes([shifted]) >> 4
}

//! Checked encoding for ordinary instructions and payload tables.

use std::collections::BTreeMap;

use crate::{Error, Result};

use super::{Instruction, InstructionData, InstructionFormat, Opcode, Operands, decode};

const PACKED_SWITCH_PAYLOAD: u16 = 0x0100;
const SPARSE_SWITCH_PAYLOAD: u16 = 0x0200;
const ARRAY_DATA_PAYLOAD: u16 = 0x0300;

/// Encodes a complete Dalvik instruction stream.
///
/// Item offsets must describe a contiguous layout beginning at zero. Branch
/// operands and switch payload targets are absolute code-unit addresses.
///
/// # Errors
///
/// Returns an error for stale layouts, mismatched operand variants, values
/// outside their selected format, malformed payloads, or invalid targets.
pub fn encode(instructions: &[Instruction]) -> Result<Vec<u16>> {
    validate_layout(instructions)?;
    let switch_bases = collect_switch_bases(instructions)?;
    let capacity = instructions.last().map_or(0, |instruction| {
        instruction
            .offset()
            .saturating_add(instruction.code_units().unwrap_or(u32::MAX))
    });
    let capacity = usize::try_from(capacity)
        .map_err(|_| Error::invalid_assembly("instruction stream does not fit this platform"))?;
    let mut output = Vec::with_capacity(capacity);

    for instruction in instructions {
        match instruction.data() {
            InstructionData::Operation { opcode, operands } => {
                encode_operation(&mut output, instruction.offset(), *opcode, operands)?;
            }
            InstructionData::PackedSwitchPayload(payload) => {
                let base = switch_base(&switch_bases, instruction.offset(), "packed-switch")?;
                encode_packed_switch(&mut output, instruction.offset(), base, payload)?;
            }
            InstructionData::SparseSwitchPayload(payload) => {
                let base = switch_base(&switch_bases, instruction.offset(), "sparse-switch")?;
                encode_sparse_switch(&mut output, instruction.offset(), base, payload)?;
            }
            InstructionData::ArrayDataPayload(payload) => {
                encode_array_data(&mut output, instruction.offset(), payload)?;
            }
        }
    }

    let decoded = decode(&output)?;
    if decoded != instructions {
        return Err(Error::invalid_assembly(
            "encoded instruction stream did not reproduce the supplied model",
        ));
    }
    Ok(output)
}

fn validate_layout(instructions: &[Instruction]) -> Result<()> {
    let mut expected = 0u32;
    for instruction in instructions {
        if instruction.offset() != expected {
            return Err(Error::invalid_assembly(format!(
                "instruction at {} should begin at contiguous offset {expected}",
                instruction.offset()
            )));
        }
        let width = instruction.code_units().ok_or_else(|| {
            Error::invalid_assembly(format!(
                "instruction at {} exceeds DEX address space",
                instruction.offset()
            ))
        })?;
        expected = expected.checked_add(width).ok_or_else(|| {
            Error::invalid_assembly("instruction layout exceeds DEX address space")
        })?;
    }
    Ok(())
}

fn collect_switch_bases(instructions: &[Instruction]) -> Result<BTreeMap<u32, u32>> {
    let mut bases = BTreeMap::new();
    for instruction in instructions {
        let InstructionData::Operation { opcode, operands } = instruction.data() else {
            continue;
        };
        if !matches!(opcode, Opcode::PackedSwitch | Opcode::SparseSwitch) {
            continue;
        }
        let Operands::RegisterBranch { target, .. } = operands else {
            return Err(operand_error(*opcode, "a register and payload target"));
        };
        if let Some(previous) = bases.insert(*target, instruction.offset())
            && previous != instruction.offset()
        {
            return Err(Error::invalid_assembly(format!(
                "switch payload at {target} is shared by instructions at {previous} and {}",
                instruction.offset()
            )));
        }
    }
    Ok(bases)
}

fn switch_base(bases: &BTreeMap<u32, u32>, payload: u32, kind: &str) -> Result<u32> {
    bases.get(&payload).copied().ok_or_else(|| {
        Error::invalid_assembly(format!(
            "{kind} payload at {payload} has no referring instruction"
        ))
    })
}

#[allow(clippy::too_many_lines)]
fn encode_operation(
    output: &mut Vec<u16>,
    offset: u32,
    opcode: Opcode,
    operands: &Operands,
) -> Result<()> {
    match (opcode.format(), operands) {
        (InstructionFormat::F10x, Operands::None) => output.push(word0(opcode, 0)),
        (InstructionFormat::F12x, Operands::Registers { first, second }) => {
            let first = nibble(*first, opcode, "first register")?;
            let second = nibble(*second, opcode, "second register")?;
            output.push(word0(opcode, first | (second << 4)));
        }
        (InstructionFormat::F11n, Operands::RegisterLiteral { register, literal }) => {
            let register = nibble(*register, opcode, "register")?;
            let literal = i8::try_from(*literal)
                .map_err(|_| range_error(opcode, "literal", *literal, -8, 7))?;
            if !(-8..=7).contains(&literal) {
                return Err(range_error(opcode, "literal", i64::from(literal), -8, 7));
            }
            let bits = literal.to_ne_bytes()[0] & 0x0f;
            output.push(word0(opcode, register | (bits << 4)));
        }
        (InstructionFormat::F11x, Operands::Register(register)) => {
            output.push(word0(opcode, byte(*register, opcode, "register")?));
        }
        (InstructionFormat::F10t, Operands::Branch { target }) => {
            let delta = branch_i8(opcode, offset, *target)?;
            output.push(word0(opcode, delta.to_ne_bytes()[0]));
        }
        (InstructionFormat::F20t, Operands::Branch { target }) => {
            output.push(word0(opcode, 0));
            output.push(i16_word(branch_i16(opcode, offset, *target)?));
        }
        (InstructionFormat::F22x, Operands::Registers { first, second }) => {
            output.push(word0(opcode, byte(*first, opcode, "first register")?));
            output.push(*second);
        }
        (InstructionFormat::F21t, Operands::RegisterBranch { register, target }) => {
            output.push(word0(opcode, byte(*register, opcode, "register")?));
            output.push(i16_word(branch_i16(opcode, offset, *target)?));
        }
        (InstructionFormat::F21s, Operands::RegisterLiteral { register, literal }) => {
            output.push(word0(opcode, byte(*register, opcode, "register")?));
            output.push(i16_word(literal_i16(opcode, *literal)?));
        }
        (InstructionFormat::F21h, Operands::RegisterLiteral { register, literal }) => {
            output.push(word0(opcode, byte(*register, opcode, "register")?));
            let shift = if opcode == Opcode::ConstWideHigh16 {
                48
            } else {
                16
            };
            let mask = (1_i64 << shift) - 1;
            if literal & mask != 0 {
                return Err(Error::invalid_assembly(format!(
                    "{} literal {literal} has nonzero low {shift} bits",
                    opcode.mnemonic()
                )));
            }
            output.push(i16_word(literal_i16(opcode, literal >> shift)?));
        }
        (InstructionFormat::F21c, Operands::RegisterIndex { register, index }) => {
            output.push(word0(opcode, byte(*register, opcode, "register")?));
            output.push(index_u16(opcode, *index)?);
        }
        (
            InstructionFormat::F23x,
            Operands::ThreeRegisters {
                first,
                second,
                third,
            },
        ) => {
            output.push(word0(opcode, byte(*first, opcode, "first register")?));
            output.push(
                u16::from(byte(*second, opcode, "second register")?)
                    | (u16::from(byte(*third, opcode, "third register")?) << 8),
            );
        }
        (
            InstructionFormat::F22t,
            Operands::RegistersBranch {
                first,
                second,
                target,
            },
        ) => {
            let first = nibble(*first, opcode, "first register")?;
            let second = nibble(*second, opcode, "second register")?;
            output.push(word0(opcode, first | (second << 4)));
            output.push(i16_word(branch_i16(opcode, offset, *target)?));
        }
        (
            InstructionFormat::F22s,
            Operands::RegistersLiteral {
                first,
                second,
                literal,
            },
        ) => {
            let first = nibble(*first, opcode, "first register")?;
            let second = nibble(*second, opcode, "second register")?;
            output.push(word0(opcode, first | (second << 4)));
            output.push(i16_word(literal_i16(opcode, *literal)?));
        }
        (
            InstructionFormat::F22c,
            Operands::RegistersIndex {
                first,
                second,
                index,
            },
        ) => {
            let first = nibble(*first, opcode, "first register")?;
            let second = nibble(*second, opcode, "second register")?;
            output.push(word0(opcode, first | (second << 4)));
            output.push(index_u16(opcode, *index)?);
        }
        (
            InstructionFormat::F22b,
            Operands::RegistersLiteral {
                first,
                second,
                literal,
            },
        ) => {
            output.push(word0(opcode, byte(*first, opcode, "first register")?));
            let second = byte(*second, opcode, "second register")?;
            let literal = i8::try_from(*literal)
                .map_err(|_| range_error(opcode, "literal", *literal, -128, 127))?;
            output.push(u16::from(second) | (u16::from(literal.to_ne_bytes()[0]) << 8));
        }
        (InstructionFormat::F30t, Operands::Branch { target }) => {
            output.push(word0(opcode, 0));
            push_i32(output, branch_i32(opcode, offset, *target)?);
        }
        (InstructionFormat::F32x, Operands::Registers { first, second }) => {
            output.extend_from_slice(&[word0(opcode, 0), *first, *second]);
        }
        (InstructionFormat::F31i, Operands::RegisterLiteral { register, literal }) => {
            output.push(word0(opcode, byte(*register, opcode, "register")?));
            let literal = i32::try_from(*literal).map_err(|_| {
                range_error(
                    opcode,
                    "literal",
                    *literal,
                    i64::from(i32::MIN),
                    i64::from(i32::MAX),
                )
            })?;
            push_i32(output, literal);
        }
        (InstructionFormat::F31t, Operands::RegisterBranch { register, target }) => {
            output.push(word0(opcode, byte(*register, opcode, "register")?));
            push_i32(output, branch_i32(opcode, offset, *target)?);
        }
        (InstructionFormat::F31c, Operands::RegisterIndex { register, index }) => {
            output.push(word0(opcode, byte(*register, opcode, "register")?));
            push_u32(output, *index);
        }
        (
            InstructionFormat::F35c,
            Operands::RegisterListIndex {
                registers,
                index,
                secondary_index: None,
            },
        ) => {
            encode_register_list(output, opcode, registers, *index, None)?;
        }
        (
            InstructionFormat::F3rc,
            Operands::RegisterRangeIndex {
                start,
                count,
                index,
                secondary_index: None,
            },
        ) => {
            encode_register_range(output, opcode, *start, *count, *index, None)?;
        }
        (
            InstructionFormat::F45cc,
            Operands::RegisterListIndex {
                registers,
                index,
                secondary_index: Some(secondary),
            },
        ) => {
            encode_register_list(output, opcode, registers, *index, Some(*secondary))?;
        }
        (
            InstructionFormat::F4rcc,
            Operands::RegisterRangeIndex {
                start,
                count,
                index,
                secondary_index: Some(secondary),
            },
        ) => {
            encode_register_range(output, opcode, *start, *count, *index, Some(*secondary))?;
        }
        (InstructionFormat::F51l, Operands::RegisterLiteral { register, literal }) => {
            output.push(word0(opcode, byte(*register, opcode, "register")?));
            push_i64(output, *literal);
        }
        _ => return Err(operand_error(opcode, expected_operands(opcode.format()))),
    }
    Ok(())
}

fn encode_register_list(
    output: &mut Vec<u16>,
    opcode: Opcode,
    registers: &[u16],
    index: u32,
    secondary: Option<u32>,
) -> Result<()> {
    let count = u8::try_from(registers.len()).map_err(|_| {
        Error::invalid_assembly(format!("{} has too many registers", opcode.mnemonic()))
    })?;
    if count > 5 {
        return Err(Error::invalid_assembly(format!(
            "{} register list has {count} entries; at most five fit",
            opcode.mnemonic()
        )));
    }
    let mut encoded = [0u8; 5];
    for (target, register) in encoded.iter_mut().zip(registers) {
        *target = nibble(*register, opcode, "register-list entry")?;
    }
    output.push(word0(opcode, (count << 4) | encoded[4]));
    output.push(index_u16(opcode, index)?);
    output.push(
        u16::from(encoded[0])
            | (u16::from(encoded[1]) << 4)
            | (u16::from(encoded[2]) << 8)
            | (u16::from(encoded[3]) << 12),
    );
    if let Some(secondary) = secondary {
        output.push(index_u16(opcode, secondary)?);
    }
    Ok(())
}

fn encode_register_range(
    output: &mut Vec<u16>,
    opcode: Opcode,
    start: u16,
    count: u8,
    index: u32,
    secondary: Option<u32>,
) -> Result<()> {
    if count != 0 && start.checked_add(u16::from(count) - 1).is_none() {
        return Err(Error::invalid_assembly(format!(
            "{} register range v{start}..+{count} exceeds v65535",
            opcode.mnemonic()
        )));
    }
    output.extend_from_slice(&[word0(opcode, count), index_u16(opcode, index)?, start]);
    if let Some(secondary) = secondary {
        output.push(index_u16(opcode, secondary)?);
    }
    Ok(())
}

fn encode_packed_switch(
    output: &mut Vec<u16>,
    offset: u32,
    base: u32,
    payload: &super::PackedSwitchPayload,
) -> Result<()> {
    require_payload_alignment(offset)?;
    let count = u16::try_from(payload.targets.len()).map_err(|_| {
        Error::invalid_assembly(format!(
            "packed-switch payload at {offset} has too many targets"
        ))
    })?;
    output.extend_from_slice(&[PACKED_SWITCH_PAYLOAD, count]);
    push_i32(output, payload.first_key);
    for target in &payload.targets {
        push_i32(output, branch_i32(Opcode::PackedSwitch, base, *target)?);
    }
    Ok(())
}

fn encode_sparse_switch(
    output: &mut Vec<u16>,
    offset: u32,
    base: u32,
    payload: &super::SparseSwitchPayload,
) -> Result<()> {
    require_payload_alignment(offset)?;
    if payload.keys.len() != payload.targets.len() {
        return Err(Error::invalid_assembly(format!(
            "sparse-switch payload at {offset} has {} keys but {} targets",
            payload.keys.len(),
            payload.targets.len()
        )));
    }
    if payload.keys.windows(2).any(|pair| pair[0] >= pair[1]) {
        return Err(Error::invalid_assembly(format!(
            "sparse-switch payload at {offset} keys are not strictly increasing"
        )));
    }
    let count = u16::try_from(payload.keys.len()).map_err(|_| {
        Error::invalid_assembly(format!(
            "sparse-switch payload at {offset} has too many targets"
        ))
    })?;
    output.extend_from_slice(&[SPARSE_SWITCH_PAYLOAD, count]);
    for key in &payload.keys {
        push_i32(output, *key);
    }
    for target in &payload.targets {
        push_i32(output, branch_i32(Opcode::SparseSwitch, base, *target)?);
    }
    Ok(())
}

fn encode_array_data(
    output: &mut Vec<u16>,
    offset: u32,
    payload: &super::ArrayDataPayload,
) -> Result<()> {
    require_payload_alignment(offset)?;
    if payload.element_width == 0 {
        return Err(Error::invalid_assembly(format!(
            "array-data payload at {offset} has zero-width elements"
        )));
    }
    let expected =
        usize::from(payload.element_width)
            .checked_mul(usize::try_from(payload.element_count).map_err(|_| {
                Error::invalid_assembly("array-data length does not fit this platform")
            })?)
            .ok_or_else(|| Error::invalid_assembly("array-data length overflowed"))?;
    if payload.data.len() != expected {
        return Err(Error::invalid_assembly(format!(
            "array-data payload at {offset} needs {expected} bytes but has {}",
            payload.data.len()
        )));
    }
    output.extend_from_slice(&[ARRAY_DATA_PAYLOAD, payload.element_width]);
    push_u32(output, payload.element_count);
    for pair in payload.data.chunks(2) {
        let low = pair[0];
        let high = pair.get(1).copied().unwrap_or(0);
        output.push(u16::from_le_bytes([low, high]));
    }
    Ok(())
}

fn expected_operands(format: InstructionFormat) -> &'static str {
    match format {
        InstructionFormat::F10x => "no operands",
        InstructionFormat::F12x | InstructionFormat::F22x | InstructionFormat::F32x => {
            "two registers"
        }
        InstructionFormat::F11n
        | InstructionFormat::F21s
        | InstructionFormat::F21h
        | InstructionFormat::F31i
        | InstructionFormat::F51l => "a register and literal",
        InstructionFormat::F11x => "one register",
        InstructionFormat::F10t | InstructionFormat::F20t | InstructionFormat::F30t => {
            "a branch target"
        }
        InstructionFormat::F21t | InstructionFormat::F31t => "a register and branch target",
        InstructionFormat::F21c | InstructionFormat::F31c => "a register and index",
        InstructionFormat::F23x => "three registers",
        InstructionFormat::F22t => "two registers and a branch target",
        InstructionFormat::F22s | InstructionFormat::F22b => "two registers and a literal",
        InstructionFormat::F22c => "two registers and an index",
        InstructionFormat::F35c | InstructionFormat::F45cc => "a register list and indices",
        InstructionFormat::F3rc | InstructionFormat::F4rcc => "a register range and indices",
    }
}

fn operand_error(opcode: Opcode, expected: &str) -> Error {
    Error::invalid_assembly(format!(
        "{} expects {expected} for format {:?}",
        opcode.mnemonic(),
        opcode.format()
    ))
}

fn require_payload_alignment(offset: u32) -> Result<()> {
    if offset.is_multiple_of(2) {
        Ok(())
    } else {
        Err(Error::invalid_assembly(format!(
            "payload at {offset} is not four-byte aligned"
        )))
    }
}

fn word0(opcode: Opcode, high: u8) -> u16 {
    u16::from(opcode.byte()) | (u16::from(high) << 8)
}

fn nibble(value: u16, opcode: Opcode, name: &str) -> Result<u8> {
    let value = u8::try_from(value)
        .ok()
        .filter(|value| *value <= 0x0f)
        .ok_or_else(|| range_error(opcode, name, i64::from(value), 0, 15))?;
    Ok(value)
}

fn byte(value: u16, opcode: Opcode, name: &str) -> Result<u8> {
    u8::try_from(value).map_err(|_| range_error(opcode, name, i64::from(value), 0, 255))
}

fn index_u16(opcode: Opcode, value: u32) -> Result<u16> {
    u16::try_from(value).map_err(|_| {
        Error::invalid_assembly(format!(
            "{} index {value} does not fit its 16-bit format",
            opcode.mnemonic()
        ))
    })
}

fn literal_i16(opcode: Opcode, value: i64) -> Result<i16> {
    i16::try_from(value).map_err(|_| {
        range_error(
            opcode,
            "literal",
            value,
            i64::from(i16::MIN),
            i64::from(i16::MAX),
        )
    })
}

fn branch_delta(opcode: Opcode, source: u32, target: u32) -> Result<i64> {
    i64::from(target)
        .checked_sub(i64::from(source))
        .ok_or_else(|| {
            Error::invalid_assembly(format!("{} branch delta overflowed", opcode.mnemonic()))
        })
}

fn branch_i8(opcode: Opcode, source: u32, target: u32) -> Result<i8> {
    let delta = branch_delta(opcode, source, target)?;
    i8::try_from(delta).map_err(|_| range_error(opcode, "branch delta", delta, -128, 127))
}

fn branch_i16(opcode: Opcode, source: u32, target: u32) -> Result<i16> {
    let delta = branch_delta(opcode, source, target)?;
    i16::try_from(delta).map_err(|_| {
        range_error(
            opcode,
            "branch delta",
            delta,
            i64::from(i16::MIN),
            i64::from(i16::MAX),
        )
    })
}

fn branch_i32(opcode: Opcode, source: u32, target: u32) -> Result<i32> {
    let delta = branch_delta(opcode, source, target)?;
    i32::try_from(delta).map_err(|_| {
        range_error(
            opcode,
            "branch delta",
            delta,
            i64::from(i32::MIN),
            i64::from(i32::MAX),
        )
    })
}

fn range_error(opcode: Opcode, name: &str, value: i64, minimum: i64, maximum: i64) -> Error {
    Error::invalid_assembly(format!(
        "{} {name} {value} is outside {minimum}..={maximum}",
        opcode.mnemonic()
    ))
}

fn i16_word(value: i16) -> u16 {
    u16::from_ne_bytes(value.to_ne_bytes())
}

fn push_u32(output: &mut Vec<u16>, value: u32) {
    let bytes = value.to_le_bytes();
    output.extend_from_slice(&[
        u16::from_le_bytes([bytes[0], bytes[1]]),
        u16::from_le_bytes([bytes[2], bytes[3]]),
    ]);
}

fn push_i32(output: &mut Vec<u16>, value: i32) {
    push_u32(output, u32::from_ne_bytes(value.to_ne_bytes()));
}

fn push_i64(output: &mut Vec<u16>, value: i64) {
    let bytes = value.to_le_bytes();
    output.extend_from_slice(&[
        u16::from_le_bytes([bytes[0], bytes[1]]),
        u16::from_le_bytes([bytes[2], bytes[3]]),
        u16::from_le_bytes([bytes[4], bytes[5]]),
        u16::from_le_bytes([bytes[6], bytes[7]]),
    ]);
}

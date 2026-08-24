//! Structural method-body validation and implicit Dalvik associations.

use std::collections::{BTreeMap, BTreeSet};

use crate::file::{CatchHandler, CodeItem, TryBlock, TypeIndex};
use crate::instruction::{InstructionData, Opcode, Operands, encode};
use crate::{Error, Result};

use super::{
    AnalyzedInstruction, BodyAnalysis, PayloadKind, PayloadLink, ProducedValue, ValueKind,
    instruction_semantics,
};

const FIRST_CODE_UNIT_OFFSET: u32 = 0;

/// Validates one code item and recovers relationships implicit in DEX layout.
///
/// This checks encodability, register-frame bounds, protected regions,
/// exception targets, `move-result` adjacency, `move-exception` placement, and
/// executable-instruction to payload links. The returned facts are independent
/// of identifier resolution and can therefore be reused by metadata adapters.
///
/// # Errors
///
/// Returns an error for a malformed or unrepresentable method body.
pub fn analyze_body(code: &CodeItem) -> Result<BodyAnalysis> {
    if code.instructions.is_empty() {
        return Err(Error::invalid_instruction(
            FIRST_CODE_UNIT_OFFSET,
            "method instruction stream is empty",
        ));
    }
    if code.ins_size > code.registers_size {
        return Err(Error::invalid_instruction(
            FIRST_CODE_UNIT_OFFSET,
            format!(
                "method declares {} incoming words but only {} registers",
                code.ins_size, code.registers_size
            ),
        ));
    }

    // The checked encoder is the canonical validation for edited instruction
    // layout, binary operand widths, branches, payload shape, and round trips.
    let encoded = encode(&code.instructions)?;
    let stream_end = u32::try_from(encoded.len())
        .map_err(|_| Error::invalid_instruction(u32::MAX, "instruction stream is too large"))?;

    let mut positions = BTreeMap::new();
    let mut operation_offsets = BTreeSet::new();
    let mut analyzed = Vec::with_capacity(code.instructions.len());
    for (position, instruction) in code.instructions.iter().enumerate() {
        positions.insert(instruction.offset(), position);
        if matches!(instruction.data(), InstructionData::Operation { .. }) {
            operation_offsets.insert(instruction.offset());
        }
        let semantics = instruction_semantics(instruction)?;
        validate_registers(instruction.offset(), &semantics.reads, code.registers_size)?;
        validate_registers(instruction.offset(), &semantics.writes, code.registers_size)?;
        analyzed.push(AnalyzedInstruction {
            offset: instruction.offset(),
            semantics,
            result_producer: None,
            result_consumer: None,
            payload: payload_link(instruction.data()),
            handler_types: Vec::new(),
        });
    }

    let handler_types = validate_tries(code, stream_end, &operation_offsets)?;
    for (offset, types) in handler_types {
        let position = positions[&offset];
        analyzed[position].handler_types = types;
    }
    link_results(code, &mut analyzed)?;
    validate_move_exceptions(code, &analyzed)?;

    let mut payload_users: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
    for item in &analyzed {
        if let Some(link) = item.payload {
            payload_users
                .entry(link.payload_offset)
                .or_default()
                .push(item.offset);
        }
    }

    Ok(BodyAnalysis {
        stream_end,
        instructions: analyzed,
        positions,
        payload_users,
    })
}

fn validate_registers(
    offset: u32,
    operands: &[super::RegisterOperand],
    register_count: u16,
) -> Result<()> {
    for operand in operands {
        let end = u32::from(operand.register)
            .checked_add(u32::from(operand.register_words()))
            .ok_or_else(|| Error::invalid_instruction(offset, "register span overflowed"))?;
        if end > u32::from(register_count) {
            return Err(Error::invalid_instruction(
                offset,
                format!(
                    "register span v{}..v{} exceeds the {}-register frame",
                    operand.register,
                    end.saturating_sub(1),
                    register_count
                ),
            ));
        }
    }
    Ok(())
}

fn payload_link(data: &InstructionData) -> Option<PayloadLink> {
    let InstructionData::Operation { opcode, operands } = data else {
        return None;
    };
    let Operands::RegisterBranch { target, .. } = operands else {
        return None;
    };
    let kind = match opcode {
        Opcode::PackedSwitch => PayloadKind::PackedSwitch,
        Opcode::SparseSwitch => PayloadKind::SparseSwitch,
        Opcode::FillArrayData => PayloadKind::ArrayData,
        _ => return None,
    };
    Some(PayloadLink {
        kind,
        payload_offset: *target,
    })
}

fn validate_tries(
    code: &CodeItem,
    stream_end: u32,
    operations: &BTreeSet<u32>,
) -> Result<BTreeMap<u32, Vec<Option<TypeIndex>>>> {
    let mut previous_end = None;
    let mut entries: BTreeMap<u32, Vec<Option<TypeIndex>>> = BTreeMap::new();
    for try_block in &code.tries {
        let end = validate_try_range(try_block, stream_end, operations)?;
        if previous_end.is_some_and(|previous| try_block.start_address < previous) {
            return Err(Error::invalid_instruction(
                try_block.start_address,
                "protected instruction ranges overlap or are out of order",
            ));
        }
        previous_end = Some(end);
        validate_handlers(try_block, stream_end, operations, &mut entries)?;
    }
    Ok(entries)
}

fn validate_try_range(
    try_block: &TryBlock,
    stream_end: u32,
    operations: &BTreeSet<u32>,
) -> Result<u32> {
    let end = try_block
        .start_address
        .checked_add(u32::from(try_block.instruction_count))
        .ok_or_else(|| {
            Error::invalid_instruction(try_block.start_address, "protected range overflowed")
        })?;
    if try_block.instruction_count == 0 || end > stream_end {
        return Err(Error::invalid_instruction(
            try_block.start_address,
            "protected instruction range is empty or outside the method",
        ));
    }
    if !operations.contains(&try_block.start_address)
        || (end != stream_end && !operations.contains(&end))
    {
        return Err(Error::invalid_instruction(
            try_block.start_address,
            "protected range does not use executable instruction boundaries",
        ));
    }
    Ok(end)
}

fn validate_handlers(
    try_block: &TryBlock,
    stream_end: u32,
    operations: &BTreeSet<u32>,
    entries: &mut BTreeMap<u32, Vec<Option<TypeIndex>>>,
) -> Result<()> {
    if try_block.handlers.is_empty() {
        return Err(Error::invalid_instruction(
            try_block.start_address,
            "exception handler list is empty",
        ));
    }
    let mut seen = BTreeSet::new();
    let mut saw_catch_all = false;
    for (position, handler) in try_block.handlers.iter().enumerate() {
        validate_handler_target(handler, stream_end, operations)?;
        match handler.exception_type {
            Some(exception_type) if !saw_catch_all && seen.insert(exception_type) => {}
            Some(_) if saw_catch_all => {
                return Err(Error::invalid_instruction(
                    handler.address,
                    "typed handler follows a catch-all handler",
                ));
            }
            Some(_) => {
                return Err(Error::invalid_instruction(
                    handler.address,
                    "duplicate exception type in one handler list",
                ));
            }
            None if position + 1 == try_block.handlers.len() && !saw_catch_all => {
                saw_catch_all = true;
            }
            None => {
                return Err(Error::invalid_instruction(
                    handler.address,
                    "catch-all handler is not the sole final catch-all",
                ));
            }
        }
        let types = entries.entry(handler.address).or_default();
        if !types.contains(&handler.exception_type) {
            types.push(handler.exception_type);
        }
    }
    Ok(())
}

fn validate_handler_target(
    handler: &CatchHandler,
    stream_end: u32,
    operations: &BTreeSet<u32>,
) -> Result<()> {
    if handler.address < stream_end && operations.contains(&handler.address) {
        Ok(())
    } else {
        Err(Error::invalid_instruction(
            handler.address,
            "exception handler target is not an executable instruction boundary",
        ))
    }
}

fn link_results(code: &CodeItem, analyzed: &mut [AnalyzedInstruction]) -> Result<()> {
    for position in 0..code.instructions.len() {
        let InstructionData::Operation { opcode, .. } = code.instructions[position].data() else {
            continue;
        };
        let Some(required) = move_result_kind(*opcode) else {
            continue;
        };
        let producer_position = position.checked_sub(1).ok_or_else(|| {
            Error::invalid_instruction(
                code.instructions[position].offset(),
                "move-result has no immediately preceding producer",
            )
        })?;
        let producer = &analyzed[producer_position];
        let produced_value = producer.semantics.produced.ok_or_else(|| {
            Error::invalid_instruction(
                code.instructions[position].offset(),
                "move-result has no immediately preceding producer",
            )
        })?;
        if produced_value == ProducedValue::Reference && required != ValueKind::Reference {
            return Err(Error::invalid_instruction(
                code.instructions[position].offset(),
                "filled-new-array result requires move-result-object",
            ));
        }
        let producer_offset = producer.offset;
        let consumer_offset = analyzed[position].offset;
        analyzed[position].result_producer = Some(producer_offset);
        analyzed[producer_position].result_consumer = Some(consumer_offset);
    }
    Ok(())
}

const fn move_result_kind(opcode: Opcode) -> Option<ValueKind> {
    match opcode {
        Opcode::MoveResult => Some(ValueKind::Single),
        Opcode::MoveResultWide => Some(ValueKind::Wide),
        Opcode::MoveResultObject => Some(ValueKind::Reference),
        _ => None,
    }
}

fn validate_move_exceptions(code: &CodeItem, analyzed: &[AnalyzedInstruction]) -> Result<()> {
    for (instruction, facts) in code.instructions.iter().zip(analyzed) {
        if matches!(
            instruction.data(),
            InstructionData::Operation {
                opcode: Opcode::MoveException,
                ..
            }
        ) && facts.handler_types.is_empty()
        {
            return Err(Error::invalid_instruction(
                instruction.offset(),
                "move-exception is not at an exception-handler entry",
            ));
        }
    }
    Ok(())
}

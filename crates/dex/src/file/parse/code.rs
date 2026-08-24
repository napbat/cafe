//! Code items, instruction streams, tries, and handlers.

use std::collections::{BTreeMap, BTreeSet};

use crate::instruction::{Instruction, InstructionData, decode};
use crate::{Error, Result};

use super::{Context, debug};
use crate::file::layout::{Alignment, CodeField, ItemWidth, TryField};
use crate::file::model::{CatchHandler, CodeItem, TryBlock, TypeIndex};

const CODE_ITEM_PADDING_VALUE: u16 = 0;
const CODE_UNITS_PER_WORD: u32 = 2;

pub(super) fn item(context: &Context<'_>, encoded_offset: u32) -> Result<CodeItem> {
    let offset = context.offset(encoded_offset, Alignment::Word, "code item")?;
    let registers_size = context
        .reader
        .u16(offset + CodeField::RegistersSize.offset())?;
    let ins_size = context.reader.u16(offset + CodeField::InsSize.offset())?;
    let outs_size = context.reader.u16(offset + CodeField::OutsSize.offset())?;
    let tries_size = context.reader.u16(offset + CodeField::TriesSize.offset())?;
    let debug_info_offset = context
        .reader
        .u32(offset + CodeField::DebugInfoOffset.offset())?;
    let instruction_count = context
        .reader
        .u32(offset + CodeField::InstructionsSize.offset())?;
    if ins_size > registers_size {
        return Err(Error::invalid_dex(
            offset + CodeField::InsSize.offset(),
            "incoming register width exceeds frame size",
        ));
    }
    let instructions_offset = offset + CodeField::Instructions.offset();
    let count = context.count(
        instruction_count,
        ItemWidth::CODE_UNIT,
        instructions_offset,
        "instruction code units",
    )?;
    let mut words = Vec::with_capacity(count);
    for index in 0..count {
        words.push(
            context
                .reader
                .u16(instructions_offset + index * ItemWidth::CODE_UNIT.bytes())?,
        );
    }
    let instructions = decode(&words)?;
    validate_registers(&instructions, registers_size)?;
    let debug_info = debug::info(
        context,
        debug_info_offset,
        registers_size,
        instruction_count,
    )?;
    let tries = if tries_size == 0 {
        Vec::new()
    } else {
        parse_tries(
            context,
            offset,
            instruction_count,
            tries_size,
            &instructions,
        )?
    };
    Ok(CodeItem {
        registers_size,
        ins_size,
        outs_size,
        instructions,
        tries,
        debug_info,
        data_offset: encoded_offset,
    })
}

fn parse_tries(
    context: &Context<'_>,
    code_offset: usize,
    instruction_count: u32,
    tries_size: u16,
    instructions: &[Instruction],
) -> Result<Vec<TryBlock>> {
    let instruction_bytes = usize::try_from(instruction_count)
        .ok()
        .and_then(|count| count.checked_mul(ItemWidth::CODE_UNIT.bytes()))
        .ok_or_else(|| Error::invalid_dex(code_offset, "instruction byte size overflowed"))?;
    let mut tries_offset = code_offset
        .checked_add(ItemWidth::CODE_HEADER.bytes() + instruction_bytes)
        .ok_or_else(|| Error::invalid_dex(code_offset, "try offset overflowed"))?;
    if !instruction_count.is_multiple_of(CODE_UNITS_PER_WORD) {
        if context.reader.u16(tries_offset)? != CODE_ITEM_PADDING_VALUE {
            return Err(Error::invalid_dex(
                tries_offset,
                "code-item alignment padding is nonzero",
            ));
        }
        tries_offset += ItemWidth::CODE_UNIT.bytes();
    }
    let tries_count = usize::from(tries_size);
    let handlers_offset = tries_offset
        .checked_add(tries_count * ItemWidth::TRY_ITEM.bytes())
        .ok_or_else(|| Error::invalid_dex(tries_offset, "try table size overflowed"))?;
    context
        .reader
        .bytes(tries_offset, tries_count * ItemWidth::TRY_ITEM.bytes())?;
    let handlers = parse_handlers(context, handlers_offset, instruction_count, instructions)?;
    let boundaries = operation_boundaries(instructions, instruction_count);
    let mut tries = Vec::with_capacity(tries_count);
    let mut previous_end = 0u32;
    for index in 0..tries_count {
        let item_offset = tries_offset + index * ItemWidth::TRY_ITEM.bytes();
        let start_address = context
            .reader
            .u32(item_offset + TryField::StartAddress.offset())?;
        let count = context
            .reader
            .u16(item_offset + TryField::InstructionCount.offset())?;
        let handler_offset = u32::from(
            context
                .reader
                .u16(item_offset + TryField::HandlerOffset.offset())?,
        );
        let end = start_address.checked_add(u32::from(count)).ok_or_else(|| {
            Error::invalid_dex(item_offset, "protected instruction range overflowed")
        })?;
        if count == 0 || end > instruction_count {
            return Err(Error::invalid_dex(
                item_offset,
                "protected instruction range is empty or outside the method",
            ));
        }
        if !boundaries.contains(&start_address) || !boundaries.contains(&end) {
            return Err(Error::invalid_dex(
                item_offset,
                "protected range does not use instruction boundaries",
            ));
        }
        if index != 0 && start_address < previous_end {
            return Err(Error::invalid_dex(item_offset, "protected ranges overlap"));
        }
        previous_end = end;
        let catches = handlers.get(&handler_offset).cloned().ok_or_else(|| {
            Error::invalid_dex(
                item_offset + TryField::HandlerOffset.offset(),
                format!("handler offset {handler_offset} is not a handler boundary"),
            )
        })?;
        tries.push(TryBlock {
            start_address,
            instruction_count: count,
            handlers: catches,
        });
    }
    Ok(tries)
}

fn parse_handlers(
    context: &Context<'_>,
    offset: usize,
    instruction_count: u32,
    instructions: &[Instruction],
) -> Result<BTreeMap<u32, Vec<CatchHandler>>> {
    let mut cursor = context.reader.cursor(offset)?;
    let count = cursor.uleb128()?;
    let count = context.count(
        count,
        ItemWidth::EXCEPTION_HANDLER_MINIMUM,
        cursor.position(),
        "exception handler lists",
    )?;
    let operation_offsets: BTreeSet<_> = instructions
        .iter()
        .filter(|instruction| matches!(instruction.data(), InstructionData::Operation { .. }))
        .map(Instruction::offset)
        .collect();
    let mut handlers = BTreeMap::new();
    for _ in 0..count {
        let relative_offset =
            u32::try_from(cursor.position().saturating_sub(offset)).map_err(|_| {
                Error::invalid_dex(
                    cursor.position(),
                    "exception handler offset exceeds 32 bits",
                )
            })?;
        let encoded_count = cursor.sleb128()?;
        let typed_count = encoded_count.checked_abs().ok_or_else(|| {
            Error::invalid_dex(cursor.position(), "exception handler count overflowed")
        })?;
        let typed_count = context.count(
            u32::try_from(typed_count).map_err(|_| {
                Error::invalid_dex(cursor.position(), "exception handler count is negative")
            })?,
            ItemWidth::TYPED_CATCH_MINIMUM,
            cursor.position(),
            "typed exception handlers",
        )?;
        let mut catches = Vec::with_capacity(typed_count + usize::from(encoded_count <= 0));
        let mut seen_types = BTreeSet::new();
        for _ in 0..typed_count {
            let entry_offset = cursor.position();
            let exception_type = cursor.uleb128()?;
            context.index(
                exception_type,
                context.header.type_ids.size,
                entry_offset,
                "exception type",
            )?;
            if !seen_types.insert(exception_type) {
                return Err(Error::invalid_dex(
                    entry_offset,
                    "duplicate exception type in one handler",
                ));
            }
            let address = cursor.uleb128()?;
            require_handler_target(address, instruction_count, &operation_offsets, entry_offset)?;
            catches.push(CatchHandler {
                exception_type: Some(TypeIndex(exception_type)),
                address,
            });
        }
        if encoded_count <= 0 {
            let entry_offset = cursor.position();
            let address = cursor.uleb128()?;
            require_handler_target(address, instruction_count, &operation_offsets, entry_offset)?;
            catches.push(CatchHandler {
                exception_type: None,
                address,
            });
        }
        if handlers.insert(relative_offset, catches).is_some() {
            return Err(Error::invalid_dex(
                offset,
                "duplicate exception handler offset",
            ));
        }
    }
    Ok(handlers)
}

fn require_handler_target(
    address: u32,
    instruction_count: u32,
    operations: &BTreeSet<u32>,
    offset: usize,
) -> Result<()> {
    if address < instruction_count && operations.contains(&address) {
        Ok(())
    } else {
        Err(Error::invalid_dex(
            offset,
            format!("exception handler target {address} is not an instruction boundary"),
        ))
    }
}

fn operation_boundaries(instructions: &[Instruction], end: u32) -> BTreeSet<u32> {
    let mut offsets: BTreeSet<_> = instructions
        .iter()
        .filter(|instruction| matches!(instruction.data(), InstructionData::Operation { .. }))
        .map(Instruction::offset)
        .collect();
    offsets.insert(end);
    offsets
}

fn validate_registers(instructions: &[Instruction], registers_size: u16) -> Result<()> {
    for instruction in instructions {
        let InstructionData::Operation { opcode, operands } = instruction.data() else {
            continue;
        };
        for register in registers(operands) {
            if register >= registers_size {
                return Err(Error::invalid_instruction(
                    instruction.offset(),
                    format!(
                        "{} references v{register}, but the frame has {registers_size} registers",
                        opcode.mnemonic()
                    ),
                ));
            }
        }
    }
    Ok(())
}

fn registers(operands: &crate::instruction::Operands) -> Vec<u16> {
    use crate::instruction::Operands;
    match operands {
        Operands::None | Operands::Branch { .. } => Vec::new(),
        Operands::Register(register)
        | Operands::RegisterLiteral { register, .. }
        | Operands::RegisterBranch { register, .. }
        | Operands::RegisterIndex { register, .. } => vec![*register],
        Operands::Registers { first, second }
        | Operands::RegistersLiteral { first, second, .. }
        | Operands::RegistersBranch { first, second, .. }
        | Operands::RegistersIndex { first, second, .. } => vec![*first, *second],
        Operands::ThreeRegisters {
            first,
            second,
            third,
        } => vec![*first, *second, *third],
        Operands::RegisterListIndex { registers, .. } => registers.clone(),
        Operands::RegisterRangeIndex { start, count, .. } => {
            if *count == 0 {
                Vec::new()
            } else {
                vec![*start, start.saturating_add(u16::from(*count) - 1)]
            }
        }
    }
}

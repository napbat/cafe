//! DEX debug-state-machine parsing and validation.

use crate::{Error, Result};

use super::Context;
use crate::file::header::ABSENT_OFFSET;
use crate::file::layout::{Alignment, ItemWidth};
use crate::file::model::{
    DEBUG_LINE_BASE, DEBUG_LINE_RANGE, DebugEvent, DebugInfo, DebugOpcode,
    FIRST_SPECIAL_DEBUG_OPCODE, INITIAL_DEBUG_ADDRESS, MINIMUM_DEBUG_LINE, StringIndex, TypeIndex,
};

pub(super) fn info(
    context: &Context<'_>,
    encoded_offset: u32,
    registers_size: u16,
    instruction_count: u32,
) -> Result<Option<DebugInfo>> {
    if encoded_offset == ABSENT_OFFSET {
        return Ok(None);
    }
    let offset = context.offset(encoded_offset, Alignment::Byte, "debug information")?;
    let mut cursor = context.reader.cursor(offset)?;
    let line_start = cursor.uleb128()?;
    if line_start < MINIMUM_DEBUG_LINE {
        return Err(Error::invalid_dex(offset, "debug line start is zero"));
    }
    let parameter_count = cursor.uleb128()?;
    let parameter_count = context.count(
        parameter_count,
        ItemWidth::BYTE,
        cursor.position(),
        "debug parameter names",
    )?;
    let mut parameter_names = Vec::with_capacity(parameter_count);
    for _ in 0..parameter_count {
        parameter_names.push(optional_string(context, &mut cursor)?);
    }

    let mut address = INITIAL_DEBUG_ADDRESS;
    let mut line = i64::from(line_start);
    let mut events = Vec::new();
    loop {
        let event_offset = cursor.position();
        let opcode = cursor.u8()?;
        let event = match DebugOpcode::from_byte(opcode) {
            Some(DebugOpcode::EndSequence) => {
                events.push(DebugEvent::EndSequence);
                break;
            }
            Some(DebugOpcode::AdvancePc) => {
                let delta = cursor.uleb128()?;
                address = advance_address(address, delta, instruction_count, event_offset)?;
                DebugEvent::AdvancePc(delta)
            }
            Some(DebugOpcode::AdvanceLine) => {
                let delta = cursor.sleb128()?;
                line = advance_line(line, delta, event_offset)?;
                DebugEvent::AdvanceLine(delta)
            }
            Some(DebugOpcode::StartLocal) => DebugEvent::StartLocal {
                register: register(context, &mut cursor, registers_size)?,
                name: optional_string(context, &mut cursor)?,
                local_type: optional_type(context, &mut cursor)?,
            },
            Some(DebugOpcode::StartLocalExtended) => DebugEvent::StartLocalExtended {
                register: register(context, &mut cursor, registers_size)?,
                name: optional_string(context, &mut cursor)?,
                local_type: optional_type(context, &mut cursor)?,
                signature: optional_string(context, &mut cursor)?,
            },
            Some(DebugOpcode::EndLocal) => {
                DebugEvent::EndLocal(register(context, &mut cursor, registers_size)?)
            }
            Some(DebugOpcode::RestartLocal) => {
                DebugEvent::RestartLocal(register(context, &mut cursor, registers_size)?)
            }
            Some(DebugOpcode::SetPrologueEnd) => DebugEvent::SetPrologueEnd,
            Some(DebugOpcode::SetEpilogueBegin) => DebugEvent::SetEpilogueBegin,
            Some(DebugOpcode::SetFile) => {
                DebugEvent::SetFile(optional_string(context, &mut cursor)?)
            }
            None => {
                let adjusted = u32::from(opcode - FIRST_SPECIAL_DEBUG_OPCODE);
                let address_delta = adjusted / DEBUG_LINE_RANGE;
                let line_delta = DEBUG_LINE_BASE
                    + i32::try_from(adjusted % DEBUG_LINE_RANGE).map_err(|_| {
                        Error::invalid_dex(event_offset, "special debug line delta overflowed")
                    })?;
                address = advance_address(address, address_delta, instruction_count, event_offset)?;
                line = advance_line(line, line_delta, event_offset)?;
                DebugEvent::Position {
                    address_delta,
                    line_delta,
                }
            }
        };
        events.push(event);
    }
    Ok(Some(DebugInfo {
        line_start,
        parameter_names,
        events,
        data_offset: encoded_offset,
    }))
}

fn register(
    context: &Context<'_>,
    cursor: &mut crate::file::io::Cursor<'_>,
    registers_size: u16,
) -> Result<u32> {
    let offset = cursor.position();
    let register = cursor.uleb128()?;
    context.index(
        register,
        u32::from(registers_size),
        offset,
        "debug register",
    )
}

fn optional_string(
    context: &Context<'_>,
    cursor: &mut crate::file::io::Cursor<'_>,
) -> Result<Option<StringIndex>> {
    let offset = cursor.position();
    cursor
        .uleb128p1()?
        .map(|index| {
            context
                .index(
                    index,
                    context.header.string_ids.size,
                    offset,
                    "debug string",
                )
                .map(StringIndex)
        })
        .transpose()
}

fn optional_type(
    context: &Context<'_>,
    cursor: &mut crate::file::io::Cursor<'_>,
) -> Result<Option<TypeIndex>> {
    let offset = cursor.position();
    cursor
        .uleb128p1()?
        .map(|index| {
            context
                .index(index, context.header.type_ids.size, offset, "debug type")
                .map(TypeIndex)
        })
        .transpose()
}

fn advance_address(current: u32, delta: u32, limit: u32, offset: usize) -> Result<u32> {
    let next = current
        .checked_add(delta)
        .ok_or_else(|| Error::invalid_dex(offset, "debug address overflowed"))?;
    if next <= limit {
        Ok(next)
    } else {
        Err(Error::invalid_dex(
            offset,
            format!("debug address {next} exceeds instruction size {limit}"),
        ))
    }
}

fn advance_line(current: i64, delta: i32, offset: usize) -> Result<i64> {
    let next = current
        .checked_add(i64::from(delta))
        .ok_or_else(|| Error::invalid_dex(offset, "debug line overflowed"))?;
    if next >= i64::from(MINIMUM_DEBUG_LINE) {
        Ok(next)
    } else {
        Err(Error::invalid_dex(offset, "debug line fell below one"))
    }
}

//! Debug programs, code items, tries, and delta-encoded class data.

use std::collections::BTreeMap;

use crate::file::header::ABSENT_OFFSET;
use crate::file::io::Writer;
use crate::file::layout::{Alignment, EMPTY_ITEM_COUNT, ITEM_COUNT_INCREMENT, ItemWidth, TryField};
use crate::file::model::{
    CatchHandler, ClassDefinition, DEBUG_LINE_BASE, DEBUG_LINE_RANGE, DebugEvent, DebugInfo,
    DebugOpcode, EncodedCatchHandlerCount, EncodedMethod, FIRST_SPECIAL_DEBUG_OPCODE, MapItem,
    MapItemType, MethodIndex, StringIndex, TypeIndex,
};
use crate::instruction::encode;
use crate::{Error, Result};

pub(super) struct CodeLayout {
    pub(super) class_data_offsets: Vec<u32>,
    pub(super) sections: Vec<MapItem>,
}

pub(super) fn write(writer: &mut Writer, classes: &[ClassDefinition]) -> Result<CodeLayout> {
    let mut sections = Vec::new();
    let (debug_offsets, debug_section) = write_debug_info(writer, classes)?;
    if let Some(section) = debug_section {
        sections.push(section);
    }
    let (code_offsets, code_section) = write_code_items(writer, classes, &debug_offsets)?;
    if let Some(section) = code_section {
        sections.push(section);
    }
    let (class_data_offsets, class_data_section) =
        write_class_data(writer, classes, &code_offsets)?;
    if let Some(section) = class_data_section {
        sections.push(section);
    }
    Ok(CodeLayout {
        class_data_offsets,
        sections,
    })
}

fn write_debug_info(
    writer: &mut Writer,
    classes: &[ClassDefinition],
) -> Result<(BTreeMap<MethodIndex, u32>, Option<MapItem>)> {
    let start = writer.position()?;
    let mut offsets = BTreeMap::new();
    let mut count = EMPTY_ITEM_COUNT;
    for method in methods(classes) {
        let Some(debug) = method
            .code
            .as_ref()
            .and_then(|code| code.debug_info.as_ref())
        else {
            continue;
        };
        let offset = writer.position()?;
        write_debug_program(writer, debug)?;
        if offsets.insert(method.method, offset).is_some() {
            return Err(Error::invalid_assembly(
                "a method has more than one debug program",
            ));
        }
        count = count
            .checked_add(ITEM_COUNT_INCREMENT)
            .ok_or_else(|| Error::invalid_assembly("debug item count overflowed"))?;
    }
    Ok((offsets, section(MapItemType::DebugInfo, count, start)))
}

fn write_debug_program(writer: &mut Writer, debug: &DebugInfo) -> Result<()> {
    writer.uleb128(debug.line_start);
    writer.uleb128(u32::try_from(debug.parameter_names.len()).map_err(|_| {
        Error::invalid_assembly("debug parameter count exceeds 32-bit address space")
    })?);
    for name in &debug.parameter_names {
        writer.uleb128p1(name.map(StringIndex::get))?;
    }
    let Some((last, preceding)) = debug.events.split_last() else {
        return Err(Error::invalid_assembly(
            "debug program has no final end-sequence event",
        ));
    };
    if !matches!(last, DebugEvent::EndSequence) {
        return Err(Error::invalid_assembly(
            "debug program has no final end-sequence event",
        ));
    }
    if preceding
        .iter()
        .any(|event| matches!(event, DebugEvent::EndSequence))
    {
        return Err(Error::invalid_assembly(
            "debug end-sequence event is not last",
        ));
    }
    for event in &debug.events {
        write_debug_event(writer, event)?;
    }
    Ok(())
}

fn write_debug_event(writer: &mut Writer, event: &DebugEvent) -> Result<()> {
    match event {
        DebugEvent::EndSequence => writer.u8(DebugOpcode::EndSequence.byte()),
        DebugEvent::AdvancePc(delta) => {
            writer.u8(DebugOpcode::AdvancePc.byte());
            writer.uleb128(*delta);
        }
        DebugEvent::AdvanceLine(delta) => {
            writer.u8(DebugOpcode::AdvanceLine.byte());
            writer.sleb128(*delta);
        }
        DebugEvent::StartLocal {
            register,
            name,
            local_type,
        } => {
            writer.u8(DebugOpcode::StartLocal.byte());
            writer.uleb128(*register);
            writer.uleb128p1(name.map(StringIndex::get))?;
            writer.uleb128p1(local_type.map(TypeIndex::get))?;
        }
        DebugEvent::StartLocalExtended {
            register,
            name,
            local_type,
            signature,
        } => {
            writer.u8(DebugOpcode::StartLocalExtended.byte());
            writer.uleb128(*register);
            writer.uleb128p1(name.map(StringIndex::get))?;
            writer.uleb128p1(local_type.map(TypeIndex::get))?;
            writer.uleb128p1(signature.map(StringIndex::get))?;
        }
        DebugEvent::EndLocal(register) => {
            writer.u8(DebugOpcode::EndLocal.byte());
            writer.uleb128(*register);
        }
        DebugEvent::RestartLocal(register) => {
            writer.u8(DebugOpcode::RestartLocal.byte());
            writer.uleb128(*register);
        }
        DebugEvent::SetPrologueEnd => writer.u8(DebugOpcode::SetPrologueEnd.byte()),
        DebugEvent::SetEpilogueBegin => writer.u8(DebugOpcode::SetEpilogueBegin.byte()),
        DebugEvent::SetFile(file) => {
            writer.u8(DebugOpcode::SetFile.byte());
            writer.uleb128p1(file.map(StringIndex::get))?;
        }
        DebugEvent::Position {
            address_delta,
            line_delta,
        } => {
            let line_adjustment = line_delta
                .checked_sub(DEBUG_LINE_BASE)
                .ok_or_else(|| Error::invalid_assembly("special debug line delta underflowed"))?;
            let line_adjustment = u32::try_from(line_adjustment).map_err(|_| {
                Error::invalid_assembly("special debug line delta is below its typed range")
            })?;
            if line_adjustment >= DEBUG_LINE_RANGE {
                return Err(Error::invalid_assembly(
                    "special debug line delta exceeds its typed range",
                ));
            }
            let adjusted = address_delta
                .checked_mul(DEBUG_LINE_RANGE)
                .and_then(|value| value.checked_add(line_adjustment))
                .ok_or_else(|| Error::invalid_assembly("special debug opcode overflowed"))?;
            let adjusted = u8::try_from(adjusted)
                .map_err(|_| Error::invalid_assembly("special debug opcode exceeds one byte"))?;
            let opcode = FIRST_SPECIAL_DEBUG_OPCODE
                .checked_add(adjusted)
                .ok_or_else(|| Error::invalid_assembly("special debug opcode overflowed"))?;
            writer.u8(opcode);
        }
    }
    Ok(())
}

fn write_code_items(
    writer: &mut Writer,
    classes: &[ClassDefinition],
    debug_offsets: &BTreeMap<MethodIndex, u32>,
) -> Result<(BTreeMap<MethodIndex, u32>, Option<MapItem>)> {
    writer.align(Alignment::Word)?;
    let start = writer.position()?;
    let mut offsets = BTreeMap::new();
    let mut count = EMPTY_ITEM_COUNT;
    for method in methods(classes) {
        let Some(code) = &method.code else {
            continue;
        };
        writer.align(Alignment::Word)?;
        let offset = writer.position()?;
        let words = encode(&code.instructions)?;
        let tries_size = u16::try_from(code.tries.len()).map_err(|_| {
            Error::invalid_assembly(format!("code item has more than {} try blocks", u16::MAX))
        })?;
        writer.u16(code.registers_size);
        writer.u16(code.ins_size);
        writer.u16(code.outs_size);
        writer.u16(tries_size);
        writer.u32(
            debug_offsets
                .get(&method.method)
                .copied()
                .unwrap_or(ABSENT_OFFSET),
        );
        writer.u32(u32::try_from(words.len()).map_err(|_| {
            Error::invalid_assembly("instruction stream exceeds 32-bit code-unit space")
        })?);
        let body_offset = writer.position()?;
        let header_width = body_offset
            .checked_sub(offset)
            .ok_or_else(|| Error::invalid_assembly("code header position underflowed"))?;
        if header_width
            != u32::try_from(ItemWidth::CODE_HEADER.bytes())
                .map_err(|_| Error::invalid_assembly("code header width exceeds 32 bits"))?
        {
            return Err(Error::invalid_assembly(
                "code header did not occupy its typed width",
            ));
        }
        writer.align(Alignment::CodeUnit)?;
        for word in &words {
            writer.u16(*word);
        }
        if !code.tries.is_empty() {
            writer.align(Alignment::Word)?;
            write_tries(writer, &code.tries)?;
        }
        if offsets.insert(method.method, offset).is_some() {
            return Err(Error::invalid_assembly(
                "a method has more than one code item",
            ));
        }
        count = count
            .checked_add(ITEM_COUNT_INCREMENT)
            .ok_or_else(|| Error::invalid_assembly("code item count overflowed"))?;
    }
    Ok((offsets, section(MapItemType::Code, count, start)))
}

fn write_tries(writer: &mut Writer, tries: &[crate::file::TryBlock]) -> Result<()> {
    let table_offset = writer.reserve(
        tries
            .len()
            .checked_mul(ItemWidth::TRY_ITEM.bytes())
            .ok_or_else(|| Error::invalid_assembly("try table size overflowed"))?,
    )?;
    let handlers_offset = writer.position()?;
    writer.uleb128(u32::try_from(tries.len()).map_err(|_| {
        Error::invalid_assembly("exception handler count exceeds 32-bit address space")
    })?);
    for (index, try_block) in tries.iter().enumerate() {
        let item_delta = u32::try_from(index * ItemWidth::TRY_ITEM.bytes())
            .map_err(|_| Error::invalid_assembly("try item offset exceeds 32 bits"))?;
        let item_offset = table_offset
            .checked_add(item_delta)
            .ok_or_else(|| Error::invalid_assembly("try item offset overflowed"))?;
        writer.patch_u32(
            item_offset + TryField::StartAddress.offset_u32(),
            try_block.start_address,
        )?;
        writer.patch_u16(
            item_offset + TryField::InstructionCount.offset_u32(),
            try_block.instruction_count,
        )?;
        let handler_delta = writer
            .position()?
            .checked_sub(handlers_offset)
            .ok_or_else(|| Error::invalid_assembly("handler offset underflowed"))?;
        writer.patch_u16(
            item_offset + TryField::HandlerOffset.offset_u32(),
            u16::try_from(handler_delta)
                .map_err(|_| Error::invalid_assembly("exception handler offset exceeds 16 bits"))?,
        )?;
        write_catches(writer, &try_block.handlers)?;
    }
    Ok(())
}

fn write_catches(writer: &mut Writer, handlers: &[CatchHandler]) -> Result<()> {
    let catch_all = handlers
        .iter()
        .position(|handler| handler.exception_type.is_none());
    if catch_all.is_some()
        && handlers
            .last()
            .is_none_or(|handler| handler.exception_type.is_some())
    {
        return Err(Error::invalid_assembly(
            "catch-all handler is not last in its handler list",
        ));
    }
    let typed_count = handlers.len() - usize::from(catch_all.is_some());
    if handlers.is_empty() {
        return Err(Error::invalid_assembly("exception handler list is empty"));
    }
    let typed_count_usize = typed_count;
    let typed_count = i32::try_from(typed_count_usize)
        .map_err(|_| Error::invalid_assembly("typed exception handler count exceeds 32 bits"))?;
    writer.sleb128(EncodedCatchHandlerCount::from_parts(typed_count, catch_all.is_some()).raw());
    for handler in handlers.iter().take(typed_count_usize) {
        let exception_type = handler.exception_type.ok_or_else(|| {
            Error::invalid_assembly("typed exception handler has no exception type")
        })?;
        writer.uleb128(exception_type.get());
        writer.uleb128(handler.address);
    }
    if let Some(position) = catch_all {
        writer.uleb128(handlers[position].address);
    }
    Ok(())
}

fn write_class_data(
    writer: &mut Writer,
    classes: &[ClassDefinition],
    code_offsets: &BTreeMap<MethodIndex, u32>,
) -> Result<(Vec<u32>, Option<MapItem>)> {
    let start = writer.position()?;
    let mut offsets = Vec::with_capacity(classes.len());
    let mut count = EMPTY_ITEM_COUNT;
    for class in classes {
        let Some(data) = &class.class_data else {
            offsets.push(ABSENT_OFFSET);
            continue;
        };
        let offset = writer.position()?;
        writer.uleb128(count_u32(data.static_fields.len(), "static field")?);
        writer.uleb128(count_u32(data.instance_fields.len(), "instance field")?);
        writer.uleb128(count_u32(data.direct_methods.len(), "direct method")?);
        writer.uleb128(count_u32(data.virtual_methods.len(), "virtual method")?);
        write_fields(writer, &data.static_fields)?;
        write_fields(writer, &data.instance_fields)?;
        write_methods(writer, &data.direct_methods, code_offsets)?;
        write_methods(writer, &data.virtual_methods, code_offsets)?;
        offsets.push(offset);
        count = count
            .checked_add(ITEM_COUNT_INCREMENT)
            .ok_or_else(|| Error::invalid_assembly("class data item count overflowed"))?;
    }
    Ok((offsets, section(MapItemType::ClassData, count, start)))
}

fn write_fields(writer: &mut Writer, fields: &[crate::file::EncodedField]) -> Result<()> {
    let mut previous = None;
    for field in fields {
        let current = field.field.get();
        let difference = match previous {
            None => current,
            Some(previous) if current == previous => {
                return Err(Error::invalid_assembly("encoded field index is duplicated"));
            }
            Some(previous) => current.checked_sub(previous).ok_or_else(|| {
                Error::invalid_assembly("encoded fields are not in increasing index order")
            })?,
        };
        writer.uleb128(difference);
        writer.uleb128(field.access_flags.bits());
        previous = Some(current);
    }
    Ok(())
}

fn write_methods(
    writer: &mut Writer,
    methods: &[EncodedMethod],
    code_offsets: &BTreeMap<MethodIndex, u32>,
) -> Result<()> {
    let mut previous = None;
    for method in methods {
        let current = method.method.get();
        let difference = match previous {
            None => current,
            Some(previous) if current == previous => {
                return Err(Error::invalid_assembly(
                    "encoded method index is duplicated",
                ));
            }
            Some(previous) => current.checked_sub(previous).ok_or_else(|| {
                Error::invalid_assembly("encoded methods are not in increasing index order")
            })?,
        };
        writer.uleb128(difference);
        writer.uleb128(method.access_flags.bits());
        writer.uleb128(
            code_offsets
                .get(&method.method)
                .copied()
                .unwrap_or(ABSENT_OFFSET),
        );
        previous = Some(current);
    }
    Ok(())
}

fn methods(classes: &[ClassDefinition]) -> impl Iterator<Item = &EncodedMethod> {
    classes
        .iter()
        .filter_map(|class| class.class_data.as_ref())
        .flat_map(|data| data.direct_methods.iter().chain(&data.virtual_methods))
}

fn count_u32(count: usize, what: &str) -> Result<u32> {
    u32::try_from(count)
        .map_err(|_| Error::invalid_assembly(format!("{what} count exceeds 32-bit address space")))
}

fn section(item_type: MapItemType, count: u32, offset: u32) -> Option<MapItem> {
    (count != EMPTY_ITEM_COUNT).then_some(MapItem {
        item_type,
        size: count,
        offset,
    })
}

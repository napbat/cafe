//! Compact code-item decoding and encoding.

use std::collections::{BTreeMap, BTreeSet};

use crate::file::io::{Reader, Writer};
use crate::file::{CatchHandler, CodeItem, EncodedCatchHandlerCount, Endian, TryBlock, TypeIndex};
use crate::instruction::{Instruction, InstructionData, decode, encode};
use crate::{Error, Result};

const CODE_ITEM_ALIGNMENT: u32 = 2;
const TRY_ITEM_ALIGNMENT: u32 = 4;
const TRY_ITEM_WIDTH: usize = 8;
const COMPACT_HEADER_WIDTH: usize = 4;
const CODE_UNIT_WIDTH: usize = 2;
const FIELD_NIBBLE_MASK: u16 = 0x000f;
const REGISTERS_SHIFT: u32 = 12;
const INS_SHIFT: u32 = 8;
const OUTS_SHIFT: u32 = 4;
const TRIES_SHIFT: u32 = 0;
const FLAG_REGISTERS: u16 = 1 << 0;
const FLAG_INS: u16 = 1 << 1;
const FLAG_OUTS: u16 = 1 << 2;
const FLAG_TRIES: u16 = 1 << 3;
const FLAG_INSTRUCTIONS: u16 = 1 << 4;
const INSTRUCTION_COUNT_SHIFT: u32 = 5;
const INLINE_INSTRUCTION_BITS: u32 = 11;
const INLINE_INSTRUCTION_MASK: u32 = (1 << INLINE_INSTRUCTION_BITS) - 1;
const MAX_PREHEADER_WORDS: usize = 6;
const PADDING_VALUE: u8 = 0;

/// Canonical code item decoded from `CompactDex`, plus its physical preheader width.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompactCodeItem {
    canonical: CodeItem,
    preheader_code_units: u8,
}

impl CompactCodeItem {
    /// Returns the canonical DEX code-item model.
    #[must_use]
    pub const fn canonical(&self) -> &CodeItem {
        &self.canonical
    }

    /// Consumes this wrapper and returns canonical DEX semantics.
    #[must_use]
    pub fn into_canonical(self) -> CodeItem {
        self.canonical
    }

    /// Returns the number of 16-bit words immediately before `data_offset`
    /// used by the `CompactDex` preheader.
    #[must_use]
    pub const fn preheader_code_units(&self) -> u8 {
        self.preheader_code_units
    }
}

/// Encoded `CompactDex` code item and its coordinates in a target data section.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EncodedCompactCodeItem {
    /// Offset at which `bytes` must be placed.
    pub start_offset: u32,
    /// Offset consumers encode in `class_data_item` after the preheader.
    pub item_offset: u32,
    /// Preheader, compact fields, instruction words, and exception data.
    pub bytes: Vec<u8>,
}

/// Decodes one `CompactDex` code item from a shared data section.
///
/// `item_offset` points at the two compact fields, after any optional
/// preheader. Debug information lives in `CompactDex`'s per-method offset table
/// and is intentionally not part of this physical code item.
///
/// # Errors
///
/// Returns an error for truncated preheaders, invalid frame sizes, malformed
/// instructions, misaligned try data, invalid handlers, or out-of-range types.
pub fn decode_code_item(
    data: &[u8],
    item_offset: u32,
    endian: Endian,
    type_ids_size: u32,
) -> Result<CompactCodeItem> {
    decode_code_item_with_base(data, item_offset, 0, endian, type_ids_size)
}

fn decode_code_item_with_base(
    data: &[u8],
    item_offset: u32,
    data_base: u32,
    endian: Endian,
    type_ids_size: u32,
) -> Result<CompactCodeItem> {
    let absolute_item_offset = data_base
        .checked_add(item_offset)
        .ok_or_else(|| Error::invalid_dex(item_offset as usize, "code offset overflowed"))?;
    if !absolute_item_offset.is_multiple_of(CODE_ITEM_ALIGNMENT) {
        return Err(Error::invalid_dex(
            absolute_item_offset as usize,
            "CompactDex code item is not code-unit aligned",
        ));
    }
    let offset = usize::try_from(item_offset)
        .map_err(|_| Error::invalid_dex(0, "CompactDex code offset is too large"))?;
    let reader = Reader::new(data, endian);
    let fields = reader.u16(offset)?;
    let count_and_flags = reader.u16(offset + CODE_UNIT_WIDTH)?;
    let mut registers_delta = (fields >> REGISTERS_SHIFT) & FIELD_NIBBLE_MASK;
    let mut ins_size = (fields >> INS_SHIFT) & FIELD_NIBBLE_MASK;
    let mut outs_size = (fields >> OUTS_SHIFT) & FIELD_NIBBLE_MASK;
    let mut tries_size = (fields >> TRIES_SHIFT) & FIELD_NIBBLE_MASK;
    let mut instruction_count = u32::from(count_and_flags >> INSTRUCTION_COUNT_SHIFT);
    let mut preheader = offset;
    let mut preheader_words = 0usize;

    if count_and_flags & FLAG_INSTRUCTIONS != 0 {
        let low = previous_word(reader, &mut preheader, &mut preheader_words)?;
        let high = previous_word(reader, &mut preheader, &mut preheader_words)?;
        instruction_count = instruction_count
            .checked_add(u32::from(low))
            .and_then(|value| value.checked_add(u32::from(high) << u16::BITS))
            .ok_or_else(|| Error::invalid_dex(offset, "CompactDex instruction count overflowed"))?;
    }
    if count_and_flags & FLAG_REGISTERS != 0 {
        registers_delta = registers_delta
            .checked_add(previous_word(reader, &mut preheader, &mut preheader_words)?)
            .ok_or_else(|| Error::invalid_dex(offset, "CompactDex register count overflowed"))?;
    }
    if count_and_flags & FLAG_INS != 0 {
        ins_size = ins_size
            .checked_add(previous_word(reader, &mut preheader, &mut preheader_words)?)
            .ok_or_else(|| Error::invalid_dex(offset, "CompactDex input count overflowed"))?;
    }
    if count_and_flags & FLAG_OUTS != 0 {
        outs_size = outs_size
            .checked_add(previous_word(reader, &mut preheader, &mut preheader_words)?)
            .ok_or_else(|| Error::invalid_dex(offset, "CompactDex output count overflowed"))?;
    }
    if count_and_flags & FLAG_TRIES != 0 {
        tries_size = tries_size
            .checked_add(previous_word(reader, &mut preheader, &mut preheader_words)?)
            .ok_or_else(|| Error::invalid_dex(offset, "CompactDex try count overflowed"))?;
    }
    if preheader_words > MAX_PREHEADER_WORDS {
        return Err(Error::invalid_dex(
            offset,
            "CompactDex preheader is too large",
        ));
    }
    let registers_size = registers_delta
        .checked_add(ins_size)
        .ok_or_else(|| Error::invalid_dex(offset, "CompactDex frame size overflowed"))?;
    let instructions_offset = offset
        .checked_add(COMPACT_HEADER_WIDTH)
        .ok_or_else(|| Error::invalid_dex(offset, "CompactDex instruction offset overflowed"))?;
    let count = usize::try_from(instruction_count)
        .map_err(|_| Error::invalid_dex(offset, "instruction count is too large"))?;
    let mut words = Vec::with_capacity(count);
    for index in 0..count {
        words.push(reader.u16(instructions_offset + index * CODE_UNIT_WIDTH)?);
    }
    let instructions = decode(&words)?;
    let tries = if tries_size == 0 {
        Vec::new()
    } else {
        parse_tries(
            reader,
            offset,
            data_base,
            instruction_count,
            tries_size,
            type_ids_size,
            &instructions,
        )?
    };
    let canonical = CodeItem {
        registers_size,
        ins_size,
        outs_size,
        instructions,
        tries,
        debug_info: None,
        data_offset: absolute_item_offset,
    };
    crate::analysis::analyze_body(&canonical)?;
    Ok(CompactCodeItem {
        canonical,
        preheader_code_units: u8::try_from(preheader_words)
            .map_err(|_| Error::invalid_dex(offset, "CompactDex preheader width is too large"))?,
    })
}

/// Encodes a canonical code item at data-section offset zero.
///
/// # Errors
///
/// Returns an error when instructions, frame sizes, or exception metadata
/// cannot be represented by `CompactDex` 001.
pub fn encode_code_item(code: &CodeItem) -> Result<EncodedCompactCodeItem> {
    encode_code_item_at(code, 0)
}

/// Encodes a canonical code item for placement at an explicit data-section
/// start offset. The offset is needed because ART aligns try tables and payload
/// instructions in the complete data address space.
///
/// # Errors
///
/// Returns an error when the start is unaligned or any code-item value cannot
/// be represented.
pub fn encode_code_item_at(code: &CodeItem, start_offset: u32) -> Result<EncodedCompactCodeItem> {
    if !start_offset.is_multiple_of(CODE_ITEM_ALIGNMENT) {
        return Err(Error::invalid_assembly(
            "CompactDex code-item storage is not code-unit aligned",
        ));
    }
    if code.registers_size < code.ins_size {
        return Err(Error::invalid_assembly(
            "CompactDex input registers exceed the frame size",
        ));
    }
    let words = encode(&code.instructions)?;
    let tries_size = u16::try_from(code.tries.len())
        .map_err(|_| Error::invalid_assembly("CompactDex try count exceeds 16 bits"))?;
    let (preheader, fields, count_and_flags) = encode_fields(
        code.registers_size,
        code.ins_size,
        code.outs_size,
        tries_size,
        u32::try_from(words.len())
            .map_err(|_| Error::invalid_assembly("instruction count exceeds 32 bits"))?,
    );
    let mut writer = Writer::new_at(Endian::Little, start_offset);
    for word in &preheader {
        writer.u16(*word);
    }
    let item_offset = writer.position()?;
    writer.u16(fields);
    writer.u16(count_and_flags);
    for word in &words {
        writer.u16(*word);
    }
    if !code.tries.is_empty() {
        align_writer(&mut writer, TRY_ITEM_ALIGNMENT)?;
        write_tries(&mut writer, &code.tries, words.len())?;
    }
    let encoded = EncodedCompactCodeItem {
        start_offset,
        item_offset,
        bytes: writer.into_bytes(),
    };
    let relative_item = item_offset
        .checked_sub(start_offset)
        .ok_or_else(|| Error::invalid_assembly("CompactDex item offset underflowed"))?;
    let decoded = decode_code_item_with_base(
        &encoded.bytes,
        relative_item,
        start_offset,
        Endian::Little,
        u32::from(u16::MAX) + 1,
    )?;
    if decoded.canonical.registers_size != code.registers_size
        || decoded.canonical.ins_size != code.ins_size
        || decoded.canonical.outs_size != code.outs_size
        || decoded.canonical.instructions != code.instructions
        || decoded.canonical.tries != code.tries
    {
        return Err(Error::invalid_assembly(
            "CompactDex code-item self-validation changed canonical semantics",
        ));
    }
    Ok(encoded)
}

fn previous_word(reader: Reader<'_>, offset: &mut usize, count: &mut usize) -> Result<u16> {
    *offset = offset
        .checked_sub(CODE_UNIT_WIDTH)
        .ok_or_else(|| Error::invalid_dex(*offset, "CompactDex preheader underflowed"))?;
    *count += 1;
    reader.u16(*offset)
}

fn encode_fields(
    registers_size: u16,
    ins_size: u16,
    outs_size: u16,
    tries_size: u16,
    instruction_count: u32,
) -> (Vec<u16>, u16, u16) {
    let registers_delta = registers_size - ins_size;
    let fields = ((registers_delta & FIELD_NIBBLE_MASK) << REGISTERS_SHIFT)
        | ((ins_size & FIELD_NIBBLE_MASK) << INS_SHIFT)
        | ((outs_size & FIELD_NIBBLE_MASK) << OUTS_SHIFT)
        | ((tries_size & FIELD_NIBBLE_MASK) << TRIES_SHIFT);
    let registers_extra = registers_delta & !FIELD_NIBBLE_MASK;
    let ins_extra = ins_size & !FIELD_NIBBLE_MASK;
    let outs_extra = outs_size & !FIELD_NIBBLE_MASK;
    let tries_extra = tries_size & !FIELD_NIBBLE_MASK;
    let inline_count = instruction_count & INLINE_INSTRUCTION_MASK;
    let instruction_extra = instruction_count - inline_count;
    let mut flags = u16::try_from(inline_count << INSTRUCTION_COUNT_SHIFT)
        .expect("11-bit count shifted by five always fits u16");
    let mut preheader = Vec::new();
    if tries_extra != 0 {
        flags |= FLAG_TRIES;
        preheader.push(tries_extra);
    }
    if outs_extra != 0 {
        flags |= FLAG_OUTS;
        preheader.push(outs_extra);
    }
    if ins_extra != 0 {
        flags |= FLAG_INS;
        preheader.push(ins_extra);
    }
    if registers_extra != 0 {
        flags |= FLAG_REGISTERS;
        preheader.push(registers_extra);
    }
    if instruction_extra != 0 {
        flags |= FLAG_INSTRUCTIONS;
        preheader.push(
            u16::try_from(instruction_extra >> u16::BITS)
                .expect("shifted instruction count fits 16 bits"),
        );
        preheader.push(
            u16::try_from(instruction_extra & u32::from(u16::MAX))
                .expect("masked instruction count fits 16 bits"),
        );
    }
    (preheader, fields, flags)
}

fn parse_tries(
    reader: Reader<'_>,
    code_offset: usize,
    data_base: u32,
    instruction_count: u32,
    tries_size: u16,
    type_ids_size: u32,
    instructions: &[Instruction],
) -> Result<Vec<TryBlock>> {
    let instruction_bytes = usize::try_from(instruction_count)
        .ok()
        .and_then(|count| count.checked_mul(CODE_UNIT_WIDTH))
        .ok_or_else(|| Error::invalid_dex(code_offset, "instruction byte size overflowed"))?;
    let instruction_end = code_offset
        .checked_add(COMPACT_HEADER_WIDTH + instruction_bytes)
        .ok_or_else(|| Error::invalid_dex(code_offset, "try offset overflowed"))?;
    let absolute_end = usize::try_from(data_base)
        .ok()
        .and_then(|base| base.checked_add(instruction_end))
        .ok_or_else(|| Error::invalid_dex(instruction_end, "absolute try offset overflowed"))?;
    let absolute_tries = align_up(absolute_end, TRY_ITEM_ALIGNMENT as usize)?;
    let tries_offset = absolute_tries
        .checked_sub(data_base as usize)
        .ok_or_else(|| Error::invalid_dex(instruction_end, "try offset underflowed"))?;
    for padding in instruction_end..tries_offset {
        if reader.u8(padding)? != 0 {
            return Err(Error::invalid_dex(
                padding,
                "CompactDex try alignment padding is nonzero",
            ));
        }
    }
    let tries_count = usize::from(tries_size);
    let try_bytes = tries_count
        .checked_mul(TRY_ITEM_WIDTH)
        .ok_or_else(|| Error::invalid_dex(tries_offset, "try table size overflowed"))?;
    reader.bytes(tries_offset, try_bytes)?;
    let handlers_offset = tries_offset
        .checked_add(try_bytes)
        .ok_or_else(|| Error::invalid_dex(tries_offset, "handler offset overflowed"))?;
    let handlers = parse_handlers(
        reader,
        handlers_offset,
        instruction_count,
        type_ids_size,
        instructions,
    )?;
    let boundaries = operation_boundaries(instructions, instruction_count);
    let mut output = Vec::with_capacity(tries_count);
    let mut previous_end = None;
    for index in 0..tries_count {
        let item = tries_offset + index * TRY_ITEM_WIDTH;
        let start_address = reader.u32(item)?;
        let count = reader.u16(item + 4)?;
        let handler_offset = u32::from(reader.u16(item + 6)?);
        let end = start_address
            .checked_add(u32::from(count))
            .ok_or_else(|| Error::invalid_dex(item, "protected range overflowed"))?;
        if count == 0 || end > instruction_count {
            return Err(Error::invalid_dex(
                item,
                "protected range is empty or outside the method",
            ));
        }
        if !boundaries.contains(&start_address) || !boundaries.contains(&end) {
            return Err(Error::invalid_dex(
                item,
                "protected range does not use instruction boundaries",
            ));
        }
        if previous_end.is_some_and(|previous_end| start_address < previous_end) {
            return Err(Error::invalid_dex(item, "protected ranges overlap"));
        }
        previous_end = Some(end);
        let catches = handlers.get(&handler_offset).cloned().ok_or_else(|| {
            Error::invalid_dex(item + 6, "handler offset is not a handler-list boundary")
        })?;
        output.push(TryBlock {
            start_address,
            instruction_count: count,
            handlers: catches,
        });
    }
    Ok(output)
}

fn parse_handlers(
    reader: Reader<'_>,
    offset: usize,
    instruction_count: u32,
    type_ids_size: u32,
    instructions: &[Instruction],
) -> Result<BTreeMap<u32, Vec<CatchHandler>>> {
    let mut cursor = reader.cursor(offset)?;
    let count = cursor.uleb128()?;
    let operation_offsets: BTreeSet<_> = instructions
        .iter()
        .filter(|instruction| matches!(instruction.data(), InstructionData::Operation { .. }))
        .map(Instruction::offset)
        .collect();
    let mut output = BTreeMap::new();
    for _ in 0..count {
        let relative = u32::try_from(cursor.position().saturating_sub(offset))
            .map_err(|_| Error::invalid_dex(offset, "handler offset exceeds 32 bits"))?;
        let encoded_count = EncodedCatchHandlerCount::from_raw(cursor.sleb128()?);
        let typed_count = encoded_count
            .typed_count()
            .ok_or_else(|| Error::invalid_dex(cursor.position(), "handler count overflowed"))?;
        let mut handlers = Vec::new();
        let mut seen_types = BTreeSet::new();
        for _ in 0..typed_count {
            let entry = cursor.position();
            let exception_type = cursor.uleb128()?;
            if exception_type >= type_ids_size {
                return Err(Error::invalid_dex(
                    entry,
                    "exception type index is out of bounds",
                ));
            }
            if !seen_types.insert(exception_type) {
                return Err(Error::invalid_dex(
                    entry,
                    "duplicate exception type in handler",
                ));
            }
            let address = cursor.uleb128()?;
            require_handler_target(address, instruction_count, &operation_offsets, entry)?;
            handlers.push(CatchHandler {
                exception_type: Some(TypeIndex::new(exception_type)),
                address,
            });
        }
        if encoded_count.has_catch_all() {
            let entry = cursor.position();
            let address = cursor.uleb128()?;
            require_handler_target(address, instruction_count, &operation_offsets, entry)?;
            handlers.push(CatchHandler {
                exception_type: None,
                address,
            });
        }
        if handlers.is_empty() {
            return Err(Error::invalid_dex(offset, "empty exception handler list"));
        }
        if output.insert(relative, handlers).is_some() {
            return Err(Error::invalid_dex(offset, "duplicate handler-list offset"));
        }
    }
    Ok(output)
}

fn write_tries(writer: &mut Writer, tries: &[TryBlock], instruction_count: usize) -> Result<()> {
    validate_try_blocks(tries, instruction_count)?;
    let table_offset = writer.reserve(
        tries
            .len()
            .checked_mul(TRY_ITEM_WIDTH)
            .ok_or_else(|| Error::invalid_assembly("CompactDex try table size overflowed"))?,
    )?;
    let handlers_offset = writer.position()?;
    writer.uleb128(
        u32::try_from(tries.len())
            .map_err(|_| Error::invalid_assembly("handler-list count exceeds 32 bits"))?,
    );
    for (index, protected) in tries.iter().enumerate() {
        let item = table_offset
            .checked_add(
                u32::try_from(index * TRY_ITEM_WIDTH)
                    .map_err(|_| Error::invalid_assembly("try item offset exceeds 32 bits"))?,
            )
            .ok_or_else(|| Error::invalid_assembly("try item offset overflowed"))?;
        writer.patch_u32(item, protected.start_address)?;
        writer.patch_u16(item + 4, protected.instruction_count)?;
        let relative = writer
            .position()?
            .checked_sub(handlers_offset)
            .ok_or_else(|| Error::invalid_assembly("handler offset underflowed"))?;
        writer.patch_u16(
            item + 6,
            u16::try_from(relative)
                .map_err(|_| Error::invalid_assembly("handler offset exceeds 16 bits"))?,
        )?;
        write_handlers(writer, &protected.handlers)?;
    }
    Ok(())
}

fn write_handlers(writer: &mut Writer, handlers: &[CatchHandler]) -> Result<()> {
    if handlers.is_empty() {
        return Err(Error::invalid_assembly("exception handler list is empty"));
    }
    let catch_all = handlers
        .iter()
        .position(|handler| handler.exception_type.is_none());
    if catch_all.is_some() && catch_all != Some(handlers.len() - 1) {
        return Err(Error::invalid_assembly("catch-all handler is not last"));
    }
    let typed_count = handlers.len() - usize::from(catch_all.is_some());
    let typed_count_i32 = i32::try_from(typed_count)
        .map_err(|_| Error::invalid_assembly("typed handler count exceeds 32 bits"))?;
    writer
        .sleb128(EncodedCatchHandlerCount::from_parts(typed_count_i32, catch_all.is_some()).raw());
    let mut seen = BTreeSet::new();
    for handler in handlers.iter().take(typed_count) {
        let exception_type = handler.exception_type.ok_or_else(|| {
            Error::invalid_assembly("typed exception handler has no exception type")
        })?;
        if !seen.insert(exception_type.get()) {
            return Err(Error::invalid_assembly("duplicate typed exception handler"));
        }
        writer.uleb128(exception_type.get());
        writer.uleb128(handler.address);
    }
    if let Some(index) = catch_all {
        writer.uleb128(handlers[index].address);
    }
    Ok(())
}

fn validate_try_blocks(tries: &[TryBlock], instruction_count: usize) -> Result<()> {
    let instruction_count = u32::try_from(instruction_count)
        .map_err(|_| Error::invalid_assembly("instruction count exceeds 32 bits"))?;
    let mut previous_end = None;
    for protected in tries {
        let end = protected
            .start_address
            .checked_add(u32::from(protected.instruction_count))
            .ok_or_else(|| Error::invalid_assembly("protected range overflowed"))?;
        if protected.instruction_count == 0 || end > instruction_count {
            return Err(Error::invalid_assembly(
                "protected range is empty or outside the instruction stream",
            ));
        }
        if previous_end.is_some_and(|previous_end| protected.start_address < previous_end) {
            return Err(Error::invalid_assembly("protected ranges overlap"));
        }
        previous_end = Some(end);
    }
    Ok(())
}

fn operation_boundaries(instructions: &[Instruction], instruction_count: u32) -> BTreeSet<u32> {
    let mut boundaries: BTreeSet<_> = instructions
        .iter()
        .filter(|instruction| matches!(instruction.data(), InstructionData::Operation { .. }))
        .map(Instruction::offset)
        .collect();
    boundaries.insert(instruction_count);
    boundaries
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
            format!("handler target {address} is not an instruction boundary"),
        ))
    }
}

fn align_writer(writer: &mut Writer, alignment: u32) -> Result<()> {
    while !writer.position()?.is_multiple_of(alignment) {
        writer.u8(PADDING_VALUE);
    }
    Ok(())
}

fn align_up(value: usize, alignment: usize) -> Result<usize> {
    let remainder = value % alignment;
    if remainder == 0 {
        Ok(value)
    } else {
        value
            .checked_add(alignment - remainder)
            .ok_or_else(|| Error::invalid_dex(value, "alignment overflowed"))
    }
}

#[cfg(test)]
mod tests {
    use super::{decode_code_item, encode_code_item, encode_code_item_at};
    use crate::file::{CatchHandler, CodeItem, Endian, TryBlock, TypeIndex};
    use crate::instruction::{Instruction, Opcode, Operands};

    fn sample(registers_size: u16, instruction_count: usize) -> CodeItem {
        let mut instructions = Vec::with_capacity(instruction_count);
        for offset in 0..instruction_count.saturating_sub(1) {
            instructions.push(Instruction::operation(
                u32::try_from(offset).expect("test instruction offset fits 32 bits"),
                Opcode::Nop,
                Operands::None,
            ));
        }
        instructions.push(Instruction::operation(
            u32::try_from(instruction_count.saturating_sub(1))
                .expect("test instruction count fits 32 bits"),
            Opcode::ReturnVoid,
            Operands::None,
        ));
        CodeItem {
            registers_size,
            ins_size: 16,
            outs_size: 17,
            instructions,
            tries: Vec::new(),
            debug_info: None,
            data_offset: 0,
        }
    }

    #[test]
    fn round_trips_inline_and_preheader_fields() {
        let code = sample(48, 2_049);
        let encoded = encode_code_item(&code).unwrap();
        let decoded =
            decode_code_item(&encoded.bytes, encoded.item_offset, Endian::Little, 1).unwrap();
        assert_eq!(decoded.canonical().registers_size, 48);
        assert_eq!(decoded.canonical().instructions, code.instructions);
        assert_eq!(decoded.preheader_code_units(), 5);
    }

    #[test]
    fn aligns_and_round_trips_exception_data_at_nonzero_offset() {
        let mut code = sample(16, 2);
        code.tries.push(TryBlock {
            start_address: 0,
            instruction_count: 1,
            handlers: vec![CatchHandler {
                exception_type: Some(TypeIndex::new(0)),
                address: 1,
            }],
        });
        let encoded = encode_code_item_at(&code, 2).unwrap();
        let mut data = vec![0; encoded.start_offset as usize];
        data.extend_from_slice(&encoded.bytes);
        let decoded = decode_code_item(&data, encoded.item_offset, Endian::Little, 1).unwrap();
        assert_eq!(decoded.canonical().tries, code.tries);
    }
}

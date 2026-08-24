//! Canonical standard-DEX restoration from VDEX quickening metadata.

use std::collections::BTreeSet;

use dex::file::compact::CompactOffsetTable;
use dex::file::{DexSourceFormat, Endian};

use super::{VdexFile, VdexSection, VdexVersion};
use crate::binary::{u32_at, update_standard_dex_integrity};
use crate::quickening::{QuickeningInfo, dequicken_code_units};
use crate::{Error, Result};

const DEX_ENDIAN_TAG_OFFSET: usize = 40;
const DEX_METHOD_COUNT_OFFSET: usize = 88;
const DEX_CLASS_COUNT_OFFSET: usize = 96;
const DEX_CLASS_OFFSET_OFFSET: usize = 100;
const CLASS_DEFINITION_SIZE: usize = 32;
const CLASS_DATA_OFFSET_IN_DEFINITION: usize = 24;
const CODE_ITEM_HEADER_SIZE: usize = 16;
const CODE_INSTRUCTION_COUNT_OFFSET: usize = 12;
const DEX_LITTLE_ENDIAN_TAG: u32 = 0x1234_5678;
const DEX_REVERSE_ENDIAN_TAG: u32 = 0x7856_3412;
const LEGACY_QUICKENING_PAIR_SIZE: usize = 8;
const LEGACY_QUICKENING_WORD_SIZE: usize = 4;
const LEGACY_QUICKENING_LENGTH_SIZE: usize = 4;

#[derive(Debug, Clone, Copy)]
struct MethodCode {
    method_index: u32,
    code_offset: u32,
}

pub(super) fn standard_dex(file: &VdexFile, index: u32) -> Result<dex::DexFile> {
    let member = file.member(index)?;
    if !matches!(member.source_format, DexSourceFormat::Standard(_)) {
        return Err(Error::MissingCanonicalizationMetadata {
            artifact: format!("VDEX member {index}"),
            message: "CompactDex must retain its split main/data representation".to_owned(),
        });
    }
    let mut bytes = file
        .member_bytes(index)
        .ok_or_else(|| Error::invalid("VDEX", member.main_range.start, "member bytes disappeared"))?
        .to_vec();
    let endian = dex_endian(&bytes)?;
    if endian == Endian::Reverse && has_quickening(file, member) {
        return Err(Error::MissingCanonicalizationMetadata {
            artifact: format!("VDEX member {index}"),
            message: "ART quickening tables are defined for little-endian runtime DEX".to_owned(),
        });
    }

    let methods = method_codes(&bytes, endian)?;
    let mut seen_offsets = BTreeSet::new();
    for method in methods {
        if !seen_offsets.insert(method.code_offset) {
            continue;
        }
        let range = instruction_range(&bytes, method.code_offset, endian)?;
        let words = read_words(&bytes[range.clone()], endian);
        let info = quickening_for_code(file, index, method, &words)?;
        let canonical = dequicken_code_units(&words, &info)?;
        write_words(&mut bytes[range], canonical.words(), endian);
    }
    update_standard_dex_integrity(&mut bytes)?;
    Ok(dex::DexFile::parse(&bytes)?)
}

pub(super) fn quickening_info(
    file: &VdexFile,
    dex_index: u32,
    method_index: u32,
) -> Result<QuickeningInfo> {
    if file.version() == VdexVersion::V009 {
        return Err(Error::MissingCanonicalizationMetadata {
            artifact: format!("VDEX 009 member {dex_index} method {method_index}"),
            message:
                "version 009 indexes quickening by code-item offset; use canonical_standard_dex"
                    .to_owned(),
        });
    }
    compact_quickening_info(file, dex_index, method_index)
}

fn quickening_for_code(
    file: &VdexFile,
    dex_index: u32,
    method: MethodCode,
    words: &[u16],
) -> Result<QuickeningInfo> {
    if file.version() != VdexVersion::V009 {
        return compact_quickening_info(file, dex_index, method.method_index);
    }
    let quickening = file.section(VdexSection::Quickening).unwrap_or_default();
    if quickening.is_empty() {
        return Ok(QuickeningInfo::default());
    }
    let dex_count = file.members().len();
    let footer_bytes = dex_count
        .checked_mul(LEGACY_QUICKENING_WORD_SIZE)
        .ok_or_else(|| Error::quickening(0, "legacy VDEX footer size overflowed"))?;
    let footer_start = quickening
        .len()
        .checked_sub(footer_bytes)
        .ok_or_else(|| Error::quickening(0, "truncated legacy VDEX quickening footer"))?;
    let dex_position = usize::try_from(dex_index)
        .map_err(|_| Error::quickening(footer_start, "DEX index is too large"))?;
    let table_start = legacy_table_boundary(quickening, footer_start, dex_position)?;
    let table_end = if dex_position + 1 < dex_count {
        legacy_table_boundary(quickening, footer_start, dex_position + 1)?
    } else {
        footer_start
    };
    if table_start > table_end
        || table_end > footer_start
        || !(table_end - table_start).is_multiple_of(LEGACY_QUICKENING_PAIR_SIZE)
    {
        return Err(Error::quickening(
            table_start,
            "invalid legacy code-item quickening table range",
        ));
    }
    let mut info_offset = None;
    for pair in quickening[table_start..table_end]
        .as_chunks::<LEGACY_QUICKENING_PAIR_SIZE>()
        .0
    {
        let code_offset = u32::from_le_bytes([pair[0], pair[1], pair[2], pair[3]]);
        let offset = u32::from_le_bytes([pair[4], pair[5], pair[6], pair[7]]);
        if code_offset == method.code_offset {
            info_offset = Some(offset);
            break;
        }
    }
    let Some(info_offset) = info_offset else {
        return Ok(QuickeningInfo::default());
    };
    let info_offset = usize::try_from(info_offset)
        .map_err(|_| Error::quickening(0, "legacy quickening offset is too large"))?;
    let record_length = usize::try_from(u32_at(quickening, info_offset, "VDEX quickening")?)
        .map_err(|_| Error::quickening(info_offset, "legacy record length is too large"))?;
    let record_start = info_offset
        .checked_add(LEGACY_QUICKENING_LENGTH_SIZE)
        .ok_or_else(|| Error::quickening(info_offset, "legacy record offset overflowed"))?;
    let record_end = record_start
        .checked_add(record_length)
        .ok_or_else(|| Error::quickening(record_start, "legacy record range overflowed"))?;
    let record = quickening
        .get(record_start..record_end)
        .ok_or_else(|| Error::quickening(record_start, "truncated legacy quickening record"))?;
    if record_end > table_start {
        return Err(Error::quickening(
            info_offset,
            "legacy quickening record overlaps its lookup table",
        ));
    }
    QuickeningInfo::parse_legacy_for_code(record, words)
}

fn compact_quickening_info(
    file: &VdexFile,
    dex_index: u32,
    method_index: u32,
) -> Result<QuickeningInfo> {
    let member = file.member(dex_index)?;
    let quickening = file.section(VdexSection::Quickening).unwrap_or_default();
    if quickening.is_empty() || file.version() == VdexVersion::V027 {
        return Ok(QuickeningInfo::default());
    }
    let table_offset = member.quickening_table_offset.ok_or_else(|| {
        Error::quickening(
            member.main_range.start,
            "member has no compact quickening-table offset",
        )
    })?;
    let table_offset = usize::try_from(table_offset)
        .map_err(|_| Error::quickening(0, "quickening table offset is too large"))?;
    let table_bytes = quickening
        .get(table_offset..)
        .ok_or_else(|| Error::quickening(table_offset, "quickening table is out of bounds"))?;
    let main = file.member_bytes(dex_index).ok_or_else(|| {
        Error::invalid("VDEX", member.main_range.start, "member bytes disappeared")
    })?;
    let method_count = usize::try_from(read_u32(main, DEX_METHOD_COUNT_OFFSET, Endian::Little)?)
        .map_err(|_| Error::quickening(table_offset, "method count is too large"))?;
    if usize::try_from(method_index).map_or(true, |index| index >= method_count) {
        return Err(Error::quickening(
            table_offset,
            format!("method index {method_index} exceeds method table size {method_count}"),
        ));
    }
    let table = CompactOffsetTable::parse_embedded(table_bytes, method_count, Endian::Little)?;
    let record_offset = table.get(method_index).ok_or_else(|| {
        Error::quickening(
            table_offset,
            format!("method index {method_index} is absent"),
        )
    })?;
    if record_offset == 0 {
        return Ok(QuickeningInfo::default());
    }
    let record_offset = usize::try_from(record_offset)
        .map_err(|_| Error::quickening(0, "quickening record offset is too large"))?;
    let record = quickening
        .get(record_offset..)
        .ok_or_else(|| Error::quickening(record_offset, "quickening record is out of bounds"))?;
    QuickeningInfo::parse_prefix(record).map(|(info, _)| info)
}

fn has_quickening(file: &VdexFile, member: &super::VdexDexMember) -> bool {
    file.section(VdexSection::Quickening)
        .is_some_and(|bytes| !bytes.is_empty())
        || member
            .quickening_table_offset
            .is_some_and(|offset| offset != 0)
}

fn legacy_table_boundary(bytes: &[u8], footer_start: usize, index: usize) -> Result<usize> {
    let offset = footer_start
        .checked_add(index.checked_mul(4).ok_or_else(|| {
            Error::quickening(footer_start, "legacy footer coordinate overflowed")
        })?)
        .ok_or_else(|| Error::quickening(footer_start, "legacy footer coordinate overflowed"))?;
    usize::try_from(u32_at(bytes, offset, "VDEX quickening")?)
        .map_err(|_| Error::quickening(offset, "legacy table offset is too large"))
}

fn method_codes(bytes: &[u8], endian: Endian) -> Result<Vec<MethodCode>> {
    let class_count = usize::try_from(read_u32(bytes, DEX_CLASS_COUNT_OFFSET, endian)?)
        .map_err(|_| Error::invalid("DEX", DEX_CLASS_COUNT_OFFSET, "class count is too large"))?;
    let class_offset = usize::try_from(read_u32(bytes, DEX_CLASS_OFFSET_OFFSET, endian)?)
        .map_err(|_| Error::invalid("DEX", DEX_CLASS_OFFSET_OFFSET, "class offset is too large"))?;
    let class_bytes = class_count
        .checked_mul(CLASS_DEFINITION_SIZE)
        .ok_or_else(|| Error::invalid("DEX", class_offset, "class table size overflowed"))?;
    require_range(bytes, class_offset, class_bytes, "class definitions")?;
    let mut output = Vec::new();
    for index in 0..class_count {
        let definition = class_offset + index * CLASS_DEFINITION_SIZE;
        let data_offset = read_u32(bytes, definition + CLASS_DATA_OFFSET_IN_DEFINITION, endian)?;
        if data_offset != 0 {
            parse_class_data(bytes, data_offset, &mut output)?;
        }
    }
    Ok(output)
}

fn parse_class_data(bytes: &[u8], data_offset: u32, output: &mut Vec<MethodCode>) -> Result<()> {
    let mut cursor = usize::try_from(data_offset)
        .map_err(|_| Error::invalid("DEX", 0, "class-data offset is too large"))?;
    let static_fields = read_uleb(bytes, &mut cursor)?;
    let instance_fields = read_uleb(bytes, &mut cursor)?;
    let direct_methods = read_uleb(bytes, &mut cursor)?;
    let virtual_methods = read_uleb(bytes, &mut cursor)?;
    skip_fields(bytes, &mut cursor, static_fields)?;
    skip_fields(bytes, &mut cursor, instance_fields)?;
    parse_methods(bytes, &mut cursor, direct_methods, output)?;
    parse_methods(bytes, &mut cursor, virtual_methods, output)
}

fn skip_fields(bytes: &[u8], cursor: &mut usize, count: u32) -> Result<()> {
    for _ in 0..count {
        let _ = read_uleb(bytes, cursor)?;
        let _ = read_uleb(bytes, cursor)?;
    }
    Ok(())
}

fn parse_methods(
    bytes: &[u8],
    cursor: &mut usize,
    count: u32,
    output: &mut Vec<MethodCode>,
) -> Result<()> {
    let mut method_index = 0u32;
    for _ in 0..count {
        let delta = read_uleb(bytes, cursor)?;
        method_index = method_index
            .checked_add(delta)
            .ok_or_else(|| Error::invalid("DEX", *cursor, "encoded method index overflowed"))?;
        let _ = read_uleb(bytes, cursor)?;
        let code_offset = read_uleb(bytes, cursor)?;
        if code_offset != 0 {
            output.push(MethodCode {
                method_index,
                code_offset,
            });
        }
    }
    Ok(())
}

fn instruction_range(
    bytes: &[u8],
    code_offset: u32,
    endian: Endian,
) -> Result<std::ops::Range<usize>> {
    let start = usize::try_from(code_offset)
        .map_err(|_| Error::invalid("DEX", 0, "code-item offset is too large"))?;
    require_range(bytes, start, CODE_ITEM_HEADER_SIZE, "code-item header")?;
    let count = usize::try_from(read_u32(
        bytes,
        start + CODE_INSTRUCTION_COUNT_OFFSET,
        endian,
    )?)
    .map_err(|_| Error::invalid("DEX", start, "instruction count is too large"))?;
    let byte_count = count
        .checked_mul(2)
        .ok_or_else(|| Error::invalid("DEX", start, "instruction byte count overflowed"))?;
    let instructions = start
        .checked_add(CODE_ITEM_HEADER_SIZE)
        .ok_or_else(|| Error::invalid("DEX", start, "instruction offset overflowed"))?;
    let end = instructions
        .checked_add(byte_count)
        .ok_or_else(|| Error::invalid("DEX", instructions, "instruction range overflowed"))?;
    require_range(bytes, instructions, byte_count, "code-item instructions")?;
    Ok(instructions..end)
}

fn read_words(bytes: &[u8], endian: Endian) -> Vec<u16> {
    bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|raw| match endian {
            Endian::Little => u16::from_le_bytes([raw[0], raw[1]]),
            Endian::Reverse => u16::from_be_bytes([raw[0], raw[1]]),
        })
        .collect()
}

fn write_words(bytes: &mut [u8], words: &[u16], endian: Endian) {
    for (output, word) in bytes.as_chunks_mut::<2>().0.iter_mut().zip(words) {
        output.copy_from_slice(&match endian {
            Endian::Little => word.to_le_bytes(),
            Endian::Reverse => word.to_be_bytes(),
        });
    }
}

fn dex_endian(bytes: &[u8]) -> Result<Endian> {
    match u32_at(bytes, DEX_ENDIAN_TAG_OFFSET, "DEX")? {
        DEX_LITTLE_ENDIAN_TAG => Ok(Endian::Little),
        DEX_REVERSE_ENDIAN_TAG => Ok(Endian::Reverse),
        _ => Err(Error::invalid(
            "DEX",
            DEX_ENDIAN_TAG_OFFSET,
            "invalid endian tag",
        )),
    }
}

fn read_u32(bytes: &[u8], offset: usize, endian: Endian) -> Result<u32> {
    let raw: [u8; 4] = bytes
        .get(offset..offset + 4)
        .ok_or_else(|| Error::invalid("DEX", offset, "truncated 32-bit value"))?
        .try_into()
        .map_err(|_| Error::invalid("DEX", offset, "truncated 32-bit value"))?;
    Ok(match endian {
        Endian::Little => u32::from_le_bytes(raw),
        Endian::Reverse => u32::from_be_bytes(raw),
    })
}

fn read_uleb(bytes: &[u8], cursor: &mut usize) -> Result<u32> {
    let mut value = 0u32;
    for group in 0..5u32 {
        let byte = bytes
            .get(*cursor)
            .copied()
            .ok_or_else(|| Error::invalid("DEX", *cursor, "truncated ULEB128"))?;
        *cursor += 1;
        let payload = u32::from(byte & 0x7f);
        if group == 4 && payload > 0x0f {
            return Err(Error::invalid(
                "DEX",
                *cursor - 1,
                "ULEB128 exceeds 32 bits",
            ));
        }
        value |= payload << (group * 7);
        if byte & 0x80 == 0 {
            return Ok(value);
        }
    }
    Err(Error::invalid("DEX", *cursor, "ULEB128 is too long"))
}

fn require_range(bytes: &[u8], offset: usize, length: usize, what: &str) -> Result<()> {
    if offset
        .checked_add(length)
        .is_some_and(|end| end <= bytes.len())
    {
        Ok(())
    } else {
        Err(Error::invalid("DEX", offset, format!("truncated {what}")))
    }
}

#[cfg(test)]
mod tests {
    use dex::file::compact::CompactOffsetTable;
    use dex::file::{
        AccessFlags, AnnotationDirectory, ClassData, ClassDefinition, CodeItem, DexBuilder,
        DexFile, DexVersion, EncodedMethod,
    };
    use dex::instruction::{Instruction, Opcode, Operands};

    use super::*;
    use crate::binary::update_standard_dex_integrity;

    struct QuickenedFixture {
        dex: Vec<u8>,
        code_offset: u32,
        method_index: u32,
        type_index: u16,
        checksum: u32,
        method_count: usize,
    }

    #[test]
    fn restores_version_009_length_prefixed_quickening() {
        let fixture = fixture();
        let mut quickening = Vec::new();
        quickening.extend_from_slice(&4u32.to_le_bytes());
        quickening.extend_from_slice(&0u16.to_le_bytes());
        quickening.extend_from_slice(&fixture.type_index.to_le_bytes());
        let table_offset = u32::try_from(quickening.len()).unwrap();
        quickening.extend_from_slice(&fixture.code_offset.to_le_bytes());
        quickening.extend_from_slice(&0u32.to_le_bytes());
        quickening.extend_from_slice(&table_offset.to_le_bytes());
        let vdex = legacy_vdex(*b"009\0", &fixture, &quickening, None);

        let parsed = VdexFile::parse(&vdex).unwrap();
        let canonical = parsed.canonical_standard_dex(0).unwrap();
        assert_eq!(first_opcode(&canonical), Opcode::CheckCast);
    }

    #[test]
    fn restores_version_012_compact_method_offsets() {
        let fixture = fixture();
        let record = QuickeningInfo::new(vec![0, fixture.type_index])
            .to_bytes()
            .unwrap();
        let mut quickening = vec![0];
        let record_offset = u32::try_from(quickening.len()).unwrap();
        quickening.extend_from_slice(&record);
        let mut offsets = vec![0; fixture.method_count];
        offsets[usize::try_from(fixture.method_index).unwrap()] = record_offset;
        let table_offset = u32::try_from(quickening.len()).unwrap();
        quickening.extend_from_slice(
            &CompactOffsetTable::new(offsets)
                .unwrap()
                .encode_embedded(Endian::Little)
                .unwrap(),
        );
        let vdex = legacy_vdex(*b"012\0", &fixture, &quickening, Some(table_offset));

        let parsed = VdexFile::parse(&vdex).unwrap();
        assert_eq!(
            parsed
                .quickening_info(0, fixture.method_index)
                .unwrap()
                .indices(),
            [0, fixture.type_index]
        );
        let canonical = parsed.canonical_standard_dex(0).unwrap();
        assert_eq!(first_opcode(&canonical), Opcode::CheckCast);
    }

    fn fixture() -> QuickenedFixture {
        let mut builder = DexBuilder::new(DexVersion::V040);
        let owner_handle = builder.intern_type("LExample;").unwrap();
        let method_handle = builder
            .intern_method_named("LExample;", "cast", "V", &[])
            .unwrap();
        let mut built = builder.build().unwrap();
        let owner = built.indices.type_index(owner_handle).unwrap();
        let method = built.indices.method(method_handle).unwrap();
        built
            .file
            .push_class(ClassDefinition {
                class: owner,
                access_flags: AccessFlags::PUBLIC,
                superclass: None,
                interfaces: Vec::new(),
                source_file: None,
                annotations: AnnotationDirectory::default(),
                class_data: Some(ClassData {
                    static_fields: Vec::new(),
                    instance_fields: Vec::new(),
                    direct_methods: vec![EncodedMethod {
                        method,
                        access_flags: AccessFlags::from_bits_retain(
                            AccessFlags::PUBLIC.bits() | AccessFlags::STATIC.bits(),
                        ),
                        code: Some(CodeItem {
                            registers_size: 1,
                            ins_size: 0,
                            outs_size: 0,
                            instructions: vec![
                                Instruction::operation(
                                    0,
                                    Opcode::CheckCast,
                                    Operands::RegisterIndex {
                                        register: 0,
                                        index: owner.get(),
                                    },
                                ),
                                Instruction::operation(2, Opcode::ReturnVoid, Operands::None),
                            ],
                            tries: Vec::new(),
                            debug_info: None,
                            data_offset: 0,
                        }),
                    }],
                    virtual_methods: Vec::new(),
                    data_offset: 0,
                }),
                static_values: Vec::new(),
                definition_index: 0,
            })
            .unwrap();
        let mut dex = built.file.to_bytes().unwrap();
        let parsed = DexFile::parse(&dex).unwrap();
        let code_offset = parsed.classes()[0]
            .class_data
            .as_ref()
            .unwrap()
            .direct_methods[0]
            .code
            .as_ref()
            .unwrap()
            .data_offset;
        let instruction = usize::try_from(code_offset).unwrap() + CODE_ITEM_HEADER_SIZE;
        dex[instruction] = Opcode::Nop.byte();
        dex[instruction + 1] = 0;
        update_standard_dex_integrity(&mut dex).unwrap();
        QuickenedFixture {
            checksum: u32::from_le_bytes(dex[8..12].try_into().unwrap()),
            method_count: parsed.methods().len(),
            dex,
            code_offset,
            method_index: method.get(),
            type_index: u16::try_from(owner.get()).unwrap(),
        }
    }

    fn legacy_vdex(
        version: [u8; 4],
        fixture: &QuickenedFixture,
        quickening: &[u8],
        table_offset: Option<u32>,
    ) -> Vec<u8> {
        let prefix = usize::from(table_offset.is_some()) * 4;
        let dex_size = prefix + fixture.dex.len();
        let mut bytes = vec![0; 24];
        bytes[..4].copy_from_slice(b"vdex");
        bytes[4..8].copy_from_slice(&version);
        bytes[8..12].copy_from_slice(&1u32.to_le_bytes());
        bytes[12..16].copy_from_slice(&u32::try_from(dex_size).unwrap().to_le_bytes());
        bytes[20..24].copy_from_slice(&u32::try_from(quickening.len()).unwrap().to_le_bytes());
        bytes.extend_from_slice(&fixture.checksum.to_le_bytes());
        if let Some(table_offset) = table_offset {
            bytes.extend_from_slice(&table_offset.to_le_bytes());
        }
        bytes.extend_from_slice(&fixture.dex);
        bytes.extend_from_slice(quickening);
        bytes
    }

    fn first_opcode(file: &DexFile) -> Opcode {
        file.classes()[0]
            .class_data
            .as_ref()
            .unwrap()
            .direct_methods[0]
            .code
            .as_ref()
            .unwrap()
            .instructions[0]
            .data()
            .opcode()
            .unwrap()
    }
}

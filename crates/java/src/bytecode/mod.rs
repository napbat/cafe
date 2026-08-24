//! Decoder and encoder for typed JVM method bytecode.

mod builder;
mod encode;
pub mod opcode;

use crate::classfile::CodeAttribute;
use crate::{Error, Result};

pub use self::builder::{BuiltCode, CatchTarget, CodeBuilder, InstructionId, Label, LocalKind};
pub use self::opcode::{ArrayType, Opcode};

/// Sentinel size used when constructing an instruction whose encoded size
/// should be inferred by the assembler.
pub const INFERRED_INSTRUCTION_SIZE: usize = 0;

pub(crate) const RESERVED_OPERAND_BYTE: u8 = 0;
pub(crate) const RESERVED_OPERAND_WORD: u16 = 0;
pub(crate) const MIN_INVOKE_INTERFACE_COUNT: u8 = 1;
pub(crate) const MIN_MULTI_ARRAY_DIMENSIONS: u8 = 1;
pub(crate) const SWITCH_ALIGNMENT: usize = size_of::<i32>();

const TABLE_SWITCH_TARGET_WIDTH: usize = size_of::<i32>();
const LOOKUP_SWITCH_PAIR_FIELD_COUNT: usize = 2;
const LOOKUP_SWITCH_PAIR_WIDTH: usize = size_of::<i32>() * LOOKUP_SWITCH_PAIR_FIELD_COUNT;
const END_BOUNDARY_SLOT_COUNT: usize = 1;
const INCLUSIVE_KEY_COUNT_ADJUSTMENT: i64 = 1;
const METHOD_START_OFFSET: usize = 0;
const BYTE_WIDTH: usize = size_of::<u8>();

/// One decoded JVM instruction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Instruction {
    /// Byte offset from the beginning of the method's code array.
    pub offset: usize,
    /// The effective opcode. For a `wide` instruction this is the widened opcode.
    pub opcode: Opcode,
    /// Whether the instruction used the `wide` prefix.
    pub wide: bool,
    /// Number of encoded bytes, including switch padding or a `wide` prefix.
    pub size: usize,
    /// Decoded operands.
    pub operand: Operand,
}

impl Instruction {
    /// Creates a standard-width instruction for assembly.
    ///
    /// The encoded size is inferred by [`encode`]. The caller supplies the byte
    /// offset because branch operands use absolute bytecode targets.
    #[must_use]
    pub const fn new(offset: usize, opcode: Opcode, operand: Operand) -> Self {
        Self {
            offset,
            opcode,
            wide: false,
            size: INFERRED_INSTRUCTION_SIZE,
            operand,
        }
    }

    /// Creates an instruction encoded with the JVM `wide` prefix.
    #[must_use]
    pub const fn new_wide(offset: usize, opcode: Opcode, operand: Operand) -> Self {
        Self {
            offset,
            opcode,
            wide: true,
            size: INFERRED_INSTRUCTION_SIZE,
            operand,
        }
    }

    /// Returns the standard JVM mnemonic.
    #[must_use]
    pub fn mnemonic(&self) -> &'static str {
        self.opcode.mnemonic()
    }
}

/// Operands attached to a decoded instruction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Operand {
    /// The instruction has no encoded operands.
    None,
    /// Signed one-byte immediate value.
    Byte(i8),
    /// Signed two-byte immediate value.
    Short(i16),
    /// Constant-pool index.
    Constant(u16),
    /// Local-variable index.
    Local(u16),
    /// Local-variable increment.
    Increment {
        /// Local-variable index.
        index: u16,
        /// Signed amount to add.
        value: i16,
    },
    /// Absolute bytecode target of a relative branch.
    Branch(i32),
    /// A dense integer switch table.
    TableSwitch {
        /// Default absolute target.
        default: i32,
        /// Lowest matching key.
        low: i32,
        /// Absolute targets ordered from `low` through the implicit high key.
        targets: Vec<i32>,
    },
    /// A sparse integer switch table.
    LookupSwitch {
        /// Default absolute target.
        default: i32,
        /// Key and absolute-target pairs.
        pairs: Vec<(i32, i32)>,
    },
    /// Primitive array type used by `newarray`.
    ArrayType(ArrayType),
    /// Operands of `invokeinterface`.
    InvokeInterface {
        /// Constant-pool interface method reference.
        index: u16,
        /// Number of argument slots, including the receiver.
        count: u8,
    },
    /// Constant-pool index used by `invokedynamic`.
    InvokeDynamic(u16),
    /// Operands of `multianewarray`.
    MultiArray {
        /// Constant-pool array class reference.
        index: u16,
        /// Number of dimensions to allocate.
        dimensions: u8,
    },
}

/// Decodes and structurally validates a method's complete bytecode array.
///
/// # Errors
///
/// Returns an error for a truncated, reserved, or structurally invalid
/// instruction, including branches that do not target instruction boundaries.
pub fn decode(code: &[u8]) -> Result<Vec<Instruction>> {
    let mut reader = BytecodeReader::new(code);
    let mut instructions = Vec::new();

    while !reader.is_empty() {
        let start = reader.position();
        let opcode_byte = reader.read_u8(start)?;
        let encoded_opcode = Opcode::from_byte(opcode_byte).ok_or_else(|| {
            Error::invalid_bytecode(
                start,
                format!("reserved or unknown opcode 0x{opcode_byte:02x}"),
            )
        })?;
        let (opcode, wide, operand) = if encoded_opcode == Opcode::Wide {
            decode_wide(&mut reader, start)?
        } else {
            (
                encoded_opcode,
                false,
                decode_operand(encoded_opcode, &mut reader, start)?,
            )
        };
        instructions.push(Instruction {
            offset: start,
            opcode,
            wide,
            size: reader.position() - start,
            operand,
        });
    }

    validate_targets(code.len(), &instructions)?;
    Ok(instructions)
}

pub use self::encode::encode;

/// Decodes a code attribute and also checks its exception-table boundaries.
///
/// # Errors
///
/// Returns an error when the bytecode is invalid or an exception-table entry
/// does not refer to valid instruction boundaries.
pub fn decode_code(code: &CodeAttribute) -> Result<Vec<Instruction>> {
    let instructions = decode(&code.code)?;
    let mut boundaries = vec![false; code.code.len() + END_BOUNDARY_SLOT_COUNT];
    for instruction in &instructions {
        boundaries[instruction.offset] = true;
    }
    boundaries[code.code.len()] = true;

    for handler in &code.exception_table {
        let start = usize::from(handler.start_pc);
        let end = usize::from(handler.end_pc);
        let target = usize::from(handler.handler_pc);
        if start >= end {
            return Err(Error::invalid_bytecode(
                start,
                format!("exception range start {start} is not before end {end}"),
            ));
        }
        if end > code.code.len() || !boundaries.get(end).copied().unwrap_or(false) {
            return Err(Error::invalid_bytecode(
                end,
                "exception range ends outside the code or within an instruction",
            ));
        }
        if !boundaries.get(start).copied().unwrap_or(false) {
            return Err(Error::invalid_bytecode(
                start,
                "exception range starts within an instruction",
            ));
        }
        if target >= code.code.len() || !boundaries.get(target).copied().unwrap_or(false) {
            return Err(Error::invalid_bytecode(
                target,
                "exception handler points outside the code or within an instruction",
            ));
        }
    }
    Ok(instructions)
}

/// Returns the JVM primitive type name for a `newarray` type code.
#[must_use]
pub const fn array_type_name(code: u8) -> Option<&'static str> {
    match ArrayType::from_byte(code) {
        Some(array_type) => Some(array_type.name()),
        None => None,
    }
}

#[allow(clippy::too_many_lines)] // Keeping the complete typed operand table together aids audits.
fn decode_operand(
    opcode: Opcode,
    reader: &mut BytecodeReader<'_>,
    start: usize,
) -> Result<Operand> {
    match opcode {
        Opcode::BiPush => Ok(Operand::Byte(reader.read_i8(start)?)),
        Opcode::SiPush => Ok(Operand::Short(reader.read_i16(start)?)),
        Opcode::Ldc => Ok(Operand::Constant(u16::from(reader.read_u8(start)?))),
        Opcode::LdcW
        | Opcode::Ldc2W
        | Opcode::GetStatic
        | Opcode::PutStatic
        | Opcode::GetField
        | Opcode::PutField
        | Opcode::InvokeVirtual
        | Opcode::InvokeSpecial
        | Opcode::InvokeStatic
        | Opcode::New
        | Opcode::ANewArray
        | Opcode::CheckCast
        | Opcode::InstanceOf => Ok(Operand::Constant(reader.read_u16(start)?)),
        Opcode::ILoad
        | Opcode::LLoad
        | Opcode::FLoad
        | Opcode::DLoad
        | Opcode::ALoad
        | Opcode::IStore
        | Opcode::LStore
        | Opcode::FStore
        | Opcode::DStore
        | Opcode::AStore
        | Opcode::Ret => Ok(Operand::Local(u16::from(reader.read_u8(start)?))),
        Opcode::IInc => Ok(Operand::Increment {
            index: u16::from(reader.read_u8(start)?),
            value: i16::from(reader.read_i8(start)?),
        }),
        Opcode::IfEq
        | Opcode::IfNe
        | Opcode::IfLt
        | Opcode::IfGe
        | Opcode::IfGt
        | Opcode::IfLe
        | Opcode::IfICmpEq
        | Opcode::IfICmpNe
        | Opcode::IfICmpLt
        | Opcode::IfICmpGe
        | Opcode::IfICmpGt
        | Opcode::IfICmpLe
        | Opcode::IfACmpEq
        | Opcode::IfACmpNe
        | Opcode::Goto
        | Opcode::Jsr
        | Opcode::IfNull
        | Opcode::IfNonNull => {
            let delta = i32::from(reader.read_i16(start)?);
            Ok(Operand::Branch(relative_target(start, delta, start)?))
        }
        Opcode::TableSwitch => decode_table_switch(reader, start),
        Opcode::LookupSwitch => decode_lookup_switch(reader, start),
        Opcode::InvokeInterface => {
            let index = reader.read_u16(start)?;
            let count = reader.read_u8(start)?;
            let zero = reader.read_u8(start)?;
            if zero != RESERVED_OPERAND_BYTE {
                return Err(Error::invalid_bytecode(
                    start,
                    "invokeinterface trailing byte must be zero",
                ));
            }
            if count < MIN_INVOKE_INTERFACE_COUNT {
                return Err(Error::invalid_bytecode(
                    start,
                    "invokeinterface argument count must not be zero",
                ));
            }
            Ok(Operand::InvokeInterface { index, count })
        }
        Opcode::InvokeDynamic => {
            let index = reader.read_u16(start)?;
            let zero = reader.read_u16(start)?;
            if zero != RESERVED_OPERAND_WORD {
                return Err(Error::invalid_bytecode(
                    start,
                    "invokedynamic trailing bytes must be zero",
                ));
            }
            Ok(Operand::InvokeDynamic(index))
        }
        Opcode::NewArray => {
            let array_type_byte = reader.read_u8(start)?;
            let array_type = ArrayType::from_byte(array_type_byte).ok_or_else(|| {
                Error::invalid_bytecode(
                    start,
                    format!("invalid newarray type code {array_type_byte}"),
                )
            })?;
            Ok(Operand::ArrayType(array_type))
        }
        Opcode::MultiANewArray => {
            let index = reader.read_u16(start)?;
            let dimensions = reader.read_u8(start)?;
            if dimensions < MIN_MULTI_ARRAY_DIMENSIONS {
                return Err(Error::invalid_bytecode(
                    start,
                    "multianewarray dimension count must not be zero",
                ));
            }
            Ok(Operand::MultiArray { index, dimensions })
        }
        Opcode::GotoW | Opcode::JsrW => {
            let delta = reader.read_i32(start)?;
            Ok(Operand::Branch(relative_target(start, delta, start)?))
        }
        Opcode::Wide => Err(Error::invalid_bytecode(
            start,
            "wide must be decoded as an instruction prefix",
        )),
        _ => Ok(Operand::None),
    }
}

fn decode_wide(reader: &mut BytecodeReader<'_>, start: usize) -> Result<(Opcode, bool, Operand)> {
    let opcode_byte = reader.read_u8(start)?;
    let opcode = Opcode::from_byte(opcode_byte).ok_or_else(|| {
        Error::invalid_bytecode(
            start,
            format!("reserved opcode 0x{opcode_byte:02x} cannot follow wide"),
        )
    })?;
    let operand = match opcode {
        Opcode::ILoad
        | Opcode::LLoad
        | Opcode::FLoad
        | Opcode::DLoad
        | Opcode::ALoad
        | Opcode::IStore
        | Opcode::LStore
        | Opcode::FStore
        | Opcode::DStore
        | Opcode::AStore
        | Opcode::Ret => Operand::Local(reader.read_u16(start)?),
        Opcode::IInc => Operand::Increment {
            index: reader.read_u16(start)?,
            value: reader.read_i16(start)?,
        },
        _ => {
            return Err(Error::invalid_bytecode(
                start,
                format!(
                    "opcode 0x{:02x} ({}) cannot follow wide",
                    opcode.byte(),
                    opcode.mnemonic()
                ),
            ));
        }
    };
    Ok((opcode, true, operand))
}

fn decode_table_switch(reader: &mut BytecodeReader<'_>, start: usize) -> Result<Operand> {
    read_switch_padding(reader, start)?;
    let default_delta = reader.read_i32(start)?;
    let low = reader.read_i32(start)?;
    let high = reader.read_i32(start)?;
    if high < low {
        return Err(Error::invalid_bytecode(
            start,
            format!("tableswitch high key {high} is below low key {low}"),
        ));
    }
    let count = i64::from(high) - i64::from(low) + INCLUSIVE_KEY_COUNT_ADJUSTMENT;
    let count = usize::try_from(count).map_err(|_| {
        Error::invalid_bytecode(start, "tableswitch target count does not fit usize")
    })?;
    if count > reader.remaining() / TABLE_SWITCH_TARGET_WIDTH {
        return Err(Error::invalid_bytecode(
            start,
            format!("tableswitch declares {count} targets beyond the end of the method"),
        ));
    }

    let default = relative_target(start, default_delta, start)?;
    let mut targets = Vec::with_capacity(count);
    for _ in 0..count {
        let delta = reader.read_i32(start)?;
        targets.push(relative_target(start, delta, start)?);
    }
    Ok(Operand::TableSwitch {
        default,
        low,
        targets,
    })
}

fn decode_lookup_switch(reader: &mut BytecodeReader<'_>, start: usize) -> Result<Operand> {
    read_switch_padding(reader, start)?;
    let default_delta = reader.read_i32(start)?;
    let pair_count = reader.read_i32(start)?;
    let pair_count = usize::try_from(pair_count).map_err(|_| {
        Error::invalid_bytecode(start, "lookupswitch pair count must not be negative")
    })?;
    if pair_count > reader.remaining() / LOOKUP_SWITCH_PAIR_WIDTH {
        return Err(Error::invalid_bytecode(
            start,
            format!("lookupswitch declares {pair_count} pairs beyond the end of the method"),
        ));
    }

    let default = relative_target(start, default_delta, start)?;
    let mut pairs = Vec::with_capacity(pair_count);
    let mut previous_key = None;
    for _ in 0..pair_count {
        let key = reader.read_i32(start)?;
        if previous_key.is_some_and(|previous| key <= previous) {
            return Err(Error::invalid_bytecode(
                start,
                "lookupswitch keys are not in strictly increasing order",
            ));
        }
        let delta = reader.read_i32(start)?;
        pairs.push((key, relative_target(start, delta, start)?));
        previous_key = Some(key);
    }
    Ok(Operand::LookupSwitch { default, pairs })
}

fn read_switch_padding(reader: &mut BytecodeReader<'_>, start: usize) -> Result<()> {
    let padding = (SWITCH_ALIGNMENT - (reader.position() % SWITCH_ALIGNMENT)) % SWITCH_ALIGNMENT;
    for _ in 0..padding {
        if reader.read_u8(start)? != RESERVED_OPERAND_BYTE {
            return Err(Error::invalid_bytecode(
                start,
                "switch alignment padding must contain zero bytes",
            ));
        }
    }
    Ok(())
}

fn relative_target(start: usize, delta: i32, error_offset: usize) -> Result<i32> {
    let target = i64::try_from(start).unwrap_or(i64::MAX) + i64::from(delta);
    i32::try_from(target).map_err(|_| {
        Error::invalid_bytecode(
            error_offset,
            "relative branch target overflows a signed offset",
        )
    })
}

fn validate_targets(code_length: usize, instructions: &[Instruction]) -> Result<()> {
    let mut boundaries = vec![false; code_length + END_BOUNDARY_SLOT_COUNT];
    for instruction in instructions {
        boundaries[instruction.offset] = true;
    }

    for instruction in instructions {
        match &instruction.operand {
            Operand::Branch(target) => validate_target(*target, instruction.offset, &boundaries)?,
            Operand::TableSwitch {
                default, targets, ..
            } => {
                validate_target(*default, instruction.offset, &boundaries)?;
                for &target in targets {
                    validate_target(target, instruction.offset, &boundaries)?;
                }
            }
            Operand::LookupSwitch { default, pairs } => {
                validate_target(*default, instruction.offset, &boundaries)?;
                for &(_, target) in pairs {
                    validate_target(target, instruction.offset, &boundaries)?;
                }
            }
            Operand::None
            | Operand::Byte(_)
            | Operand::Short(_)
            | Operand::Constant(_)
            | Operand::Local(_)
            | Operand::Increment { .. }
            | Operand::ArrayType(_)
            | Operand::InvokeInterface { .. }
            | Operand::InvokeDynamic(_)
            | Operand::MultiArray { .. } => {}
        }
    }
    Ok(())
}

fn validate_target(target: i32, source: usize, boundaries: &[bool]) -> Result<()> {
    let target = usize::try_from(target).map_err(|_| {
        Error::invalid_bytecode(
            source,
            format!("branch target {target} is before the method"),
        )
    })?;
    if target >= boundaries.len().saturating_sub(END_BOUNDARY_SLOT_COUNT) {
        return Err(Error::invalid_bytecode(
            source,
            format!("branch target {target} is outside the method"),
        ));
    }
    if !boundaries[target] {
        return Err(Error::invalid_bytecode(
            source,
            format!("branch target {target} is inside another instruction"),
        ));
    }
    Ok(())
}

struct BytecodeReader<'a> {
    code: &'a [u8],
    position: usize,
}

impl<'a> BytecodeReader<'a> {
    const fn new(code: &'a [u8]) -> Self {
        Self {
            code,
            position: METHOD_START_OFFSET,
        }
    }

    const fn position(&self) -> usize {
        self.position
    }

    const fn remaining(&self) -> usize {
        self.code.len() - self.position
    }

    const fn is_empty(&self) -> bool {
        self.remaining() == 0
    }

    fn read_u8(&mut self, instruction_offset: usize) -> Result<u8> {
        let value = *self.code.get(self.position).ok_or_else(|| {
            Error::invalid_bytecode(instruction_offset, "truncated instruction operand")
        })?;
        self.position += BYTE_WIDTH;
        Ok(value)
    }

    fn read_i8(&mut self, instruction_offset: usize) -> Result<i8> {
        Ok(self.read_u8(instruction_offset)?.cast_signed())
    }

    fn read_u16(&mut self, instruction_offset: usize) -> Result<u16> {
        Ok(u16::from_be_bytes([
            self.read_u8(instruction_offset)?,
            self.read_u8(instruction_offset)?,
        ]))
    }

    fn read_i16(&mut self, instruction_offset: usize) -> Result<i16> {
        Ok(self.read_u16(instruction_offset)?.cast_signed())
    }

    fn read_i32(&mut self, instruction_offset: usize) -> Result<i32> {
        Ok(i32::from_be_bytes([
            self.read_u8(instruction_offset)?,
            self.read_u8(instruction_offset)?,
            self.read_u8(instruction_offset)?,
            self.read_u8(instruction_offset)?,
        ]))
    }
}

/// Returns the standard mnemonic for an encoded opcode byte.
#[must_use]
pub const fn mnemonic(opcode: u8) -> Option<&'static str> {
    match Opcode::from_byte(opcode) {
        Some(opcode) => Some(opcode.mnemonic()),
        None => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{Instruction, Opcode, Operand, decode, encode};

    #[test]
    fn assembles_fresh_typed_instructions() {
        let instructions = [
            Instruction::new(0, Opcode::IConst0, Operand::None),
            Instruction::new(1, Opcode::IReturn, Operand::None),
        ];
        assert_eq!(
            encode(&instructions).unwrap(),
            [Opcode::IConst0.byte(), Opcode::IReturn.byte()]
        );
    }

    #[test]
    fn decodes_constant_pool_and_wide_operands() {
        let code = [0x2a, 0xb7, 0x00, 0x0c, 0xc4, 0x15, 0x01, 0x2c, 0xb1];
        let instructions = decode(&code).unwrap();
        assert_eq!(instructions.len(), 4);
        assert_eq!(instructions[1].operand, Operand::Constant(12));
        assert!(instructions[2].wide);
        assert_eq!(instructions[2].operand, Operand::Local(300));
        assert_eq!(instructions[3].offset, 8);
        assert_eq!(encode(&instructions).unwrap(), code);
    }

    #[test]
    fn decodes_aligned_table_switch() {
        let code = [
            0xaa, 0, 0, 0, // opcode and padding
            0, 0, 0, 26, // default -> 26
            0, 0, 0, 1, // low
            0, 0, 0, 2, // high
            0, 0, 0, 24, // key 1 -> 24
            0, 0, 0, 25, // key 2 -> 25
            0x03, 0x04, 0xac, // iconst_0, iconst_1, ireturn
        ];
        let instructions = decode(&code).unwrap();
        assert_eq!(instructions.len(), 4);
        assert_eq!(instructions[0].size, 24);
        assert!(matches!(
            &instructions[0].operand,
            Operand::TableSwitch { default: 26, low: 1, targets }
                if targets == &[24, 25]
        ));
        assert_eq!(encode(&instructions).unwrap(), code);
    }

    #[test]
    fn rejects_branches_into_an_operand() {
        let error = decode(&[0xa7, 0x00, 0x01]).unwrap_err();
        assert!(error.to_string().contains("inside another instruction"));
    }

    #[test]
    fn rejects_reserved_opcodes() {
        let error = decode(&[0xcb]).unwrap_err();
        assert!(error.to_string().contains("reserved"));
    }

    #[test]
    fn mnemonic_table_covers_every_defined_opcode() {
        for &opcode in Opcode::ALL {
            assert_eq!(super::mnemonic(opcode.byte()), Some(opcode.mnemonic()));
        }
        for opcode in 0xcb..=0xfd {
            assert!(super::mnemonic(opcode).is_none());
        }
    }
}

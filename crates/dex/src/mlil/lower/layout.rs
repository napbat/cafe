//! Fixed-width Dalvik layout with symbolic block and payload targets.

use std::collections::BTreeMap;

use disassembler::cfglib::BlockId;

use crate::instruction::{ArrayDataPayload, Instruction, Opcode, Operands, SparseSwitchPayload};

use super::super::{Error, Result};

#[derive(Debug, Clone)]
enum Planned {
    Plain(Opcode, Operands),
    Goto(BlockId),
    ConditionalSkip {
        opcode: Opcode,
        first: u16,
        second: Option<u16>,
    },
    Switch {
        register: u16,
        payload: usize,
    },
    FillArray {
        register: u16,
        payload: usize,
    },
    SparsePayload {
        keys: Vec<i32>,
        targets: Vec<BlockId>,
    },
    ArrayPayload(ArrayDataPayload),
}

impl Planned {
    fn width(&self) -> Option<u32> {
        match self {
            Self::Plain(opcode, _) | Self::ConditionalSkip { opcode, .. } => {
                Some(opcode.format().code_units())
            }
            Self::Goto(_) | Self::Switch { .. } | Self::FillArray { .. } => Some(3),
            Self::SparsePayload { keys, .. } => u32::try_from(keys.len())
                .ok()?
                .checked_mul(4)?
                .checked_add(2),
            Self::ArrayPayload(payload) => payload.code_units(),
        }
    }
}

#[derive(Debug, Clone)]
enum PayloadRequest {
    Switch {
        keys: Vec<i32>,
        targets: Vec<BlockId>,
    },
    Array(ArrayDataPayload),
}

pub(super) struct Planner {
    items: Vec<Planned>,
    offsets: Vec<u32>,
    blocks: BTreeMap<BlockId, u32>,
    payloads: Vec<PayloadRequest>,
    payload_offsets: Vec<u32>,
    cursor: u32,
}

impl Planner {
    pub(super) fn new() -> Self {
        Self {
            items: Vec::new(),
            offsets: Vec::new(),
            blocks: BTreeMap::new(),
            payloads: Vec::new(),
            payload_offsets: Vec::new(),
            cursor: 0,
        }
    }

    pub(super) const fn cursor(&self) -> u32 {
        self.cursor
    }

    pub(super) fn bind(&mut self, block: BlockId) {
        self.blocks.insert(block, self.cursor);
    }

    pub(super) fn plain(&mut self, opcode: Opcode, operands: Operands) -> Result<u32> {
        self.push(Planned::Plain(opcode, operands))
    }

    pub(super) fn goto(&mut self, target: BlockId) -> Result<u32> {
        self.push(Planned::Goto(target))
    }

    pub(super) fn conditional_skip(
        &mut self,
        opcode: Opcode,
        first: u16,
        second: Option<u16>,
    ) -> Result<u32> {
        self.push(Planned::ConditionalSkip {
            opcode,
            first,
            second,
        })
    }

    pub(super) fn switch(
        &mut self,
        register: u16,
        keys: Vec<i32>,
        targets: Vec<BlockId>,
    ) -> Result<u32> {
        let payload = self.payloads.len();
        self.payloads.push(PayloadRequest::Switch { keys, targets });
        self.push(Planned::Switch { register, payload })
    }

    pub(super) fn fill_array(
        &mut self,
        register: u16,
        payload_value: ArrayDataPayload,
    ) -> Result<u32> {
        let payload = self.payloads.len();
        self.payloads.push(PayloadRequest::Array(payload_value));
        self.push(Planned::FillArray { register, payload })
    }

    pub(super) fn block_offset(&self, block: BlockId) -> Option<u32> {
        self.blocks.get(&block).copied()
    }

    pub(super) fn finish(mut self) -> Result<Vec<Instruction>> {
        for payload in std::mem::take(&mut self.payloads) {
            if !self.cursor.is_multiple_of(2) {
                let safe_target = self.blocks.keys().next().copied().ok_or_else(|| {
                    Error::lowering(
                        ::mlil::InstructionId::from_raw(0),
                        "Dalvik payload alignment has no executable target",
                    )
                })?;
                self.push(Planned::Goto(safe_target))?;
            }
            self.payload_offsets.push(self.cursor);
            match payload {
                PayloadRequest::Switch { keys, targets } => {
                    self.push(Planned::SparsePayload { keys, targets })?;
                }
                PayloadRequest::Array(payload) => {
                    self.push(Planned::ArrayPayload(payload))?;
                }
            }
        }
        self.items
            .iter()
            .zip(&self.offsets)
            .map(|(item, &offset)| self.render(item, offset))
            .collect()
    }

    fn push(&mut self, item: Planned) -> Result<u32> {
        let offset = self.cursor;
        let width = item.width().ok_or_else(|| {
            Error::lowering(
                ::mlil::InstructionId::from_raw(0),
                "Dalvik planned instruction width overflowed",
            )
        })?;
        self.cursor = self.cursor.checked_add(width).ok_or_else(|| {
            Error::lowering(
                ::mlil::InstructionId::from_raw(0),
                "Dalvik instruction layout exceeds u32",
            )
        })?;
        self.offsets.push(offset);
        self.items.push(item);
        Ok(offset)
    }

    fn render(&self, item: &Planned, offset: u32) -> Result<Instruction> {
        let instruction = match item {
            Planned::Plain(opcode, operands) => {
                Instruction::operation(offset, *opcode, operands.clone())
            }
            Planned::Goto(target) => Instruction::operation(
                offset,
                Opcode::Goto32,
                Operands::Branch {
                    target: self.require_block(*target)?,
                },
            ),
            Planned::ConditionalSkip {
                opcode,
                first,
                second,
            } => {
                let target = offset.checked_add(5).ok_or_else(|| {
                    Error::lowering(
                        ::mlil::InstructionId::from_raw(0),
                        "conditional skip target overflowed",
                    )
                })?;
                let operands = second.map_or(
                    Operands::RegisterBranch {
                        register: *first,
                        target,
                    },
                    |second| Operands::RegistersBranch {
                        first: *first,
                        second,
                        target,
                    },
                );
                Instruction::operation(offset, *opcode, operands)
            }
            Planned::Switch { register, payload } => Instruction::operation(
                offset,
                Opcode::SparseSwitch,
                Operands::RegisterBranch {
                    register: *register,
                    target: self.payload_offsets[*payload],
                },
            ),
            Planned::FillArray { register, payload } => Instruction::operation(
                offset,
                Opcode::FillArrayData,
                Operands::RegisterBranch {
                    register: *register,
                    target: self.payload_offsets[*payload],
                },
            ),
            Planned::SparsePayload { keys, targets } => Instruction::sparse_switch(
                offset,
                SparseSwitchPayload {
                    keys: keys.clone(),
                    targets: targets
                        .iter()
                        .map(|target| self.require_block(*target))
                        .collect::<Result<Vec<_>>>()?,
                },
            ),
            Planned::ArrayPayload(payload) => Instruction::array_data(offset, payload.clone()),
        };
        Ok(instruction)
    }

    fn require_block(&self, block: BlockId) -> Result<u32> {
        self.block_offset(block).ok_or_else(|| {
            Error::lowering(
                ::mlil::InstructionId::from_raw(0),
                format!("Dalvik target block {block} has no generated offset"),
            )
        })
    }
}

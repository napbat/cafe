//! DEX state-machine debugging information.

use super::{StringIndex, TypeIndex};

/// Fixed debug-state-machine opcode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub(crate) enum DebugOpcode {
    EndSequence = 0x00,
    AdvancePc = 0x01,
    AdvanceLine = 0x02,
    StartLocal = 0x03,
    StartLocalExtended = 0x04,
    EndLocal = 0x05,
    RestartLocal = 0x06,
    SetPrologueEnd = 0x07,
    SetEpilogueBegin = 0x08,
    SetFile = 0x09,
}

impl DebugOpcode {
    pub(crate) const fn from_byte(value: u8) -> Option<Self> {
        match value {
            0x00 => Some(Self::EndSequence),
            0x01 => Some(Self::AdvancePc),
            0x02 => Some(Self::AdvanceLine),
            0x03 => Some(Self::StartLocal),
            0x04 => Some(Self::StartLocalExtended),
            0x05 => Some(Self::EndLocal),
            0x06 => Some(Self::RestartLocal),
            0x07 => Some(Self::SetPrologueEnd),
            0x08 => Some(Self::SetEpilogueBegin),
            0x09 => Some(Self::SetFile),
            _ => None,
        }
    }

    pub(crate) const fn byte(self) -> u8 {
        self as u8
    }
}

/// First compact line-position opcode.
pub(crate) const FIRST_SPECIAL_DEBUG_OPCODE: u8 = 0x0a;
/// Smallest line delta represented by a compact position opcode.
pub(crate) const DEBUG_LINE_BASE: i32 = -4;
/// Number of line deltas represented at each compact address step.
pub(crate) const DEBUG_LINE_RANGE: u32 = 15;
/// Smallest valid source line in a debug program.
pub(crate) const MINIMUM_DEBUG_LINE: u32 = 1;
/// Initial code-unit address of the debug state machine.
pub(crate) const INITIAL_DEBUG_ADDRESS: u32 = 0;

/// Parsed `debug_info_item` retaining native event order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DebugInfo {
    /// Initial source line.
    pub line_start: u32,
    /// Optional parameter-name indices in declaration order.
    pub parameter_names: Vec<Option<StringIndex>>,
    /// State-machine events through the mandatory end marker.
    pub events: Vec<DebugEvent>,
    /// Original absolute data offset.
    pub data_offset: u32,
}

/// One debugging state-machine event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DebugEvent {
    /// End the sequence.
    EndSequence,
    /// Advance the current code-unit address.
    AdvancePc(u32),
    /// Advance the current source line.
    AdvanceLine(i32),
    /// Begin a local variable's live range.
    StartLocal {
        /// Register containing the local.
        register: u32,
        /// Optional local name.
        name: Option<StringIndex>,
        /// Optional local type.
        local_type: Option<TypeIndex>,
    },
    /// Begin a local variable with a generic signature.
    StartLocalExtended {
        /// Register containing the local.
        register: u32,
        /// Optional local name.
        name: Option<StringIndex>,
        /// Optional local type.
        local_type: Option<TypeIndex>,
        /// Optional generic-signature string.
        signature: Option<StringIndex>,
    },
    /// End the current local variable in a register.
    EndLocal(u32),
    /// Restart the most recently ended local in a register.
    RestartLocal(u32),
    /// Mark the next position as the end of a prologue.
    SetPrologueEnd,
    /// Mark the next position as the beginning of an epilogue.
    SetEpilogueBegin,
    /// Change the current source file.
    SetFile(Option<StringIndex>),
    /// Combined address and line advancement encoded by a special opcode.
    Position {
        /// Code-unit address delta.
        address_delta: u32,
        /// Source-line delta.
        line_delta: i32,
    },
}

//! Dalvik LLIL method-body metadata and checked native conversion.

use crate::Result;
use crate::analysis::analyze_body;
use crate::file::{CodeItem, DebugInfo, TryBlock};

use super::{Instruction, lift_instructions, lower_instructions};

/// One complete Dalvik LLIL method body.
///
/// Register-frame declarations, ordered exception handlers, debug events, and
/// the original code-item location remain exact. Offset-changing transforms
/// must update that metadata before reverse-conversion validation can succeed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Body {
    /// Number of registers in the frame.
    pub registers_size: u16,
    /// Incoming argument width in register words.
    pub ins_size: u16,
    /// Outgoing argument width in register words.
    pub outs_size: u16,
    /// Normalized Dalvik LLIL instructions and payloads in native layout order.
    pub instructions: Vec<Instruction>,
    /// Exact ordered native protected regions and handlers.
    pub tries: Vec<TryBlock>,
    /// Exact optional DEX debugging state machine.
    pub debug_info: Option<DebugInfo>,
    /// Original absolute `code_item` offset.
    pub data_offset: u32,
}

impl Body {
    /// Lifts a checked native DEX code item into Dalvik LLIL.
    ///
    /// # Errors
    ///
    /// Returns an error when the native instruction layout, register frame,
    /// result pairing, payload relationships, or exception metadata is invalid.
    pub fn from_code(code: &CodeItem) -> Result<Self> {
        analyze_body(code)?;
        Ok(Self {
            registers_size: code.registers_size,
            ins_size: code.ins_size,
            outs_size: code.outs_size,
            instructions: lift_instructions(&code.instructions)?,
            tries: code.tries.clone(),
            debug_info: code.debug_info.clone(),
            data_offset: code.data_offset,
        })
    }

    /// Reconstructs and validates the exact native DEX code item.
    ///
    /// # Errors
    ///
    /// Returns an error for inconsistent LLIL/native encodings, malformed
    /// layout or operands, stale protected regions, invalid handlers, or broken
    /// result and payload relationships.
    pub fn to_code(&self) -> Result<CodeItem> {
        let candidate = CodeItem {
            registers_size: self.registers_size,
            ins_size: self.ins_size,
            outs_size: self.outs_size,
            instructions: lower_instructions(&self.instructions)?,
            tries: self.tries.clone(),
            debug_info: self.debug_info.clone(),
            data_offset: self.data_offset,
        };
        analyze_body(&candidate)?;
        Ok(candidate)
    }

    /// Verifies semantic/encoding agreement and all retained native metadata.
    ///
    /// # Errors
    ///
    /// Returns the same validation errors as [`Self::to_code`].
    pub fn verify(&self) -> Result<()> {
        self.to_code().map(drop)
    }
}

/// Lifts a complete native DEX code item into Dalvik LLIL.
///
/// # Errors
///
/// Returns an error for malformed instructions, registers, control flow, or
/// exception metadata.
pub fn lift_code(code: &CodeItem) -> Result<Body> {
    Body::from_code(code)
}

/// Lowers a Dalvik LLIL body back into a checked native DEX code item.
///
/// # Errors
///
/// Returns an error for stale encoding provenance, layout, or body metadata.
pub fn lower_code(body: &Body) -> Result<CodeItem> {
    body.to_code()
}

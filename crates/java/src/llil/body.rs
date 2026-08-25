//! JVM LLIL method-body metadata and checked native conversion.

use crate::Result;
use crate::bytecode;
use crate::classfile::{Attribute, CodeAttribute, ExceptionHandler};

use super::{Instruction, lift_instructions, lower_instructions};

/// One complete JVM LLIL method body.
///
/// Native handlers and nested code attributes remain exact. Offset-changing
/// transformations must update them through the class-file editing APIs before
/// a body can pass reverse-conversion validation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Body {
    /// Constant-pool index of the enclosing `Code` attribute name.
    pub name_index: u16,
    /// Declared maximum operand-stack depth.
    pub max_stack: u16,
    /// Declared number of local-variable slots.
    pub max_locals: u16,
    /// Normalized JVM LLIL instructions in native layout order.
    pub instructions: Vec<Instruction>,
    /// Exact ordered native exception table.
    pub exception_table: Vec<ExceptionHandler>,
    /// Exact nested code attributes, including debug data and stack maps.
    pub attributes: Vec<Attribute>,
}

impl Body {
    /// Lifts a checked native `Code` attribute into JVM LLIL.
    ///
    /// # Errors
    ///
    /// Returns an error when bytecode, control-flow targets, or exception-table
    /// boundaries are malformed.
    pub fn from_code(code: &CodeAttribute) -> Result<Self> {
        let native = bytecode::decode_code(code)?;
        Ok(Self {
            name_index: code.name_index,
            max_stack: code.max_stack,
            max_locals: code.max_locals,
            instructions: lift_instructions(&native)?,
            exception_table: code.exception_table.clone(),
            attributes: code.attributes.clone(),
        })
    }

    /// Reconstructs and validates the exact native `Code` attribute.
    ///
    /// # Errors
    ///
    /// Returns an error for inconsistent LLIL/native encodings, invalid layout,
    /// invalid targets, or stale exception and nested metadata boundaries.
    pub fn to_code(&self) -> Result<CodeAttribute> {
        let native = lower_instructions(&self.instructions)?;
        let candidate = CodeAttribute {
            name_index: self.name_index,
            max_stack: self.max_stack,
            max_locals: self.max_locals,
            code: bytecode::encode(&native)?,
            exception_table: self.exception_table.clone(),
            attributes: self.attributes.clone(),
        };
        bytecode::decode_code(&candidate)?;
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

/// Lifts a complete native JVM body into LLIL.
///
/// # Errors
///
/// Returns an error for malformed bytecode or metadata.
pub fn lift_code(code: &CodeAttribute) -> Result<Body> {
    Body::from_code(code)
}

/// Lowers a JVM LLIL body back into a checked native `Code` attribute.
///
/// # Errors
///
/// Returns an error for stale encoding provenance, layout, or metadata.
pub fn lower_code(body: &Body) -> Result<CodeAttribute> {
    body.to_code()
}

//! Transactional method-body editing and offset-sensitive metadata handling.

mod offsets;

pub use self::offsets::BytecodeOffsetMap;

use std::collections::HashSet;

use crate::bytecode::{self, Instruction};
use crate::{Error, Result};

use super::{
    Attribute, CodeAttribute, KnownAttribute, LocalVariableTarget, StackMapFrame,
    TypeAnnotationTarget, VerificationType,
};

impl CodeAttribute {
    /// Decodes this attribute into typed instructions.
    ///
    /// # Errors
    ///
    /// Returns an error if the bytecode or exception-table boundaries are invalid.
    pub fn instructions(&self) -> Result<Vec<Instruction>> {
        bytecode::decode_code(self)
    }

    /// Replaces this method body when all existing metadata offsets stay valid.
    ///
    /// If the method has offset-sensitive metadata, the replacement must retain
    /// every instruction boundary and the code length. Use
    /// [`Self::set_instructions_with_offset_map`] for a structural rewrite or
    /// [`Self::set_instructions_dropping_metadata`] when metadata is disposable.
    /// The operation is transactional.
    ///
    /// # Errors
    ///
    /// Returns an error if encoding fails, exception boundaries become invalid,
    /// or a layout change would leave metadata stale.
    pub fn set_instructions(&mut self, instructions: &[Instruction]) -> Result<()> {
        let new_code = bytecode::encode(instructions)?;
        if self.has_offset_sensitive_metadata() && !same_layout(&self.code, &new_code)? {
            return Err(Error::invalid_assembly(
                "bytecode layout changed while offset-sensitive metadata is present",
            ));
        }
        self.replace_code_transactionally(new_code)
    }

    /// Replaces this method body and removes all nested code metadata and handlers.
    ///
    /// This explicit destructive variant is useful when a later analysis pass
    /// will regenerate stack maps and debugging information.
    ///
    /// # Errors
    ///
    /// Returns an error if the replacement instructions cannot be encoded.
    pub fn set_instructions_dropping_metadata(
        &mut self,
        instructions: &[Instruction],
    ) -> Result<()> {
        let new_code = bytecode::encode(instructions)?;
        self.code = new_code;
        self.exception_table.clear();
        self.attributes.clear();
        Ok(())
    }

    /// Replaces this body and remaps every modeled offset-sensitive structure.
    ///
    /// Unknown nested code attributes are rejected because Cafe cannot know
    /// whether their payload embeds bytecode offsets. The operation is
    /// transactional and retains the original body on any failure.
    ///
    /// # Errors
    ///
    /// Returns an error for an incomplete or non-monotonic map, an unknown code
    /// attribute, an invalid replacement body, or an unrepresentable range.
    pub fn set_instructions_with_offset_map(
        &mut self,
        instructions: &[Instruction],
        offsets: &BytecodeOffsetMap,
    ) -> Result<()> {
        let new_code = bytecode::encode(instructions)?;
        let mut candidate = self.clone();
        candidate.code = new_code;
        remap_metadata(&mut candidate, offsets)?;
        validate_metadata_boundaries(&candidate)?;
        *self = candidate;
        Ok(())
    }

    /// Returns whether handlers or nested attributes may contain bytecode offsets.
    #[must_use]
    pub fn has_offset_sensitive_metadata(&self) -> bool {
        !self.exception_table.is_empty() || !self.attributes.is_empty()
    }

    fn replace_code_transactionally(&mut self, new_code: Vec<u8>) -> Result<()> {
        let old_code = std::mem::replace(&mut self.code, new_code);
        if let Err(error) = bytecode::decode_code(self) {
            self.code = old_code;
            return Err(error);
        }
        Ok(())
    }
}

fn same_layout(old_code: &[u8], new_code: &[u8]) -> Result<bool> {
    let old = bytecode::decode(old_code)?;
    let new = bytecode::decode(new_code)?;
    Ok(old_code.len() == new_code.len()
        && old.len() == new.len()
        && old
            .iter()
            .zip(new)
            .all(|(old, new)| old.offset == new.offset && old.size == new.size))
}

fn remap_metadata(code: &mut CodeAttribute, offsets: &BytecodeOffsetMap) -> Result<()> {
    for handler in &mut code.exception_table {
        handler.start_pc = offsets.require(handler.start_pc, "exception start")?;
        handler.end_pc = offsets.require(handler.end_pc, "exception end")?;
        handler.handler_pc = offsets.require(handler.handler_pc, "exception handler")?;
    }
    for attribute in &mut code.attributes {
        match attribute {
            Attribute::Known(attribute) => remap_known_attribute(attribute, offsets)?,
            Attribute::Raw(attribute) => {
                return Err(Error::invalid_assembly(format!(
                    "cannot remap unknown code attribute `{}`",
                    attribute.name
                )));
            }
            Attribute::Code(_) => {
                return Err(Error::invalid_assembly(
                    "a Code attribute cannot contain another Code attribute",
                ));
            }
        }
    }
    Ok(())
}

fn remap_known_attribute(
    attribute: &mut KnownAttribute,
    offsets: &BytecodeOffsetMap,
) -> Result<()> {
    match attribute {
        KnownAttribute::LineNumberTable(table) => {
            for line in &mut table.lines {
                line.start_pc = offsets.require(line.start_pc, "line-number")?;
            }
        }
        KnownAttribute::LocalVariableTable(table) => {
            for variable in &mut table.variables {
                remap_range(&mut variable.start_pc, &mut variable.length, offsets)?;
            }
        }
        KnownAttribute::LocalVariableTypeTable(table) => {
            for variable in &mut table.variables {
                remap_range(&mut variable.start_pc, &mut variable.length, offsets)?;
            }
        }
        KnownAttribute::StackMapTable(table) => remap_stack_maps(&mut table.frames, offsets)?,
        KnownAttribute::RuntimeVisibleTypeAnnotations(table)
        | KnownAttribute::RuntimeInvisibleTypeAnnotations(table) => {
            for annotation in &mut table.annotations {
                remap_type_target(&mut annotation.target, offsets)?;
            }
        }
        _ => {
            return Err(Error::invalid_assembly(format!(
                "{} is not a valid modeled Code attribute",
                attribute.name()
            )));
        }
    }
    Ok(())
}

fn remap_range(start: &mut u16, length: &mut u16, offsets: &BytecodeOffsetMap) -> Result<()> {
    let old_end = start
        .checked_add(*length)
        .ok_or_else(|| Error::invalid_assembly("bytecode metadata range overflows u16"))?;
    let new_start = offsets.require(*start, "range start")?;
    let new_end = offsets.require(old_end, "range end")?;
    let new_length = new_end
        .checked_sub(new_start)
        .ok_or_else(|| Error::invalid_assembly("offset map reverses a metadata range"))?;
    *start = new_start;
    *length = new_length;
    Ok(())
}

fn remap_stack_maps(frames: &mut Vec<StackMapFrame>, offsets: &BytecodeOffsetMap) -> Result<()> {
    let mut old_previous = None;
    let mut new_previous = None;
    let mut remapped = Vec::with_capacity(frames.len());
    for mut frame in std::mem::take(frames) {
        let old_absolute = absolute_frame_offset(old_previous, frame.offset_delta())?;
        let new_absolute = offsets.require(old_absolute, "stack-map frame")?;
        let new_delta = frame_delta(new_previous, new_absolute)?;
        remap_frame_verification_types(&mut frame, offsets)?;
        remapped.push(frame_with_delta(frame, new_delta)?);
        old_previous = Some(old_absolute);
        new_previous = Some(new_absolute);
    }
    *frames = remapped;
    Ok(())
}

fn absolute_frame_offset(previous: Option<u16>, delta: u16) -> Result<u16> {
    match previous {
        None => Ok(delta),
        Some(previous) => previous
            .checked_add(delta)
            .and_then(|offset| offset.checked_add(1))
            .ok_or_else(|| Error::invalid_assembly("stack-map frame offset overflows u16")),
    }
}

fn frame_delta(previous: Option<u16>, absolute: u16) -> Result<u16> {
    match previous {
        None => Ok(absolute),
        Some(previous) => absolute
            .checked_sub(previous)
            .and_then(|distance| distance.checked_sub(1))
            .ok_or_else(|| Error::invalid_assembly("offset map reorders stack-map frames")),
    }
}

fn frame_with_delta(frame: StackMapFrame, delta: u16) -> Result<StackMapFrame> {
    let frame = match frame {
        StackMapFrame::Same { .. } if delta <= 63 => StackMapFrame::Same {
            offset_delta: u8::try_from(delta)
                .map_err(|_| Error::invalid_assembly("compact stack-map offset exceeds u8"))?,
        },
        StackMapFrame::Same { .. } | StackMapFrame::SameExtended { .. } => {
            StackMapFrame::SameExtended {
                offset_delta: delta,
            }
        }
        StackMapFrame::SameLocalsOneStack { stack, .. }
        | StackMapFrame::SameLocalsOneStackExtended { stack, .. }
            if delta <= 63 =>
        {
            StackMapFrame::SameLocalsOneStack {
                offset_delta: u8::try_from(delta)
                    .map_err(|_| Error::invalid_assembly("compact stack-map offset exceeds u8"))?,
                stack,
            }
        }
        StackMapFrame::SameLocalsOneStack { stack, .. }
        | StackMapFrame::SameLocalsOneStackExtended { stack, .. } => {
            StackMapFrame::SameLocalsOneStackExtended {
                offset_delta: delta,
                stack,
            }
        }
        StackMapFrame::Chop { absent_locals, .. } => StackMapFrame::Chop {
            offset_delta: delta,
            absent_locals,
        },
        StackMapFrame::Append { locals, .. } => StackMapFrame::Append {
            offset_delta: delta,
            locals,
        },
        StackMapFrame::Full { locals, stack, .. } => StackMapFrame::Full {
            offset_delta: delta,
            locals,
            stack,
        },
    };
    Ok(frame)
}

fn remap_frame_verification_types(
    frame: &mut StackMapFrame,
    offsets: &BytecodeOffsetMap,
) -> Result<()> {
    match frame {
        StackMapFrame::Same { .. }
        | StackMapFrame::Chop { .. }
        | StackMapFrame::SameExtended { .. } => {}
        StackMapFrame::SameLocalsOneStack { stack, .. }
        | StackMapFrame::SameLocalsOneStackExtended { stack, .. } => {
            remap_verification_type(stack, offsets)?;
        }
        StackMapFrame::Append { locals, .. } => {
            for value in locals {
                remap_verification_type(value, offsets)?;
            }
        }
        StackMapFrame::Full { locals, stack, .. } => {
            for value in locals.iter_mut().chain(stack) {
                remap_verification_type(value, offsets)?;
            }
        }
    }
    Ok(())
}

fn remap_verification_type(
    value: &mut VerificationType,
    offsets: &BytecodeOffsetMap,
) -> Result<()> {
    if let VerificationType::Uninitialized(offset) = value {
        *offset = offsets.require(*offset, "uninitialized value")?;
    }
    Ok(())
}

fn remap_type_target(target: &mut TypeAnnotationTarget, offsets: &BytecodeOffsetMap) -> Result<()> {
    match target {
        TypeAnnotationTarget::LocalVariable(ranges)
        | TypeAnnotationTarget::ResourceVariable(ranges) => {
            for LocalVariableTarget {
                start_pc, length, ..
            } in ranges
            {
                remap_range(start_pc, length, offsets)?;
            }
        }
        TypeAnnotationTarget::InstanceOf(offset)
        | TypeAnnotationTarget::New(offset)
        | TypeAnnotationTarget::ConstructorReference(offset)
        | TypeAnnotationTarget::MethodReference(offset)
        | TypeAnnotationTarget::Cast { offset, .. }
        | TypeAnnotationTarget::ConstructorInvocationTypeArgument { offset, .. }
        | TypeAnnotationTarget::MethodInvocationTypeArgument { offset, .. }
        | TypeAnnotationTarget::ConstructorReferenceTypeArgument { offset, .. }
        | TypeAnnotationTarget::MethodReferenceTypeArgument { offset, .. } => {
            *offset = offsets.require(*offset, "type annotation")?;
        }
        TypeAnnotationTarget::ClassTypeParameter(_)
        | TypeAnnotationTarget::MethodTypeParameter(_)
        | TypeAnnotationTarget::ClassExtends(_)
        | TypeAnnotationTarget::ClassTypeParameterBound { .. }
        | TypeAnnotationTarget::MethodTypeParameterBound { .. }
        | TypeAnnotationTarget::Field
        | TypeAnnotationTarget::MethodReturn
        | TypeAnnotationTarget::MethodReceiver
        | TypeAnnotationTarget::MethodFormalParameter(_)
        | TypeAnnotationTarget::Throws(_)
        | TypeAnnotationTarget::ExceptionParameter(_) => {}
    }
    Ok(())
}

fn validate_metadata_boundaries(code: &CodeAttribute) -> Result<()> {
    let instructions = bytecode::decode_code(code)?;
    let boundaries: HashSet<u16> = instructions
        .iter()
        .map(|instruction| {
            u16::try_from(instruction.offset)
                .map_err(|_| Error::invalid_assembly("instruction offset exceeds u16"))
        })
        .chain(std::iter::once(u16::try_from(code.code.len()).map_err(
            |_| Error::invalid_assembly("code length exceeds u16 metadata range"),
        )))
        .collect::<Result<_>>()?;
    for attribute in &code.attributes {
        validate_known_boundaries(attribute, &boundaries, code.code.len())?;
    }
    Ok(())
}

fn validate_known_boundaries(
    attribute: &Attribute,
    boundaries: &HashSet<u16>,
    code_length: usize,
) -> Result<()> {
    let Attribute::Known(attribute) = attribute else {
        return Err(Error::invalid_assembly(
            "only modeled attributes can be checked after offset remapping",
        ));
    };
    match attribute {
        KnownAttribute::LineNumberTable(table) => {
            for line in &table.lines {
                require_boundary(line.start_pc, boundaries, code_length, false)?;
            }
        }
        KnownAttribute::LocalVariableTable(table) => {
            for variable in &table.variables {
                require_range(variable.start_pc, variable.length, boundaries)?;
            }
        }
        KnownAttribute::LocalVariableTypeTable(table) => {
            for variable in &table.variables {
                require_range(variable.start_pc, variable.length, boundaries)?;
            }
        }
        KnownAttribute::StackMapTable(table) => {
            let mut previous = None;
            for frame in &table.frames {
                let offset = absolute_frame_offset(previous, frame.offset_delta())?;
                require_boundary(offset, boundaries, code_length, false)?;
                previous = Some(offset);
            }
        }
        KnownAttribute::RuntimeVisibleTypeAnnotations(_)
        | KnownAttribute::RuntimeInvisibleTypeAnnotations(_) => {}
        _ => {
            return Err(Error::invalid_assembly(format!(
                "{} is not valid inside Code",
                attribute.name()
            )));
        }
    }
    Ok(())
}

fn require_range(start: u16, length: u16, boundaries: &HashSet<u16>) -> Result<()> {
    let end = start
        .checked_add(length)
        .ok_or_else(|| Error::invalid_assembly("remapped metadata range overflows u16"))?;
    if boundaries.contains(&start) && boundaries.contains(&end) {
        Ok(())
    } else {
        Err(Error::invalid_assembly(format!(
            "remapped metadata range {start}..{end} is not aligned to instructions"
        )))
    }
}

fn require_boundary(
    offset: u16,
    boundaries: &HashSet<u16>,
    code_length: usize,
    allow_end: bool,
) -> Result<()> {
    if boundaries.contains(&offset) && (allow_end || usize::from(offset) < code_length) {
        Ok(())
    } else {
        Err(Error::invalid_assembly(format!(
            "remapped bytecode offset {offset} is not an instruction boundary"
        )))
    }
}

//! Full-frame `StackMapTable` encoding and code-attribute installation.

use crate::classfile::{
    Attribute, CodeAttribute, ConstantPool, KnownAttribute, StackMapFrame, StackMapTableAttribute,
    VerificationType,
};
use crate::{Error, Result};

use super::model::{FrameState, FrameValue, MethodAnalysis};

const STACK_MAP_TABLE_ATTRIBUTE_NAME: &str = "StackMapTable";
const STACK_MAP_OFFSET_BIAS: usize = 1;

impl MethodAnalysis {
    /// Encodes every reachable non-entry frame as a conservative full frame.
    ///
    /// Object and array symbols are interned into the supplied constant pool.
    /// Full frames trade compactness for deterministic, easily audited output.
    ///
    /// # Errors
    ///
    /// Returns an error for an unrepresentable offset, malformed internal wide
    /// slot, or a constant pool that cannot accept a referenced class.
    pub fn stack_map_table(&self, pool: &mut ConstantPool) -> Result<StackMapTableAttribute> {
        let mut frames = Vec::new();
        let mut previous = None;
        for (&offset, frame) in &self.entries {
            if offset == self.flow.entry() {
                continue;
            }
            let delta = match previous {
                None => offset,
                Some(previous) => offset
                    .checked_sub(previous)
                    .and_then(|distance| distance.checked_sub(STACK_MAP_OFFSET_BIAS))
                    .ok_or_else(|| {
                        Error::invalid_assembly("stack-map frame offsets are not increasing")
                    })?,
            };
            let offset_delta = u16::try_from(delta)
                .map_err(|_| Error::invalid_assembly("stack-map frame offset delta exceeds u16"))?;
            frames.push(StackMapFrame::Full {
                offset_delta,
                locals: verification_locals(pool, frame)?,
                stack: verification_stack(pool, frame)?,
            });
            previous = Some(offset);
        }
        Ok(StackMapTableAttribute {
            name_index: pool.intern_utf8(STACK_MAP_TABLE_ATTRIBUTE_NAME)?,
            frames,
        })
    }

    /// Installs computed maxima and replaces the code's stack-map table.
    ///
    /// Existing non-stack-map attributes retain their relative order. The new
    /// table occupies the first removed table's position or is appended.
    ///
    /// # Errors
    ///
    /// Returns an error if frame offsets or required class constants cannot be
    /// represented.
    pub fn apply_to_code(&self, pool: &mut ConstantPool, code: &mut CodeAttribute) -> Result<()> {
        let table = self.stack_map_table(pool)?;
        let insertion = code
            .attributes
            .iter()
            .position(|attribute| {
                matches!(
                    attribute,
                    Attribute::Known(KnownAttribute::StackMapTable(_))
                )
            })
            .unwrap_or(code.attributes.len());
        let mut retained = Vec::with_capacity(code.attributes.len() + 1);
        for attribute in std::mem::take(&mut code.attributes) {
            if !matches!(
                attribute,
                Attribute::Known(KnownAttribute::StackMapTable(_))
            ) {
                retained.push(attribute);
            }
        }
        retained.insert(
            insertion.min(retained.len()),
            Attribute::Known(KnownAttribute::StackMapTable(table)),
        );
        code.attributes = retained;
        code.max_stack = self.max_stack;
        code.max_locals = self.max_locals;
        Ok(())
    }
}

fn verification_locals(
    pool: &mut ConstantPool,
    frame: &FrameState,
) -> Result<Vec<VerificationType>> {
    let end = frame
        .locals
        .iter()
        .rposition(|value| *value != FrameValue::Top)
        .map_or(0, |position| position + 1);
    let mut values = Vec::new();
    let mut position = 0;
    while position < end {
        let value = &frame.locals[position];
        values.push(verification_type(pool, value)?);
        if value.is_category_two() {
            if frame.locals.get(position + 1) != Some(&FrameValue::WideContinuation) {
                return Err(Error::invalid_assembly(
                    "wide local lacks its continuation during stack-map encoding",
                ));
            }
            position += 2;
        } else if *value == FrameValue::WideContinuation {
            return Err(Error::invalid_assembly(
                "orphaned wide continuation during stack-map encoding",
            ));
        } else {
            position += 1;
        }
    }
    Ok(values)
}

fn verification_stack(
    pool: &mut ConstantPool,
    frame: &FrameState,
) -> Result<Vec<VerificationType>> {
    frame
        .stack
        .iter()
        .map(|value| verification_type(pool, value))
        .collect()
}

fn verification_type(pool: &mut ConstantPool, value: &FrameValue) -> Result<VerificationType> {
    Ok(match value {
        FrameValue::Top => VerificationType::Top,
        FrameValue::Integer => VerificationType::Integer,
        FrameValue::Float => VerificationType::Float,
        FrameValue::Long => VerificationType::Long,
        FrameValue::Double => VerificationType::Double,
        FrameValue::Null => VerificationType::Null,
        FrameValue::Reference(name) => VerificationType::Object(pool.intern_class(name)?),
        FrameValue::UninitializedThis => VerificationType::UninitializedThis,
        FrameValue::Uninitialized { offset, .. } => VerificationType::Uninitialized(*offset),
        FrameValue::WideContinuation => {
            return Err(Error::invalid_assembly(
                "wide continuation cannot be encoded as a verification type",
            ));
        }
    })
}

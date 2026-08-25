//! JVM frame values and native storage mapped into MLIL variables.

use ::mlil::{
    AllocationSite, FunctionBuilder, NativeVariable, SourceStorage, TypedVariable, ValueType,
    VariableId, VariableRole,
};
use disassembler::{BinaryFormat, CodeAddress};

use crate::analysis::{FrameState, FrameValue};
use crate::classfile::MethodAccessFlags;
use crate::descriptor::MethodDescriptor;

use super::Result;

pub(super) struct StateVariables {
    locals: Vec<VariableId>,
    stack: Vec<VariableId>,
}

impl StateVariables {
    pub(super) fn declare(
        builder: &mut FunctionBuilder,
        max_locals: u16,
        max_stack: u16,
        descriptor: &MethodDescriptor,
        access_flags: MethodAccessFlags,
    ) -> Result<Self> {
        let parameter_roles = parameter_roles(max_locals, descriptor, access_flags);
        let locals = (0..max_locals)
            .map(|index| {
                builder.declare_variable(
                    parameter_roles[usize::from(index)],
                    Some(NativeVariable {
                        format: BinaryFormat::JavaClass,
                        storage: SourceStorage::JvmLocal(index),
                    }),
                )
            })
            .collect::<::mlil::Result<Vec<_>>>()?;
        let stack = (0..max_stack)
            .map(|index| {
                builder.declare_variable(
                    VariableRole::Temporary,
                    Some(NativeVariable {
                        format: BinaryFormat::JavaClass,
                        storage: SourceStorage::JvmStack(index),
                    }),
                )
            })
            .collect::<::mlil::Result<Vec<_>>>()?;
        Ok(Self { locals, stack })
    }

    pub(super) fn local(&self, frame: &FrameState, index: u16, owner: &str) -> TypedVariable {
        TypedVariable::new(
            self.locals[usize::from(index)],
            frame
                .locals()
                .get(usize::from(index))
                .map_or(ValueType::Unknown, |value| value_type(value, owner)),
        )
    }

    pub(super) fn stack(&self, frame: &FrameState, index: usize, owner: &str) -> TypedVariable {
        TypedVariable::new(
            self.stack[index],
            frame
                .stack()
                .get(index)
                .map_or(ValueType::Unknown, |value| value_type(value, owner)),
        )
    }

    pub(super) fn is_native_state(&self, variable: VariableId) -> bool {
        self.locals.contains(&variable) || self.stack.contains(&variable)
    }
}

pub(super) fn value_type(value: &FrameValue, owner: &str) -> ValueType {
    match value {
        FrameValue::Top => ValueType::Unknown,
        FrameValue::Integer => ValueType::Integer,
        FrameValue::Float => ValueType::Float,
        FrameValue::Long => ValueType::Long,
        FrameValue::Double => ValueType::Double,
        FrameValue::Null => ValueType::Null,
        FrameValue::Reference(name) => ValueType::Reference(Some(reference_descriptor(name))),
        FrameValue::UninitializedThis => ValueType::UninitializedThis(reference_descriptor(owner)),
        FrameValue::Uninitialized { class, offset } => ValueType::Uninitialized {
            descriptor: reference_descriptor(class),
            site: AllocationSite {
                format: BinaryFormat::JavaClass,
                address: CodeAddress::from(*offset),
            },
        },
        FrameValue::WideContinuation => ValueType::Conflict,
    }
}

pub(super) fn reference_descriptor(name: &str) -> String {
    if name.starts_with('[') {
        name.to_owned()
    } else {
        format!("L{name};")
    }
}

fn parameter_roles(
    max_locals: u16,
    descriptor: &MethodDescriptor,
    access_flags: MethodAccessFlags,
) -> Vec<VariableRole> {
    let mut roles = vec![VariableRole::Local; usize::from(max_locals)];
    let mut slot = 0usize;
    let mut ordinal = 0u16;
    if !access_flags.contains(MethodAccessFlags::STATIC) {
        roles[slot] = VariableRole::Parameter(ordinal);
        slot += 1;
        ordinal += 1;
    }
    for parameter in &descriptor.parameters {
        if let Some(role) = roles.get_mut(slot) {
            *role = VariableRole::Parameter(ordinal);
        }
        slot += parameter.slot_width().slot_count();
        ordinal = ordinal.saturating_add(1);
    }
    roles
}

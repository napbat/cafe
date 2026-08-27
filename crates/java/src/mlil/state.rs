//! JVM frame values and native storage mapped into MLIL variables.

use ::mlil::{
    AllocationSite, FunctionBuilder, JavaDialect, NativeVariable, SourceStorage, TypedVariable,
    ValueType, VariableId, VariableRole,
};
use disassembler::{BinaryFormat, CodeAddress};

use crate::analysis::{FrameState, FrameValue};
use crate::classfile::MethodAccessFlags;
use crate::descriptor::{JavaType, MethodDescriptor, ReturnType};

use super::Result;

pub(crate) trait VariableAllocator {
    fn declare_variable(
        &mut self,
        role: VariableRole,
        native: Option<NativeVariable>,
    ) -> ::mlil::Result<VariableId>;
}

impl VariableAllocator for FunctionBuilder {
    fn declare_variable(
        &mut self,
        role: VariableRole,
        native: Option<NativeVariable>,
    ) -> ::mlil::Result<VariableId> {
        cfglib::ir::mlil::FunctionBuilder::<JavaDialect>::declare_variable(self, role, native)
    }
}

pub(crate) struct StateVariables {
    locals: Vec<VariableId>,
    stack: Vec<VariableId>,
    parameters: Vec<VariableId>,
    returns: Vec<ValueType>,
}

impl StateVariables {
    pub(crate) fn declare(
        builder: &mut (impl VariableAllocator + ?Sized),
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
        let parameters = parameter_roles
            .iter()
            .enumerate()
            .filter_map(|(index, role)| {
                matches!(role, VariableRole::Parameter(_)).then_some(locals[index])
            })
            .collect();
        let returns = match &descriptor.return_type {
            ReturnType::Void => Vec::new(),
            ReturnType::Type(value) => vec![descriptor_value_type(value)],
        };
        Ok(Self {
            locals,
            stack,
            parameters,
            returns,
        })
    }

    pub(crate) fn parameters(&self) -> &[VariableId] {
        &self.parameters
    }

    pub(crate) fn returns(&self) -> &[ValueType] {
        &self.returns
    }

    pub(crate) fn local(&self, frame: &FrameState, index: u16, owner: &str) -> TypedVariable {
        TypedVariable::new(
            self.locals[usize::from(index)],
            frame
                .locals()
                .get(usize::from(index))
                .map_or(ValueType::Unknown, |value| value_type(value, owner)),
        )
    }

    pub(crate) fn stack(&self, frame: &FrameState, index: usize, owner: &str) -> TypedVariable {
        TypedVariable::new(
            self.stack[index],
            frame
                .stack()
                .get(index)
                .map_or(ValueType::Unknown, |value| value_type(value, owner)),
        )
    }
}

fn descriptor_value_type(value: &JavaType) -> ValueType {
    match value {
        JavaType::Byte | JavaType::Char | JavaType::Int | JavaType::Short | JavaType::Boolean => {
            ValueType::Integer
        }
        JavaType::Long => ValueType::Long,
        JavaType::Float => ValueType::Float,
        JavaType::Double => ValueType::Double,
        JavaType::Object(name) => ValueType::Reference(Some(reference_descriptor(name))),
        JavaType::Array(_) => ValueType::Reference(Some(java_type_descriptor(value))),
    }
}

fn java_type_descriptor(value: &JavaType) -> String {
    match value {
        JavaType::Byte => "B".to_owned(),
        JavaType::Char => "C".to_owned(),
        JavaType::Double => "D".to_owned(),
        JavaType::Float => "F".to_owned(),
        JavaType::Int => "I".to_owned(),
        JavaType::Long => "J".to_owned(),
        JavaType::Short => "S".to_owned(),
        JavaType::Boolean => "Z".to_owned(),
        JavaType::Object(name) => reference_descriptor(name),
        JavaType::Array(element) => format!("[{}", java_type_descriptor(element)),
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

pub(crate) fn reference_descriptor(name: &str) -> String {
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

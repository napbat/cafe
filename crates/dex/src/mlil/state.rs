//! Dalvik register lattice values mapped into MLIL variables.

use ::mlil::{
    AllocationSite, FunctionBuilder, NativeVariable, SourceStorage, TypedVariable, ValueType,
    VariableId, VariableRole,
};
use disassembler::{BinaryFormat, CodeAddress};

use crate::analysis::{ReferenceType, RegisterFrame, RegisterType, ValueKind};
use crate::file::{AccessFlags, CodeItem, DexFile, EncodedMethod};

use super::Result;

pub(super) struct StateVariables {
    registers: Vec<VariableId>,
    pub(super) result: VariableId,
    pub(super) exception: VariableId,
}

impl StateVariables {
    pub(super) fn declare(
        builder: &mut FunctionBuilder,
        file: &DexFile,
        declaration: &EncodedMethod,
        code: &CodeItem,
    ) -> Result<Self> {
        let roles = parameter_roles(file, declaration, code)?;
        let registers = (0..code.registers_size)
            .map(|register| {
                builder.declare_variable(
                    roles[usize::from(register)],
                    Some(NativeVariable {
                        format: BinaryFormat::Dex,
                        storage: SourceStorage::DexRegister(register),
                    }),
                )
            })
            .collect::<::mlil::Result<Vec<_>>>()?;
        let result = builder.declare_variable(
            VariableRole::Temporary,
            Some(NativeVariable {
                format: BinaryFormat::Dex,
                storage: SourceStorage::DexResult,
            }),
        )?;
        let exception = builder.declare_variable(
            VariableRole::Exception,
            Some(NativeVariable {
                format: BinaryFormat::Dex,
                storage: SourceStorage::DexException,
            }),
        )?;
        Ok(Self {
            registers,
            result,
            exception,
        })
    }

    pub(super) fn register(
        &self,
        frame: Option<&RegisterFrame>,
        register: u16,
        fallback: &ValueKind,
    ) -> TypedVariable {
        let value_type = frame
            .and_then(|frame| frame.register(register))
            .map_or_else(|| value_kind(fallback), register_type);
        TypedVariable::new(self.registers[usize::from(register)], value_type)
    }

    pub(super) fn is_native_state(&self, variable: VariableId) -> bool {
        self.registers.contains(&variable) || variable == self.result
    }
}

pub(super) fn register_type(value: &RegisterType) -> ValueType {
    match value {
        RegisterType::Unknown => ValueType::Unknown,
        RegisterType::Conflict | RegisterType::WideContinuation => ValueType::Conflict,
        RegisterType::Zero => ValueType::Zero,
        RegisterType::Single => ValueType::Bits32,
        RegisterType::Integer => ValueType::Integer,
        RegisterType::Float => ValueType::Float,
        RegisterType::WideZero | RegisterType::Wide => ValueType::Bits64,
        RegisterType::Long => ValueType::Long,
        RegisterType::Double => ValueType::Double,
        RegisterType::Reference(reference) => reference_type(reference),
    }
}

pub(super) fn value_kind(value: &ValueKind) -> ValueType {
    match value {
        ValueKind::Single => ValueType::Bits32,
        ValueKind::Wide => ValueType::Bits64,
        ValueKind::Integer => ValueType::Integer,
        ValueKind::Float => ValueType::Float,
        ValueKind::Long => ValueType::Long,
        ValueKind::Double => ValueType::Double,
        ValueKind::Reference => ValueType::Reference(None),
    }
}

fn reference_type(reference: &ReferenceType) -> ValueType {
    match reference {
        ReferenceType::Any => ValueType::Reference(None),
        ReferenceType::Descriptor(descriptor) => ValueType::Reference(Some(descriptor.clone())),
        ReferenceType::Uninitialized {
            descriptor,
            allocation_offset,
        } => ValueType::Uninitialized {
            descriptor: descriptor.clone(),
            site: AllocationSite {
                format: BinaryFormat::Dex,
                address: CodeAddress::from(*allocation_offset),
            },
        },
        ReferenceType::UninitializedThis { descriptor } => {
            ValueType::UninitializedThis(descriptor.clone())
        }
    }
}

fn parameter_roles(
    file: &DexFile,
    declaration: &EncodedMethod,
    code: &CodeItem,
) -> Result<Vec<VariableRole>> {
    let mut roles = vec![VariableRole::Local; usize::from(code.registers_size)];
    let mut cursor = usize::from(code.registers_size - code.ins_size);
    let mut ordinal = 0u16;
    if !declaration.access_flags.contains(AccessFlags::STATIC) {
        roles[cursor] = VariableRole::Parameter(ordinal);
        cursor += 1;
        ordinal += 1;
    }
    let method = file.resolve_method_id(declaration.method)?;
    let prototype = file.resolve_prototype(method.prototype)?;
    for parameter in &prototype.parameters {
        if let Some(role) = roles.get_mut(cursor) {
            *role = VariableRole::Parameter(ordinal);
        }
        let descriptor = file.type_descriptor(*parameter)?;
        cursor += usize::from(matches!(descriptor.as_bytes().first(), Some(b'J' | b'D'))) + 1;
        ordinal = ordinal.saturating_add(1);
    }
    Ok(roles)
}

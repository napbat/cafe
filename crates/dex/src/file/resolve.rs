//! Strongly typed identifier resolution and descriptor construction.

use std::fmt;

use super::{
    CallSite, CallSiteIndex, DexFile, DexString, FieldId, FieldIndex, MethodHandle,
    MethodHandleIndex, MethodHandleKind, MethodId, MethodIndex, PrototypeId, PrototypeIndex,
    StringIndex, TypeId, TypeIndex,
};
use crate::{Error, IdentifierTable, Result};

/// Fully resolved DEX field identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ResolvedField<'a> {
    /// Declaring class descriptor.
    pub owner: &'a str,
    /// Field name.
    pub name: &'a str,
    /// Field type descriptor.
    pub field_type: &'a str,
}

impl fmt::Display for ResolvedField<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{}->{}:{}",
            self.owner, self.name, self.field_type
        )
    }
}

/// Fully resolved DEX method identity.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ResolvedMethod<'a> {
    /// Declaring class descriptor.
    pub owner: &'a str,
    /// Method name.
    pub name: &'a str,
    /// JVM-compatible method descriptor assembled from the prototype table.
    pub signature: String,
}

impl fmt::Display for ResolvedMethod<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}->{}{}", self.owner, self.name, self.signature)
    }
}

/// Resolved target selected by a DEX method handle.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ResolvedMethodHandleTarget<'a> {
    /// Field target selected by a get or put handle.
    Field(ResolvedField<'a>),
    /// Method target selected by an invocation handle.
    Method(ResolvedMethod<'a>),
}

impl fmt::Display for ResolvedMethodHandleTarget<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Field(field) => field.fmt(formatter),
            Self::Method(method) => method.fmt(formatter),
        }
    }
}

/// Fully resolved DEX method handle.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ResolvedMethodHandle<'a> {
    /// Handle invocation or field-access behavior.
    pub kind: MethodHandleKind,
    /// Field or method identity selected by the handle kind.
    pub target: ResolvedMethodHandleTarget<'a>,
}

impl fmt::Display for ResolvedMethodHandle<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{} {}", self.kind.name(), self.target)
    }
}

impl DexFile {
    /// Resolves a string-table index.
    ///
    /// # Errors
    ///
    /// Returns an error when the index is outside the string table.
    pub fn resolve_string(&self, index: StringIndex) -> Result<&DexString> {
        get(self.strings(), index.get(), IdentifierTable::String)
    }

    /// Resolves a type-table entry.
    ///
    /// # Errors
    ///
    /// Returns an error when the type index is outside the table.
    pub fn resolve_type(&self, index: TypeIndex) -> Result<&TypeId> {
        get(self.types(), index.get(), IdentifierTable::Type)
    }

    /// Resolves a type index to its exact descriptor text.
    ///
    /// # Errors
    ///
    /// Returns an error when either the type or descriptor-string index is invalid.
    pub fn type_descriptor(&self, index: TypeIndex) -> Result<&str> {
        let descriptor = self.resolve_type(index)?.descriptor;
        Ok(&self.resolve_string(descriptor)?.text)
    }

    /// Resolves a prototype-table entry.
    ///
    /// # Errors
    ///
    /// Returns an error when the prototype index is outside the table.
    pub fn resolve_prototype(&self, index: PrototypeIndex) -> Result<&PrototypeId> {
        get(self.prototypes(), index.get(), IdentifierTable::Prototype)
    }

    /// Constructs the JVM-compatible descriptor represented by a DEX prototype.
    ///
    /// # Errors
    ///
    /// Returns an error when the prototype or one of its type references is invalid.
    pub fn prototype_descriptor(&self, index: PrototypeIndex) -> Result<String> {
        let prototype = self.resolve_prototype(index)?;
        let mut descriptor = String::from("(");
        for &parameter in &prototype.parameters {
            descriptor.push_str(self.type_descriptor(parameter)?);
        }
        descriptor.push(')');
        descriptor.push_str(self.type_descriptor(prototype.return_type)?);
        Ok(descriptor)
    }

    /// Resolves a field-table entry.
    ///
    /// # Errors
    ///
    /// Returns an error when the field index is outside the table.
    pub fn resolve_field_id(&self, index: FieldIndex) -> Result<&FieldId> {
        get(self.fields(), index.get(), IdentifierTable::Field)
    }

    /// Resolves a complete field identity.
    ///
    /// # Errors
    ///
    /// Returns an error when the field or any referenced identifier is invalid.
    pub fn resolve_field(&self, index: FieldIndex) -> Result<ResolvedField<'_>> {
        let field = self.resolve_field_id(index)?;
        Ok(ResolvedField {
            owner: self.type_descriptor(field.class)?,
            name: &self.resolve_string(field.name)?.text,
            field_type: self.type_descriptor(field.field_type)?,
        })
    }

    /// Resolves a method-table entry.
    ///
    /// # Errors
    ///
    /// Returns an error when the method index is outside the table.
    pub fn resolve_method_id(&self, index: MethodIndex) -> Result<&MethodId> {
        get(self.methods(), index.get(), IdentifierTable::Method)
    }

    /// Resolves a complete method identity.
    ///
    /// # Errors
    ///
    /// Returns an error when the method or any referenced identifier is invalid.
    pub fn resolve_method(&self, index: MethodIndex) -> Result<ResolvedMethod<'_>> {
        let method = self.resolve_method_id(index)?;
        Ok(ResolvedMethod {
            owner: self.type_descriptor(method.class)?,
            name: &self.resolve_string(method.name)?.text,
            signature: self.prototype_descriptor(method.prototype)?,
        })
    }

    /// Resolves a call-site table entry.
    ///
    /// # Errors
    ///
    /// Returns an error when the call-site index is outside the table.
    pub fn resolve_call_site(&self, index: CallSiteIndex) -> Result<&CallSite> {
        get(self.call_sites(), index.get(), IdentifierTable::CallSite)
    }

    /// Resolves a method-handle table entry without following its target.
    ///
    /// # Errors
    ///
    /// Returns an error when the method-handle index is outside the table.
    pub fn resolve_method_handle_id(&self, index: MethodHandleIndex) -> Result<&MethodHandle> {
        get(
            self.method_handles(),
            index.get(),
            IdentifierTable::MethodHandle,
        )
    }

    /// Resolves a method handle and its typed field or method target.
    ///
    /// # Errors
    ///
    /// Returns an error when the handle or target-table index is invalid.
    pub fn resolve_method_handle(
        &self,
        index: MethodHandleIndex,
    ) -> Result<ResolvedMethodHandle<'_>> {
        let handle = self.resolve_method_handle_id(index)?;
        let target_index = u32::from(handle.target_index);
        let target = if handle.kind.references_field() {
            ResolvedMethodHandleTarget::Field(self.resolve_field(FieldIndex::new(target_index))?)
        } else {
            ResolvedMethodHandleTarget::Method(self.resolve_method(MethodIndex::new(target_index))?)
        };
        Ok(ResolvedMethodHandle {
            kind: handle.kind,
            target,
        })
    }
}

fn get<T>(values: &[T], index: u32, table: IdentifierTable) -> Result<&T> {
    usize::try_from(index)
        .ok()
        .and_then(|position| values.get(position))
        .ok_or(Error::InvalidIndex { table, index })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::file::{DexString, DexVersion};

    #[test]
    fn resolves_method_descriptors_through_typed_indices() {
        let mut file = DexFile::new(DexVersion::V040);
        let name = file.push_string(DexString::new("run")).unwrap();
        let owner_descriptor = file.push_string(DexString::new("LExample;")).unwrap();
        let int_descriptor = file.push_string(DexString::new("I")).unwrap();
        let void_descriptor = file.push_string(DexString::new("V")).unwrap();
        let owner = file
            .push_type(TypeId {
                descriptor: owner_descriptor,
            })
            .unwrap();
        let int = file
            .push_type(TypeId {
                descriptor: int_descriptor,
            })
            .unwrap();
        let void = file
            .push_type(TypeId {
                descriptor: void_descriptor,
            })
            .unwrap();
        let prototype = file
            .push_prototype(PrototypeId {
                shorty: void_descriptor,
                return_type: void,
                parameters: vec![int],
                parameters_offset: 0,
            })
            .unwrap();
        let method = file
            .push_method(MethodId {
                class: owner,
                prototype,
                name,
            })
            .unwrap();

        let resolved = file.resolve_method(method).unwrap();
        assert_eq!(resolved.owner, "LExample;");
        assert_eq!(resolved.name, "run");
        assert_eq!(resolved.signature, "(I)V");
    }
}

//! Owned program type, field, and method definitions.

use std::collections::BTreeMap;

use disassembler::{ControlFlowGraph, FunctionBody, GraphError, RawAccessFlags};

use crate::{DefinitionKind, Error, FieldId, MethodId, Result, SymbolComponent, TypeId};

/// One field definition owned by a [`TypeDefinition`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldDefinition {
    id: FieldId,
    access_flags: RawAccessFlags,
}

impl FieldDefinition {
    /// Creates a detached field definition.
    ///
    /// # Errors
    ///
    /// Returns an error if the field name or signature is empty.
    pub fn new(id: FieldId, access_flags: RawAccessFlags) -> Result<Self> {
        validate_member_id(DefinitionKind::Field, &id.name, &id.signature)?;
        Ok(Self { id, access_flags })
    }

    /// Returns the field's native identity.
    #[must_use]
    pub const fn id(&self) -> &FieldId {
        &self.id
    }

    /// Returns the unmodified native access-flag bits.
    #[must_use]
    pub const fn access_flags(&self) -> RawAccessFlags {
        self.access_flags
    }

    /// Replaces the native access-flag bits.
    pub const fn set_access_flags(&mut self, access_flags: RawAccessFlags) {
        self.access_flags = access_flags;
    }
}

/// One method definition owned by a [`TypeDefinition`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MethodDefinition {
    id: MethodId,
    access_flags: RawAccessFlags,
    body: Option<FunctionBody>,
}

impl MethodDefinition {
    /// Creates a detached method definition.
    ///
    /// # Errors
    ///
    /// Returns an error if the method name or signature is empty.
    pub fn new(
        id: MethodId,
        access_flags: RawAccessFlags,
        body: Option<FunctionBody>,
    ) -> Result<Self> {
        validate_member_id(DefinitionKind::Method, &id.name, &id.signature)?;
        Ok(Self {
            id,
            access_flags,
            body,
        })
    }

    /// Returns the method's native overload identity.
    #[must_use]
    pub const fn id(&self) -> &MethodId {
        &self.id
    }

    /// Returns the unmodified native access-flag bits.
    #[must_use]
    pub const fn access_flags(&self) -> RawAccessFlags {
        self.access_flags
    }

    /// Replaces the native access-flag bits.
    pub const fn set_access_flags(&mut self, access_flags: RawAccessFlags) {
        self.access_flags = access_flags;
    }

    /// Returns the executable body, if this is not a declaration-only method.
    #[must_use]
    pub const fn body(&self) -> Option<&FunctionBody> {
        self.body.as_ref()
    }

    /// Returns the editable executable body, if present.
    #[must_use]
    pub const fn body_mut(&mut self) -> Option<&mut FunctionBody> {
        self.body.as_mut()
    }

    /// Replaces the executable body and returns the previous body.
    pub fn replace_body(&mut self, body: Option<FunctionBody>) -> Option<FunctionBody> {
        std::mem::replace(&mut self.body, body)
    }

    /// Builds a verified graph for the executable body, when present.
    ///
    /// # Errors
    ///
    /// Returns an error when the retained disassembly has invalid instruction
    /// ordering, branches, exception ranges, or graph structure.
    pub fn control_flow_graph(&self) -> std::result::Result<Option<ControlFlowGraph>, GraphError> {
        self.body
            .as_ref()
            .map(FunctionBody::control_flow_graph)
            .transpose()
    }
}

/// One format-qualified type definition and its owned members.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypeDefinition {
    id: TypeId,
    access_flags: RawAccessFlags,
    superclass: Option<TypeId>,
    interfaces: Vec<TypeId>,
    fields: Vec<FieldDefinition>,
    field_index: BTreeMap<FieldId, usize>,
    methods: Vec<MethodDefinition>,
    method_index: BTreeMap<MethodId, usize>,
}

impl TypeDefinition {
    /// Creates an empty detached type definition.
    ///
    /// # Errors
    ///
    /// Returns an error if the native type name is empty.
    pub fn new(id: TypeId, access_flags: RawAccessFlags) -> Result<Self> {
        if id.name.is_empty() {
            return Err(Error::EmptySymbol {
                kind: DefinitionKind::Type,
                component: SymbolComponent::Type,
            });
        }
        Ok(Self {
            id,
            access_flags,
            superclass: None,
            interfaces: Vec::new(),
            fields: Vec::new(),
            field_index: BTreeMap::new(),
            methods: Vec::new(),
            method_index: BTreeMap::new(),
        })
    }

    /// Returns the format-qualified type identity.
    #[must_use]
    pub const fn id(&self) -> &TypeId {
        &self.id
    }

    /// Returns the unmodified native access-flag bits.
    #[must_use]
    pub const fn access_flags(&self) -> RawAccessFlags {
        self.access_flags
    }

    /// Replaces the native access-flag bits.
    pub const fn set_access_flags(&mut self, access_flags: RawAccessFlags) {
        self.access_flags = access_flags;
    }

    /// Returns the direct superclass or base type.
    #[must_use]
    pub const fn superclass(&self) -> Option<&TypeId> {
        self.superclass.as_ref()
    }

    /// Replaces the direct superclass or base type.
    pub fn set_superclass(&mut self, superclass: Option<TypeId>) {
        self.superclass = superclass;
    }

    /// Returns directly implemented or extended interfaces in native order.
    #[must_use]
    pub fn interfaces(&self) -> &[TypeId] {
        &self.interfaces
    }

    /// Adds an interface unless that exact reference is already present.
    ///
    /// Returns whether the interface was inserted.
    pub fn add_interface(&mut self, interface: TypeId) -> bool {
        if self.interfaces.contains(&interface) {
            false
        } else {
            self.interfaces.push(interface);
            true
        }
    }

    /// Iterates through fields in native declaration order.
    #[must_use]
    pub fn fields(&self) -> impl ExactSizeIterator<Item = &FieldDefinition> {
        self.fields.iter()
    }

    /// Iterates through editable fields without permitting identity changes.
    pub fn fields_mut(&mut self) -> impl ExactSizeIterator<Item = &mut FieldDefinition> {
        self.fields.iter_mut()
    }

    /// Returns one exact field definition.
    #[must_use]
    pub fn field(&self, id: &FieldId) -> Option<&FieldDefinition> {
        self.field_index.get(id).map(|&index| &self.fields[index])
    }

    /// Returns one exact editable field definition.
    #[must_use]
    pub fn field_mut(&mut self, id: &FieldId) -> Option<&mut FieldDefinition> {
        let index = self.field_index.get(id).copied()?;
        self.fields.get_mut(index)
    }

    /// Adds a field while preserving declaration order and indexed lookup.
    ///
    /// # Errors
    ///
    /// Returns an error if this type already contains the same name/signature.
    pub fn insert_field(&mut self, field: FieldDefinition) -> Result<()> {
        let id = field.id.clone();
        if self.field_index.contains_key(&id) {
            return Err(self.duplicate_member(DefinitionKind::Field, &id.name, &id.signature));
        }
        self.field_index.insert(id, self.fields.len());
        self.fields.push(field);
        Ok(())
    }

    /// Iterates through methods in native declaration order.
    #[must_use]
    pub fn methods(&self) -> impl ExactSizeIterator<Item = &MethodDefinition> {
        self.methods.iter()
    }

    /// Iterates through editable methods without permitting identity changes.
    pub fn methods_mut(&mut self) -> impl ExactSizeIterator<Item = &mut MethodDefinition> {
        self.methods.iter_mut()
    }

    /// Returns one exact method overload.
    #[must_use]
    pub fn method(&self, id: &MethodId) -> Option<&MethodDefinition> {
        self.method_index.get(id).map(|&index| &self.methods[index])
    }

    /// Returns one exact editable method overload.
    #[must_use]
    pub fn method_mut(&mut self, id: &MethodId) -> Option<&mut MethodDefinition> {
        let index = self.method_index.get(id).copied()?;
        self.methods.get_mut(index)
    }

    /// Adds a method while preserving declaration order and indexed lookup.
    ///
    /// # Errors
    ///
    /// Returns an error if this type already contains the same name/signature.
    pub fn insert_method(&mut self, method: MethodDefinition) -> Result<()> {
        let id = method.id.clone();
        if self.method_index.contains_key(&id) {
            return Err(self.duplicate_member(DefinitionKind::Method, &id.name, &id.signature));
        }
        self.method_index.insert(id, self.methods.len());
        self.methods.push(method);
        Ok(())
    }

    /// Returns the number of fields owned by this type.
    #[must_use]
    pub fn field_count(&self) -> usize {
        self.fields.len()
    }

    /// Returns the number of methods owned by this type.
    #[must_use]
    pub fn method_count(&self) -> usize {
        self.methods.len()
    }

    fn duplicate_member(&self, kind: DefinitionKind, name: &str, signature: &str) -> Error {
        Error::DuplicateDefinition {
            format: self.id.format,
            container: self.id.name.clone(),
            kind,
            name: name.to_owned(),
            signature: signature.to_owned(),
        }
    }
}

fn validate_member_id(kind: DefinitionKind, name: &str, signature: &str) -> Result<()> {
    let component = if name.is_empty() {
        Some(SymbolComponent::Name)
    } else if signature.is_empty() {
        Some(SymbolComponent::Signature)
    } else {
        None
    };
    match component {
        Some(component) => Err(Error::EmptySymbol { kind, component }),
        None => Ok(()),
    }
}

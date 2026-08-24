//! Module ownership, indexed lookup, and raw-disassembly conversion.

use std::collections::BTreeMap;

use disassembler::{Disassembly, RawAccessFlags};

use crate::{
    DefinitionKind, Error, MethodDefinition, MethodId, ModuleId, Result, SymbolComponent,
    TypeDefinition, TypeId,
};

/// One source artifact containing owned type and member definitions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Module {
    id: ModuleId,
    types: Vec<TypeDefinition>,
    type_index: BTreeMap<TypeId, usize>,
}

impl Module {
    /// Creates an empty module.
    ///
    /// # Errors
    ///
    /// Returns an error if the native module name is empty.
    pub fn new(id: ModuleId) -> Result<Self> {
        if id.name.is_empty() {
            return Err(Error::EmptySymbol {
                kind: DefinitionKind::Module,
                component: SymbolComponent::Module,
            });
        }
        Ok(Self {
            id,
            types: Vec::new(),
            type_index: BTreeMap::new(),
        })
    }

    /// Returns the module's format-qualified identity.
    #[must_use]
    pub const fn id(&self) -> &ModuleId {
        &self.id
    }

    /// Replaces the native module or artifact name.
    ///
    /// # Errors
    ///
    /// Returns an error if the replacement name is empty.
    pub fn set_name(&mut self, name: impl Into<String>) -> Result<()> {
        let name = name.into();
        if name.is_empty() {
            return Err(Error::EmptySymbol {
                kind: DefinitionKind::Module,
                component: SymbolComponent::Module,
            });
        }
        self.id.name = name;
        Ok(())
    }

    /// Iterates through top-level type definitions in source order.
    #[must_use]
    pub fn types(&self) -> impl ExactSizeIterator<Item = &TypeDefinition> {
        self.types.iter()
    }

    /// Iterates through editable types without permitting identity changes.
    pub fn types_mut(&mut self) -> impl ExactSizeIterator<Item = &mut TypeDefinition> {
        self.types.iter_mut()
    }

    /// Returns one exact type definition.
    #[must_use]
    pub fn type_definition(&self, id: &TypeId) -> Option<&TypeDefinition> {
        self.type_index.get(id).map(|&index| &self.types[index])
    }

    /// Returns one exact editable type definition.
    #[must_use]
    pub fn type_definition_mut(&mut self, id: &TypeId) -> Option<&mut TypeDefinition> {
        let index = self.type_index.get(id).copied()?;
        self.types.get_mut(index)
    }

    /// Adds a type while preserving source order and indexed lookup.
    ///
    /// # Errors
    ///
    /// Returns an error for a format mismatch or duplicate type identity.
    pub fn insert_type(&mut self, definition: TypeDefinition) -> Result<()> {
        let id = definition.id().clone();
        if id.format != self.id.format {
            return Err(Error::FormatMismatch {
                module: self.id.name.clone(),
                module_format: self.id.format,
                type_name: id.name,
                type_format: id.format,
            });
        }
        if self.type_index.contains_key(&id) {
            return Err(Error::DuplicateDefinition {
                format: self.id.format,
                container: self.id.name.clone(),
                kind: DefinitionKind::Type,
                name: id.name,
                signature: String::new(),
            });
        }
        self.type_index.insert(id, self.types.len());
        self.types.push(definition);
        Ok(())
    }

    /// Returns the number of types in this module.
    #[must_use]
    pub fn type_count(&self) -> usize {
        self.types.len()
    }

    /// Returns the number of fields across all module types.
    #[must_use]
    pub fn field_count(&self) -> usize {
        self.types.iter().map(TypeDefinition::field_count).sum()
    }

    /// Returns the number of methods across all module types.
    #[must_use]
    pub fn method_count(&self) -> usize {
        self.types.iter().map(TypeDefinition::method_count).sum()
    }
}

impl TryFrom<Disassembly> for Module {
    type Error = Error;

    fn try_from(disassembly: Disassembly) -> Result<Self> {
        let format = disassembly.format;
        let mut module = Self::new(ModuleId::new(format, disassembly.name))?;
        for function in disassembly.functions {
            let type_id = TypeId::new(format, function.symbol.owner);
            if module.type_definition(&type_id).is_none() {
                module.insert_type(TypeDefinition::new(
                    type_id.clone(),
                    RawAccessFlags::default(),
                )?)?;
            }
            let method = MethodDefinition::new(
                MethodId::new(function.symbol.name, function.symbol.signature),
                function.access_flags,
                function.body,
            )?;
            module
                .type_definition_mut(&type_id)
                .expect("type was inserted before its method")
                .insert_method(method)?;
        }
        Ok(module)
    }
}

impl TryFrom<&Disassembly> for Module {
    type Error = Error;

    fn try_from(disassembly: &Disassembly) -> Result<Self> {
        Self::try_from(disassembly.clone())
    }
}

//! Multi-module storage, traversal, and definition resolution.

use disassembler::Disassembly;

use crate::{MethodDefinition, MethodId, Module, Resolution, Result, TypeDefinition, TypeId};

/// Owned collection of modules exposed through one format-neutral interface.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Program {
    modules: Vec<Module>,
}

impl Program {
    /// Creates an empty program.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            modules: Vec::new(),
        }
    }

    /// Builds a program from already validated modules.
    #[must_use]
    pub fn from_modules(modules: impl IntoIterator<Item = Module>) -> Self {
        Self {
            modules: modules.into_iter().collect(),
        }
    }

    /// Converts raw disassemblies into modules in input order.
    ///
    /// # Errors
    ///
    /// Returns an error if any disassembly has an empty or duplicate native
    /// definition identity. No partial program is returned.
    pub fn from_disassemblies(
        disassemblies: impl IntoIterator<Item = Disassembly>,
    ) -> Result<Self> {
        disassemblies
            .into_iter()
            .map(Module::try_from)
            .collect::<Result<Vec<_>>>()
            .map(Self::from_modules)
    }

    /// Appends a validated module and returns its stable position.
    pub fn push_module(&mut self, module: Module) -> usize {
        let index = self.modules.len();
        self.modules.push(module);
        index
    }

    /// Converts and appends one raw disassembly atomically.
    ///
    /// # Errors
    ///
    /// Returns an error for invalid or duplicate identities within the source
    /// artifact. The program is unchanged on failure.
    pub fn add_disassembly(&mut self, disassembly: Disassembly) -> Result<usize> {
        let module = Module::try_from(disassembly)?;
        Ok(self.push_module(module))
    }

    /// Iterates through modules in insertion order.
    #[must_use]
    pub fn modules(&self) -> impl ExactSizeIterator<Item = &Module> {
        self.modules.iter()
    }

    /// Iterates through editable modules without permitting identity changes.
    pub fn modules_mut(&mut self) -> impl ExactSizeIterator<Item = &mut Module> {
        self.modules.iter_mut()
    }

    /// Iterates through every type in module and source order.
    pub fn types(&self) -> impl Iterator<Item = &TypeDefinition> {
        self.modules.iter().flat_map(Module::types)
    }

    /// Iterates through every method in module, type, and source order.
    pub fn methods(&self) -> impl Iterator<Item = &MethodDefinition> {
        self.types().flat_map(TypeDefinition::methods)
    }

    /// Resolves a type identity across every loaded module.
    #[must_use]
    pub fn resolve_type(&self, id: &TypeId) -> Resolution<&TypeDefinition> {
        resolve(
            self.modules
                .iter()
                .filter_map(|module| module.type_definition(id)),
        )
    }

    /// Resolves one exact method overload across every matching type.
    #[must_use]
    pub fn resolve_method(
        &self,
        owner: &TypeId,
        method: &MethodId,
    ) -> Resolution<&MethodDefinition> {
        resolve(
            self.modules
                .iter()
                .filter_map(|module| module.type_definition(owner))
                .filter_map(|definition| definition.method(method)),
        )
    }

    /// Returns the number of loaded modules.
    #[must_use]
    pub fn module_count(&self) -> usize {
        self.modules.len()
    }

    /// Returns the number of type definitions, including ambiguous duplicates.
    #[must_use]
    pub fn type_count(&self) -> usize {
        self.modules.iter().map(Module::type_count).sum()
    }

    /// Returns the number of field definitions.
    #[must_use]
    pub fn field_count(&self) -> usize {
        self.modules.iter().map(Module::field_count).sum()
    }

    /// Returns the number of method definitions.
    #[must_use]
    pub fn method_count(&self) -> usize {
        self.modules.iter().map(Module::method_count).sum()
    }
}

fn resolve<T>(mut matches: impl Iterator<Item = T>) -> Resolution<T> {
    let Some(first) = matches.next() else {
        return Resolution::Missing;
    };
    let additional = matches.count();
    if additional == 0 {
        Resolution::Unique(first)
    } else {
        Resolution::Ambiguous {
            matches: additional + 1,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::resolve;
    use crate::Resolution;

    #[test]
    fn resolves_without_collecting_matches() {
        assert_eq!(resolve(std::iter::empty::<u8>()), Resolution::Missing);
        assert_eq!(resolve([7].into_iter()), Resolution::Unique(7));
        assert_eq!(
            resolve([7, 8, 9].into_iter()),
            Resolution::Ambiguous { matches: 3 }
        );
    }
}

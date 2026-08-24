//! Stable symbolic interning and deterministic identifier-table construction.

use std::collections::BTreeMap;

use crate::{Error, Result};

use super::{
    DexContainer, DexFile, DexString, DexVersion, FieldId, FieldIndex, MethodId, MethodIndex,
    PrototypeId, PrototypeIndex, StringIndex, TypeId, TypeIndex,
};

macro_rules! handle_type {
    ($name:ident, $description:literal) => {
        #[doc = $description]
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub struct $name(u32);

        impl $name {
            fn position(self) -> Result<usize> {
                usize::try_from(self.0)
                    .map_err(|_| Error::invalid_assembly("symbolic handle does not fit platform"))
            }
        }
    };
}

handle_type!(
    StringHandle,
    "Stable handle for an interned exact DEX string."
);
handle_type!(
    TypeHandle,
    "Stable handle for an interned DEX type descriptor."
);
handle_type!(
    PrototypeHandle,
    "Stable handle for an interned method prototype."
);
handle_type!(
    FieldHandle,
    "Stable handle for an interned field identifier."
);
handle_type!(
    MethodIdHandle,
    "Stable handle for an interned method identifier."
);

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct SymbolicPrototype {
    return_type: TypeHandle,
    parameters: Vec<TypeHandle>,
    shorty: StringHandle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct SymbolicField {
    class: TypeHandle,
    field_type: TypeHandle,
    name: StringHandle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct SymbolicMethod {
    class: TypeHandle,
    prototype: PrototypeHandle,
    name: StringHandle,
}

/// Deterministic DEX identifier-table builder with stable insertion handles.
///
/// Values can be interned in any order. [`Self::build`] sorts all native tables
/// according to the DEX specification and returns a complete handle-to-index
/// map for class definitions, annotations, encoded values, and instructions.
#[derive(Debug, Clone)]
pub struct DexBuilder {
    version: DexVersion,
    strings: Vec<Vec<u16>>,
    string_handles: BTreeMap<Vec<u16>, StringHandle>,
    types: Vec<StringHandle>,
    type_handles: BTreeMap<StringHandle, TypeHandle>,
    prototypes: Vec<SymbolicPrototype>,
    prototype_handles: BTreeMap<SymbolicPrototype, PrototypeHandle>,
    fields: Vec<SymbolicField>,
    field_handles: BTreeMap<SymbolicField, FieldHandle>,
    methods: Vec<SymbolicMethod>,
    method_handles: BTreeMap<SymbolicMethod, MethodIdHandle>,
}

impl DexBuilder {
    /// Creates an empty builder for a supported DEX version.
    #[must_use]
    pub fn new(version: DexVersion) -> Self {
        Self {
            version,
            strings: Vec::new(),
            string_handles: BTreeMap::new(),
            types: Vec::new(),
            type_handles: BTreeMap::new(),
            prototypes: Vec::new(),
            prototype_handles: BTreeMap::new(),
            fields: Vec::new(),
            field_handles: BTreeMap::new(),
            methods: Vec::new(),
            method_handles: BTreeMap::new(),
        }
    }

    /// Returns the output DEX version.
    #[must_use]
    pub const fn version(&self) -> DexVersion {
        self.version
    }

    /// Interns valid Unicode text and returns a stable handle.
    ///
    /// # Errors
    ///
    /// Returns an error when the string interner exceeds 32-bit handles.
    pub fn intern_string(&mut self, value: &str) -> Result<StringHandle> {
        self.intern_utf16(value.encode_utf16().collect())
    }

    /// Interns exact Java UTF-16 units, including unpaired surrogates.
    ///
    /// # Errors
    ///
    /// Returns an error when the string interner exceeds 32-bit handles.
    pub fn intern_utf16(&mut self, units: Vec<u16>) -> Result<StringHandle> {
        if let Some(&handle) = self.string_handles.get(&units) {
            return Ok(handle);
        }
        let handle = StringHandle(next_handle(self.strings.len(), "string")?);
        self.strings.push(units.clone());
        self.string_handles.insert(units, handle);
        Ok(handle)
    }

    /// Interns a type descriptor from valid Unicode text.
    ///
    /// # Errors
    ///
    /// Returns an error when a required interner exceeds 32-bit handles.
    pub fn intern_type(&mut self, descriptor: &str) -> Result<TypeHandle> {
        let descriptor = self.intern_string(descriptor)?;
        self.intern_type_string(descriptor)
    }

    /// Interns a type descriptor already present in the string pool.
    ///
    /// # Errors
    ///
    /// Returns an error for a foreign string handle or a type interner beyond
    /// 32-bit handles.
    pub fn intern_type_string(&mut self, descriptor: StringHandle) -> Result<TypeHandle> {
        self.require_string(descriptor)?;
        if let Some(&handle) = self.type_handles.get(&descriptor) {
            return Ok(handle);
        }
        let handle = TypeHandle(next_handle(self.types.len(), "type")?);
        self.types.push(descriptor);
        self.type_handles.insert(descriptor, handle);
        Ok(handle)
    }

    /// Interns a method prototype and derives its exact shorty descriptor.
    ///
    /// # Errors
    ///
    /// Returns an error for a foreign handle, an empty descriptor, or table
    /// growth beyond 32-bit handles. Full descriptor-category validation runs
    /// during [`Self::build`].
    pub fn intern_prototype(
        &mut self,
        return_type: TypeHandle,
        parameters: impl IntoIterator<Item = TypeHandle>,
    ) -> Result<PrototypeHandle> {
        self.require_type(return_type)?;
        let parameters = parameters.into_iter().collect::<Vec<_>>();
        for &parameter in &parameters {
            self.require_type(parameter)?;
        }
        let mut shorty = vec![self.shorty_unit(return_type)?];
        for &parameter in &parameters {
            shorty.push(self.shorty_unit(parameter)?);
        }
        let shorty = self.intern_utf16(shorty)?;
        let value = SymbolicPrototype {
            return_type,
            parameters,
            shorty,
        };
        if let Some(&handle) = self.prototype_handles.get(&value) {
            return Ok(handle);
        }
        let handle = PrototypeHandle(next_handle(self.prototypes.len(), "prototype")?);
        self.prototypes.push(value.clone());
        self.prototype_handles.insert(value, handle);
        Ok(handle)
    }

    /// Interns a field from symbolic type and name handles.
    ///
    /// # Errors
    ///
    /// Returns an error for foreign handles or a field interner beyond 32-bit
    /// handles.
    pub fn intern_field(
        &mut self,
        class: TypeHandle,
        name: StringHandle,
        field_type: TypeHandle,
    ) -> Result<FieldHandle> {
        self.require_type(class)?;
        self.require_type(field_type)?;
        self.require_string(name)?;
        let value = SymbolicField {
            class,
            field_type,
            name,
        };
        if let Some(&handle) = self.field_handles.get(&value) {
            return Ok(handle);
        }
        let handle = FieldHandle(next_handle(self.fields.len(), "field")?);
        self.fields.push(value);
        self.field_handles.insert(value, handle);
        Ok(handle)
    }

    /// Interns a field and all of its textual dependencies.
    ///
    /// # Errors
    ///
    /// Returns an error for invalid dependencies or an interner beyond its
    /// 32-bit handle space.
    pub fn intern_field_named(
        &mut self,
        class_descriptor: &str,
        name: &str,
        field_descriptor: &str,
    ) -> Result<FieldHandle> {
        let class = self.intern_type(class_descriptor)?;
        let name = self.intern_string(name)?;
        let field_type = self.intern_type(field_descriptor)?;
        self.intern_field(class, name, field_type)
    }

    /// Interns a method from symbolic owner, name, and prototype handles.
    ///
    /// # Errors
    ///
    /// Returns an error for foreign handles or a method interner beyond 32-bit
    /// handles.
    pub fn intern_method(
        &mut self,
        class: TypeHandle,
        name: StringHandle,
        prototype: PrototypeHandle,
    ) -> Result<MethodIdHandle> {
        self.require_type(class)?;
        self.require_string(name)?;
        self.require_prototype(prototype)?;
        let value = SymbolicMethod {
            class,
            prototype,
            name,
        };
        if let Some(&handle) = self.method_handles.get(&value) {
            return Ok(handle);
        }
        let handle = MethodIdHandle(next_handle(self.methods.len(), "method")?);
        self.methods.push(value);
        self.method_handles.insert(value, handle);
        Ok(handle)
    }

    /// Interns a method and all of its textual and type dependencies.
    ///
    /// # Errors
    ///
    /// Returns an error for invalid dependencies or an interner beyond its
    /// 32-bit handle space.
    pub fn intern_method_named(
        &mut self,
        class_descriptor: &str,
        name: &str,
        return_descriptor: &str,
        parameter_descriptors: &[&str],
    ) -> Result<MethodIdHandle> {
        let class = self.intern_type(class_descriptor)?;
        let name = self.intern_string(name)?;
        let return_type = self.intern_type(return_descriptor)?;
        let parameters = parameter_descriptors
            .iter()
            .map(|descriptor| self.intern_type(descriptor))
            .collect::<Result<Vec<_>>>()?;
        let prototype = self.intern_prototype(return_type, parameters)?;
        self.intern_method(class, name, prototype)
    }

    /// Produces sorted native identifier tables and stable index mappings.
    ///
    /// # Errors
    ///
    /// Returns an error when a descriptor, member name, table limit, ordering
    /// rule, or version constraint is invalid.
    ///
    /// # Panics
    ///
    /// Panics only if this builder's private handle tables violate their own
    /// insertion invariants; callers cannot construct such a state.
    pub fn build(self) -> Result<BuiltDex> {
        let string_order = sorted_positions(&self.strings, Clone::clone);
        let string_indices = native_indices::<StringIndex>(self.strings.len(), &string_order)?;
        let mut file = DexFile::new(self.version);
        for &position in &string_order {
            file.push_string(DexString::from_utf16(self.strings[position].clone()))?;
        }

        let type_order = sorted_positions(&self.types, |descriptor| {
            string_indices[descriptor.position().expect("validated handle")].get()
        });
        let type_indices = native_indices::<TypeIndex>(self.types.len(), &type_order)?;
        for &position in &type_order {
            file.push_type(TypeId {
                descriptor: string_indices
                    [self.types[position].position().expect("validated handle")],
            })?;
        }

        let native_prototype = |prototype: &SymbolicPrototype| PrototypeId {
            shorty: string_indices[prototype.shorty.position().expect("validated handle")],
            return_type: type_indices[prototype.return_type.position().expect("validated handle")],
            parameters: prototype
                .parameters
                .iter()
                .map(|handle| type_indices[handle.position().expect("validated handle")])
                .collect(),
            parameters_offset: 0,
        };
        let prototype_order = sorted_positions(&self.prototypes, |value| {
            let native = native_prototype(value);
            (native.return_type, native.parameters)
        });
        let prototype_indices =
            native_indices::<PrototypeIndex>(self.prototypes.len(), &prototype_order)?;
        for &position in &prototype_order {
            file.push_prototype(native_prototype(&self.prototypes[position]))?;
        }

        let native_field = |value: &SymbolicField| FieldId {
            class: type_indices[value.class.position().expect("validated handle")],
            field_type: type_indices[value.field_type.position().expect("validated handle")],
            name: string_indices[value.name.position().expect("validated handle")],
        };
        let field_order = sorted_positions(&self.fields, |value| {
            let native = native_field(value);
            (native.class, native.name, native.field_type)
        });
        let field_indices = native_indices::<FieldIndex>(self.fields.len(), &field_order)?;
        for &position in &field_order {
            file.push_field(native_field(&self.fields[position]))?;
        }

        let native_method = |value: &SymbolicMethod| MethodId {
            class: type_indices[value.class.position().expect("validated handle")],
            prototype: prototype_indices[value.prototype.position().expect("validated handle")],
            name: string_indices[value.name.position().expect("validated handle")],
        };
        let method_order = sorted_positions(&self.methods, |value| {
            let native = native_method(value);
            (native.class, native.name, native.prototype)
        });
        let method_indices = native_indices::<MethodIndex>(self.methods.len(), &method_order)?;
        for &position in &method_order {
            file.push_method(native_method(&self.methods[position]))?;
        }

        if self.version == DexVersion::V041 {
            let mut container = DexContainer::new();
            container.push(file.clone())?;
            container.to_bytes()?;
        } else {
            file.to_bytes()?;
        }
        Ok(BuiltDex {
            file,
            indices: DexIndices {
                strings: string_indices,
                types: type_indices,
                prototypes: prototype_indices,
                fields: field_indices,
                methods: method_indices,
            },
        })
    }

    fn require_string(&self, handle: StringHandle) -> Result<()> {
        require_handle(handle.position()?, self.strings.len(), "string")
    }

    fn require_type(&self, handle: TypeHandle) -> Result<()> {
        require_handle(handle.position()?, self.types.len(), "type")
    }

    fn require_prototype(&self, handle: PrototypeHandle) -> Result<()> {
        require_handle(handle.position()?, self.prototypes.len(), "prototype")
    }

    fn shorty_unit(&self, handle: TypeHandle) -> Result<u16> {
        let descriptor_handle = *self
            .types
            .get(handle.position()?)
            .ok_or_else(|| Error::invalid_assembly("foreign type handle"))?;
        let descriptor = self
            .strings
            .get(descriptor_handle.position()?)
            .ok_or_else(|| Error::invalid_assembly("foreign descriptor string handle"))?;
        let first = *descriptor
            .first()
            .ok_or_else(|| Error::invalid_assembly("empty type descriptor"))?;
        Ok(if first == u16::from(b'[') || first == u16::from(b'L') {
            u16::from(b'L')
        } else {
            first
        })
    }
}

/// Result of symbolic table construction.
#[derive(Debug, Clone)]
pub struct BuiltDex {
    /// Editable DEX file containing all sorted identifiers.
    pub file: DexFile,
    /// Stable-handle to native-index mappings for subsequent model creation.
    pub indices: DexIndices,
}

/// Native indices assigned to every symbolic builder handle.
#[derive(Debug, Clone, Default)]
pub struct DexIndices {
    strings: Vec<StringIndex>,
    types: Vec<TypeIndex>,
    prototypes: Vec<PrototypeIndex>,
    fields: Vec<FieldIndex>,
    methods: Vec<MethodIndex>,
}

impl DexIndices {
    /// Resolves a string handle produced by the corresponding builder.
    #[must_use]
    pub fn string(&self, handle: StringHandle) -> Option<StringIndex> {
        handle
            .position()
            .ok()
            .and_then(|index| self.strings.get(index).copied())
    }

    /// Resolves a type handle produced by the corresponding builder.
    #[must_use]
    pub fn type_index(&self, handle: TypeHandle) -> Option<TypeIndex> {
        handle
            .position()
            .ok()
            .and_then(|index| self.types.get(index).copied())
    }

    /// Resolves a prototype handle produced by the corresponding builder.
    #[must_use]
    pub fn prototype(&self, handle: PrototypeHandle) -> Option<PrototypeIndex> {
        handle
            .position()
            .ok()
            .and_then(|index| self.prototypes.get(index).copied())
    }

    /// Resolves a field handle produced by the corresponding builder.
    #[must_use]
    pub fn field(&self, handle: FieldHandle) -> Option<FieldIndex> {
        handle
            .position()
            .ok()
            .and_then(|index| self.fields.get(index).copied())
    }

    /// Resolves a method handle produced by the corresponding builder.
    #[must_use]
    pub fn method(&self, handle: MethodIdHandle) -> Option<MethodIndex> {
        handle
            .position()
            .ok()
            .and_then(|index| self.methods.get(index).copied())
    }
}

trait NativeIndex: Copy {
    fn from_u32(value: u32) -> Self;
}

macro_rules! native_index {
    ($type:ty) => {
        impl NativeIndex for $type {
            fn from_u32(value: u32) -> Self {
                Self::new(value)
            }
        }
    };
}

native_index!(StringIndex);
native_index!(TypeIndex);
native_index!(PrototypeIndex);
native_index!(FieldIndex);
native_index!(MethodIndex);

fn native_indices<T: NativeIndex>(length: usize, order: &[usize]) -> Result<Vec<T>> {
    let mut output = vec![T::from_u32(0); length];
    for (native, &symbolic) in order.iter().enumerate() {
        output[symbolic] = T::from_u32(
            u32::try_from(native)
                .map_err(|_| Error::invalid_assembly("identifier table exceeds 32-bit indices"))?,
        );
    }
    Ok(output)
}

fn sorted_positions<T, K: Ord>(values: &[T], mut key: impl FnMut(&T) -> K) -> Vec<usize> {
    let mut positions = (0..values.len()).collect::<Vec<_>>();
    positions.sort_by_key(|&position| key(&values[position]));
    positions
}

fn require_handle(position: usize, length: usize, what: &str) -> Result<()> {
    if position < length {
        Ok(())
    } else {
        Err(Error::invalid_assembly(format!("foreign {what} handle")))
    }
}

fn next_handle(length: usize, what: &str) -> Result<u32> {
    u32::try_from(length)
        .map_err(|_| Error::invalid_assembly(format!("{what} interner exceeds 32-bit handles")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interns_duplicates_and_sorts_every_identifier_table() -> Result<()> {
        let mut builder = DexBuilder::new(DexVersion::V040);
        let first = builder.intern_string("zeta")?;
        let duplicate = builder.intern_string("zeta")?;
        assert_eq!(first, duplicate);
        builder.intern_string("alpha")?;

        let method = builder.intern_method_named(
            "Lsample/Owner;",
            "work",
            "Ljava/lang/Object;",
            &["I", "[Ljava/lang/String;"],
        )?;
        builder.intern_field_named("Lsample/Owner;", "value", "I")?;
        let built = builder.build()?;

        assert!(
            built
                .file
                .strings()
                .windows(2)
                .all(|pair| pair[0].utf16_units < pair[1].utf16_units)
        );
        assert!(built.indices.method(method).is_some());
        assert!(built.file.to_bytes().is_ok());
        Ok(())
    }

    #[test]
    fn version_041_builders_validate_through_a_container() -> Result<()> {
        let mut builder = DexBuilder::new(DexVersion::V041);
        builder.intern_type("Ljava/lang/Object;")?;
        let built = builder.build()?;
        let mut container = DexContainer::new();
        container.push(built.file)?;
        assert_eq!(
            DexContainer::parse(&container.to_bytes()?)?.members().len(),
            1
        );
        Ok(())
    }
}

//! DEX container header, identifier tables, definitions, and binary round trips.

mod header;
mod integrity;
mod io;
mod layout;
mod model;
mod mutf8;
mod parse;
mod resolve;
mod validation;
mod write;

pub use self::header::{DexHeader, DexVersion, Endian, Section};
pub use self::layout::ItemWidth;
pub use self::model::*;
pub use self::resolve::{
    ResolvedField, ResolvedMethod, ResolvedMethodHandle, ResolvedMethodHandleTarget,
};

use self::header::LEGACY_HEADER_OFFSET;
use crate::{Error, Result};

/// Parsed and editable logical DEX file.
#[derive(Debug, Clone)]
pub struct DexFile {
    header: DexHeader,
    strings: Vec<DexString>,
    types: Vec<TypeId>,
    prototypes: Vec<PrototypeId>,
    fields: Vec<FieldId>,
    methods: Vec<MethodId>,
    classes: Vec<ClassDefinition>,
    call_sites: Vec<CallSite>,
    method_handles: Vec<MethodHandle>,
    map: Vec<MapItem>,
    link_data: Vec<u8>,
    hidden_api: Option<HiddenApiClassData>,
    original: Option<Vec<u8>>,
    dirty: bool,
}

impl DexFile {
    /// Creates an empty editable DEX model using the requested format version.
    ///
    /// Versions 035 through 040 can be assembled directly. Version 041 is a
    /// container-member format and therefore requires a future container
    /// assembler before a newly created value can be serialized.
    #[must_use]
    pub fn new(version: DexVersion) -> Self {
        Self {
            header: DexHeader::empty(version),
            strings: Vec::new(),
            types: Vec::new(),
            prototypes: Vec::new(),
            fields: Vec::new(),
            methods: Vec::new(),
            classes: Vec::new(),
            call_sites: Vec::new(),
            method_handles: Vec::new(),
            map: Vec::new(),
            link_data: Vec::new(),
            hidden_api: None,
            original: None,
            dirty: true,
        }
    }

    /// Parses one logical DEX file from memory.
    ///
    /// Version 041 container members are addressed by the header offset stored
    /// in their own header; use the container parser for a multi-header buffer.
    ///
    /// # Errors
    ///
    /// Returns an error for malformed magic, integrity fields, offsets, tables,
    /// variable-length data, instructions, or cross references.
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        parse::parse(bytes, LEGACY_HEADER_OFFSET)
    }

    /// Returns the parsed header and exact original section coordinates.
    #[must_use]
    pub const fn header(&self) -> &DexHeader {
        &self.header
    }

    /// Returns the DEX format version.
    #[must_use]
    pub const fn version(&self) -> DexVersion {
        self.header.version
    }

    /// Selects the DEX version used by the next assembly.
    pub const fn set_version(&mut self, version: DexVersion) {
        self.header.version = version;
        self.header.header_size = version.header_size();
        self.dirty = true;
    }

    /// Selects the byte order used by the next assembly.
    pub const fn set_endian(&mut self, endian: Endian) {
        self.header.endian = endian;
        self.dirty = true;
    }

    /// Returns strings in native `string_id` order.
    #[must_use]
    pub fn strings(&self) -> &[DexString] {
        &self.strings
    }

    /// Returns the editable native string table and marks the file as edited.
    pub fn strings_mut(&mut self) -> &mut Vec<DexString> {
        self.dirty = true;
        &mut self.strings
    }

    /// Appends a string and returns its native table index.
    ///
    /// DEX tables are ordered. Assembly rejects a value appended out of
    /// order, allowing callers to perform several related edits before the
    /// complete model is checked.
    ///
    /// # Errors
    ///
    /// Returns an error when the resulting index would exceed 32 bits.
    pub fn push_string(&mut self, value: DexString) -> Result<StringIndex> {
        let index = next_index(self.strings.len(), "string")?;
        self.strings.push(value);
        self.dirty = true;
        Ok(StringIndex::new(index))
    }

    /// Returns types in native `type_id` order.
    #[must_use]
    pub fn types(&self) -> &[TypeId] {
        &self.types
    }

    /// Returns the editable native type table and marks the file as edited.
    pub fn types_mut(&mut self) -> &mut Vec<TypeId> {
        self.dirty = true;
        &mut self.types
    }

    /// Appends a type and returns its native table index.
    ///
    /// # Errors
    ///
    /// Returns an error when the resulting index would exceed 32 bits.
    pub fn push_type(&mut self, value: TypeId) -> Result<TypeIndex> {
        let index = next_index(self.types.len(), "type")?;
        self.types.push(value);
        self.dirty = true;
        Ok(TypeIndex::new(index))
    }

    /// Returns method prototypes in native `proto_id` order.
    #[must_use]
    pub fn prototypes(&self) -> &[PrototypeId] {
        &self.prototypes
    }

    /// Returns the editable native prototype table and marks the file as edited.
    pub fn prototypes_mut(&mut self) -> &mut Vec<PrototypeId> {
        self.dirty = true;
        &mut self.prototypes
    }

    /// Appends a method prototype and returns its native table index.
    ///
    /// # Errors
    ///
    /// Returns an error when the resulting index would exceed 32 bits.
    pub fn push_prototype(&mut self, value: PrototypeId) -> Result<PrototypeIndex> {
        let index = next_index(self.prototypes.len(), "prototype")?;
        self.prototypes.push(value);
        self.dirty = true;
        Ok(PrototypeIndex::new(index))
    }

    /// Returns field identifiers in native `field_id` order.
    #[must_use]
    pub fn fields(&self) -> &[FieldId] {
        &self.fields
    }

    /// Returns the editable native field table and marks the file as edited.
    pub fn fields_mut(&mut self) -> &mut Vec<FieldId> {
        self.dirty = true;
        &mut self.fields
    }

    /// Appends a field identifier and returns its native table index.
    ///
    /// # Errors
    ///
    /// Returns an error when the resulting index would exceed 32 bits.
    pub fn push_field(&mut self, value: FieldId) -> Result<FieldIndex> {
        let index = next_index(self.fields.len(), "field")?;
        self.fields.push(value);
        self.dirty = true;
        Ok(FieldIndex::new(index))
    }

    /// Returns method identifiers in native `method_id` order.
    #[must_use]
    pub fn methods(&self) -> &[MethodId] {
        &self.methods
    }

    /// Returns the editable native method table and marks the file as edited.
    pub fn methods_mut(&mut self) -> &mut Vec<MethodId> {
        self.dirty = true;
        &mut self.methods
    }

    /// Appends a method identifier and returns its native table index.
    ///
    /// # Errors
    ///
    /// Returns an error when the resulting index would exceed 32 bits.
    pub fn push_method(&mut self, value: MethodId) -> Result<MethodIndex> {
        let index = next_index(self.methods.len(), "method")?;
        self.methods.push(value);
        self.dirty = true;
        Ok(MethodIndex::new(index))
    }

    /// Returns class definitions in native dependency order.
    #[must_use]
    pub fn classes(&self) -> &[ClassDefinition] {
        &self.classes
    }

    /// Returns mutable class definitions and marks the file as edited.
    pub fn classes_mut(&mut self) -> &mut Vec<ClassDefinition> {
        self.dirty = true;
        &mut self.classes
    }

    /// Appends a class definition and assigns its native definition index.
    ///
    /// # Errors
    ///
    /// Returns an error when the resulting index would exceed 32 bits.
    pub fn push_class(&mut self, mut value: ClassDefinition) -> Result<u32> {
        let index = next_index(self.classes.len(), "class definition")?;
        value.definition_index = index;
        self.classes.push(value);
        self.dirty = true;
        Ok(index)
    }

    /// Returns call-site definitions in native order.
    #[must_use]
    pub fn call_sites(&self) -> &[CallSite] {
        &self.call_sites
    }

    /// Returns the editable native call-site table and marks the file as edited.
    pub fn call_sites_mut(&mut self) -> &mut Vec<CallSite> {
        self.dirty = true;
        &mut self.call_sites
    }

    /// Appends a call site and returns its native table index.
    ///
    /// # Errors
    ///
    /// Returns an error when the resulting index would exceed 32 bits.
    pub fn push_call_site(&mut self, value: CallSite) -> Result<CallSiteIndex> {
        let index = next_index(self.call_sites.len(), "call site")?;
        self.call_sites.push(value);
        self.dirty = true;
        Ok(CallSiteIndex::new(index))
    }

    /// Returns method handles in native order.
    #[must_use]
    pub fn method_handles(&self) -> &[MethodHandle] {
        &self.method_handles
    }

    /// Returns the editable native method-handle table and marks the file as edited.
    pub fn method_handles_mut(&mut self) -> &mut Vec<MethodHandle> {
        self.dirty = true;
        &mut self.method_handles
    }

    /// Appends a method handle and returns its native table index.
    ///
    /// # Errors
    ///
    /// Returns an error when the resulting index would exceed 32 bits.
    pub fn push_method_handle(&mut self, value: MethodHandle) -> Result<MethodHandleIndex> {
        let index = next_index(self.method_handles.len(), "method handle")?;
        self.method_handles.push(value);
        self.dirty = true;
        Ok(MethodHandleIndex::new(index))
    }

    /// Returns the parsed map list in file-offset order.
    #[must_use]
    pub fn map(&self) -> &[MapItem] {
        &self.map
    }

    /// Returns opaque static-link data exactly as encoded.
    #[must_use]
    pub fn link_data(&self) -> &[u8] {
        &self.link_data
    }

    /// Returns editable opaque static-link data and marks the file as edited.
    pub fn link_data_mut(&mut self) -> &mut Vec<u8> {
        self.dirty = true;
        &mut self.link_data
    }

    /// Returns hidden-API restriction flags for boot-class-path DEX files.
    #[must_use]
    pub const fn hidden_api(&self) -> Option<&HiddenApiClassData> {
        self.hidden_api.as_ref()
    }

    /// Replaces hidden-API restriction metadata.
    pub fn set_hidden_api(&mut self, value: Option<HiddenApiClassData>) {
        self.hidden_api = value;
        self.dirty = true;
    }

    /// Returns editable hidden-API metadata and marks the file as edited.
    pub fn hidden_api_mut(&mut self) -> Option<&mut HiddenApiClassData> {
        self.dirty = true;
        self.hidden_api.as_mut()
    }

    /// Returns whether no mutating API has been used since parsing.
    #[must_use]
    pub const fn is_pristine(&self) -> bool {
        !self.dirty
    }

    /// Returns the exact parsed logical-file bytes while pristine.
    #[must_use]
    pub fn original_bytes(&self) -> Option<&[u8]> {
        (!self.dirty).then_some(self.original.as_deref()).flatten()
    }

    /// Assembles this logical DEX file and recomputes SHA-1 and Adler-32 fields.
    ///
    /// An unchanged parsed file is returned byte-for-byte without normalization.
    ///
    /// # Errors
    ///
    /// Returns an error when an edited model violates table ordering, index,
    /// size, alignment, instruction, or encoded-value constraints.
    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        if !self.dirty
            && let Some(original) = &self.original
        {
            return Ok(original.clone());
        }
        write::assemble(self)
    }

    /// Applies a checked edit transaction, restoring the previous file on any
    /// closure or assembly-validation failure.
    ///
    /// # Errors
    ///
    /// Returns the closure error or the complete assembly validation error.
    pub fn try_edit<T>(&mut self, edit: impl FnOnce(&mut Self) -> Result<T>) -> Result<T> {
        let previous = self.clone();
        let result = edit(self).and_then(|value| self.to_bytes().map(|_| value));
        if result.is_err() {
            *self = previous;
        }
        result
    }
}

fn next_index(length: usize, what: &str) -> Result<u32> {
    u32::try_from(length)
        .map_err(|_| Error::invalid_assembly(format!("{what} table exceeds 32-bit indices")))
}

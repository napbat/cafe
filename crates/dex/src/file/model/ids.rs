//! Native DEX identifier-table indices and entries.

macro_rules! index_type {
    ($name:ident, $description:literal) => {
        #[doc = $description]
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub struct $name(pub u32);

        impl $name {
            /// Creates an index from its native table value.
            #[must_use]
            pub const fn new(value: u32) -> Self {
                Self(value)
            }

            /// Returns the native table value.
            #[must_use]
            pub const fn get(self) -> u32 {
                self.0
            }
        }
    };
}

index_type!(StringIndex, "Index into the DEX string identifier table.");
index_type!(TypeIndex, "Index into the DEX type identifier table.");
index_type!(
    PrototypeIndex,
    "Index into the DEX prototype identifier table."
);
index_type!(FieldIndex, "Index into the DEX field identifier table.");
index_type!(MethodIndex, "Index into the DEX method identifier table.");
index_type!(
    CallSiteIndex,
    "Index into the DEX call-site identifier table."
);
index_type!(MethodHandleIndex, "Index into the DEX method-handle table.");

/// DEX modified UTF-8 string with exact Java UTF-16 content retained.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DexString {
    /// Lossy Rust string view; unpaired surrogates appear as U+FFFD.
    pub text: String,
    /// Exact UTF-16 code units described by the encoded size prefix.
    pub utf16_units: Vec<u16>,
    /// Original absolute `string_data_item` offset, if parsed from a file.
    pub data_offset: Option<u32>,
}

impl DexString {
    /// Creates a DEX string from valid Unicode text.
    #[must_use]
    pub fn new(value: &str) -> Self {
        Self {
            text: value.to_owned(),
            utf16_units: value.encode_utf16().collect(),
            data_offset: None,
        }
    }

    /// Creates a DEX string from exact Java UTF-16 code units.
    #[must_use]
    pub fn from_utf16(utf16_units: Vec<u16>) -> Self {
        Self {
            text: String::from_utf16_lossy(&utf16_units),
            utf16_units,
            data_offset: None,
        }
    }

    /// Returns whether every surrogate belongs to a valid pair.
    #[must_use]
    pub fn is_valid_unicode(&self) -> bool {
        char::decode_utf16(self.utf16_units.iter().copied()).all(|value| value.is_ok())
    }
}

/// One type identifier referencing a descriptor string.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TypeId {
    /// Descriptor string index.
    pub descriptor: StringIndex,
}

/// One method-prototype identifier.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrototypeId {
    /// Short-form descriptor string index.
    pub shorty: StringIndex,
    /// Return type index.
    pub return_type: TypeIndex,
    /// Parameter types in declaration order.
    pub parameters: Vec<TypeIndex>,
    /// Original absolute `type_list` offset, or zero when absent.
    pub parameters_offset: u32,
}

/// One field identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FieldId {
    /// Defining class type.
    pub class: TypeIndex,
    /// Field type.
    pub field_type: TypeIndex,
    /// Field name string.
    pub name: StringIndex,
}

/// One method identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MethodId {
    /// Defining class type.
    pub class: TypeIndex,
    /// Method prototype.
    pub prototype: PrototypeIndex,
    /// Method name string.
    pub name: StringIndex,
}

/// DEX method-handle behavior.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u16)]
pub enum MethodHandleKind {
    /// Write a static field.
    StaticPut = 0,
    /// Read a static field.
    StaticGet = 1,
    /// Write an instance field.
    InstancePut = 2,
    /// Read an instance field.
    InstanceGet = 3,
    /// Invoke a static method.
    InvokeStatic = 4,
    /// Invoke a virtual instance method.
    InvokeInstance = 5,
    /// Invoke a constructor.
    InvokeConstructor = 6,
    /// Invoke a direct method.
    InvokeDirect = 7,
    /// Invoke an interface method.
    InvokeInterface = 8,
}

impl MethodHandleKind {
    /// Parses the encoded `method_handle_type` value.
    #[must_use]
    pub const fn from_u16(value: u16) -> Option<Self> {
        match value {
            0 => Some(Self::StaticPut),
            1 => Some(Self::StaticGet),
            2 => Some(Self::InstancePut),
            3 => Some(Self::InstanceGet),
            4 => Some(Self::InvokeStatic),
            5 => Some(Self::InvokeInstance),
            6 => Some(Self::InvokeConstructor),
            7 => Some(Self::InvokeDirect),
            8 => Some(Self::InvokeInterface),
            _ => None,
        }
    }

    /// Returns whether the handle references the field table.
    #[must_use]
    pub const fn references_field(self) -> bool {
        matches!(
            self,
            Self::StaticPut | Self::StaticGet | Self::InstancePut | Self::InstanceGet
        )
    }

    /// Returns the exact encoded method-handle type.
    #[must_use]
    pub const fn as_u16(self) -> u16 {
        self as u16
    }
}

/// One method handle retaining its native target index.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MethodHandle {
    /// Handle behavior and target-table choice.
    pub kind: MethodHandleKind,
    /// Field or method index selected by `kind`.
    pub target_index: u16,
}

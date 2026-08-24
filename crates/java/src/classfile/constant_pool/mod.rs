//! JVM constant-pool types, parsing, validation, and display helpers.

mod intern;

use std::fmt::Write;

use crate::{Error, Result};

use super::io::Reader;
use super::modified_utf8;

/// Reserved constant-pool index that never contains a usable constant.
pub const RESERVED_CONSTANT_POOL_INDEX: u16 = 0;
/// First constant-pool index that can contain a usable constant.
pub const FIRST_USABLE_CONSTANT_POOL_INDEX: u16 = RESERVED_CONSTANT_POOL_INDEX + 1;

const MIN_CONSTANT_POOL_COUNT: u16 = 1;
const INITIAL_REFERENCE_DEPTH: u8 = 0;
const REFERENCE_DEPTH_STEP: u8 = 1;
const MAX_REFERENCE_DISPLAY_DEPTH: u8 = 8;
const UNAVAILABLE_SOURCE_OFFSET: usize = 0;

macro_rules! define_encoded_u8_enum {
    (
        $(#[$enum_metadata:meta])*
        $name:ident {
            $($(#[$variant_metadata:meta])* $variant:ident = $value:expr => $display_name:literal),+ $(,)?
        }
    ) => {
        $(#[$enum_metadata])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
        #[repr(u8)]
        pub enum $name {
            $(
                $(#[$variant_metadata])*
                $variant = $value,
            )+
        }

        impl $name {
            /// Every standardized encoded value, ordered by its byte encoding.
            pub const ALL: &[Self] = &[$(Self::$variant),+];

            /// Converts an encoded byte to a known value.
            #[must_use]
            pub const fn from_byte(value: u8) -> Option<Self> {
                match value {
                    $($value => Some(Self::$variant),)+
                    _ => None,
                }
            }

            /// Returns the class-file byte encoding.
            #[must_use]
            pub const fn byte(self) -> u8 {
                self as u8
            }

            /// Returns the JVM specification name.
            #[must_use]
            pub const fn name(self) -> &'static str {
                match self {
                    $(Self::$variant => $display_name),+
                }
            }
        }
    };
}

define_encoded_u8_enum! {
    /// Standardized constant-pool tags.
    ConstantTag {
        /// A modified UTF-8 string.
        Utf8 = 1 => "Utf8",
        /// A signed integer.
        Integer = 3 => "Integer",
        /// A single-precision float.
        Float = 4 => "Float",
        /// A signed long.
        Long = 5 => "Long",
        /// A double-precision float.
        Double = 6 => "Double",
        /// A class or interface.
        Class = 7 => "Class",
        /// A string constant.
        String = 8 => "String",
        /// A field reference.
        FieldRef = 9 => "Fieldref",
        /// A class method reference.
        MethodRef = 10 => "Methodref",
        /// An interface method reference.
        InterfaceMethodRef = 11 => "InterfaceMethodref",
        /// A name and descriptor pair.
        NameAndType = 12 => "NameAndType",
        /// A method handle.
        MethodHandle = 15 => "MethodHandle",
        /// A method type.
        MethodType = 16 => "MethodType",
        /// A dynamically computed constant.
        Dynamic = 17 => "Dynamic",
        /// A dynamically selected call site.
        InvokeDynamic = 18 => "InvokeDynamic",
        /// A module.
        Module = 19 => "Module",
        /// A package.
        Package = 20 => "Package",
    }
}

define_encoded_u8_enum! {
    /// Standardized `CONSTANT_MethodHandle` reference kinds.
    MethodHandleKind {
        /// Read an instance field.
        GetField = 1 => "getField",
        /// Read a static field.
        GetStatic = 2 => "getStatic",
        /// Write an instance field.
        PutField = 3 => "putField",
        /// Write a static field.
        PutStatic = 4 => "putStatic",
        /// Invoke a virtual method.
        InvokeVirtual = 5 => "invokeVirtual",
        /// Invoke a static method.
        InvokeStatic = 6 => "invokeStatic",
        /// Invoke a special method.
        InvokeSpecial = 7 => "invokeSpecial",
        /// Invoke a constructor.
        NewInvokeSpecial = 8 => "newInvokeSpecial",
        /// Invoke an interface method.
        InvokeInterface = 9 => "invokeInterface",
    }
}

/// Number of constant-pool slots occupied by one encoded constant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ConstantSlotWidth {
    /// Every constant except a `long` or `double`.
    Single,
    /// A `long` or `double` plus its reserved following slot.
    Double,
}

impl ConstantSlotWidth {
    /// Returns the number of occupied constant-pool slots.
    #[must_use]
    pub const fn slot_count(self) -> usize {
        match self {
            Self::Single => 1,
            Self::Double => 2,
        }
    }
}

/// A constant-pool slot.
#[derive(Debug, Clone, PartialEq)]
pub enum Constant {
    /// Index zero or the second slot occupied by a `long` or `double`.
    Unusable,
    /// A modified UTF-8 string.
    Utf8(Utf8Constant),
    /// A signed Java integer.
    Integer(i32),
    /// An IEEE-754 single-precision value.
    Float(f32),
    /// A signed Java long.
    Long(i64),
    /// An IEEE-754 double-precision value.
    Double(f64),
    /// A class or interface symbolic reference.
    Class {
        /// Index of a UTF-8 internal class name or array descriptor.
        name_index: u16,
    },
    /// A string constant.
    String {
        /// Index of the UTF-8 string contents.
        string_index: u16,
    },
    /// A field symbolic reference.
    FieldRef {
        /// Index of the declaring class.
        class_index: u16,
        /// Index of the field name and descriptor.
        name_and_type_index: u16,
    },
    /// A class method symbolic reference.
    MethodRef {
        /// Index of the declaring class.
        class_index: u16,
        /// Index of the method name and descriptor.
        name_and_type_index: u16,
    },
    /// An interface method symbolic reference.
    InterfaceMethodRef {
        /// Index of the declaring interface.
        class_index: u16,
        /// Index of the method name and descriptor.
        name_and_type_index: u16,
    },
    /// A name paired with a field or method descriptor.
    NameAndType {
        /// Index of the UTF-8 name.
        name_index: u16,
        /// Index of the UTF-8 descriptor.
        descriptor_index: u16,
    },
    /// A method-handle constant.
    MethodHandle {
        /// JVM reference kind.
        reference_kind: MethodHandleKind,
        /// Index of a compatible field or method reference.
        reference_index: u16,
    },
    /// A method-type constant.
    MethodType {
        /// Index of a UTF-8 method descriptor.
        descriptor_index: u16,
    },
    /// A dynamically computed constant.
    Dynamic {
        /// Index into the class `BootstrapMethods` attribute.
        bootstrap_method_attr_index: u16,
        /// Index of the constant name and descriptor.
        name_and_type_index: u16,
    },
    /// A dynamically selected call site.
    InvokeDynamic {
        /// Index into the class `BootstrapMethods` attribute.
        bootstrap_method_attr_index: u16,
        /// Index of the call-site name and descriptor.
        name_and_type_index: u16,
    },
    /// A module name.
    Module {
        /// Index of the UTF-8 module name.
        name_index: u16,
    },
    /// A package name.
    Package {
        /// Index of the UTF-8 package name.
        name_index: u16,
    },
}

impl Constant {
    /// Returns the number of constant-pool slots occupied by this constant.
    #[must_use]
    pub const fn slot_width(&self) -> ConstantSlotWidth {
        match self {
            Self::Long(_) | Self::Double(_) => ConstantSlotWidth::Double,
            _ => ConstantSlotWidth::Single,
        }
    }

    /// Returns this constant's standardized tag, or `None` for an unusable slot.
    #[must_use]
    pub const fn tag(&self) -> Option<ConstantTag> {
        let tag = match self {
            Self::Unusable => return None,
            Self::Utf8(_) => ConstantTag::Utf8,
            Self::Integer(_) => ConstantTag::Integer,
            Self::Float(_) => ConstantTag::Float,
            Self::Long(_) => ConstantTag::Long,
            Self::Double(_) => ConstantTag::Double,
            Self::Class { .. } => ConstantTag::Class,
            Self::String { .. } => ConstantTag::String,
            Self::FieldRef { .. } => ConstantTag::FieldRef,
            Self::MethodRef { .. } => ConstantTag::MethodRef,
            Self::InterfaceMethodRef { .. } => ConstantTag::InterfaceMethodRef,
            Self::NameAndType { .. } => ConstantTag::NameAndType,
            Self::MethodHandle { .. } => ConstantTag::MethodHandle,
            Self::MethodType { .. } => ConstantTag::MethodType,
            Self::Dynamic { .. } => ConstantTag::Dynamic,
            Self::InvokeDynamic { .. } => ConstantTag::InvokeDynamic,
            Self::Module { .. } => ConstantTag::Module,
            Self::Package { .. } => ConstantTag::Package,
        };
        Some(tag)
    }

    /// Returns the JVM name of this constant-pool tag.
    #[must_use]
    pub const fn tag_name(&self) -> &'static str {
        match self.tag() {
            Some(tag) => tag.name(),
            None => "Unusable",
        }
    }
}

/// The UTF-16 value of a class-file modified UTF-8 constant.
///
/// The ordinary Rust string view is convenient for names and descriptors. The
/// exact UTF-16 units are retained because Java strings can contain unpaired
/// surrogates, which a Rust [`String`] cannot represent directly.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Utf8Constant {
    text: String,
    utf16_units: Vec<u16>,
}

impl Utf8Constant {
    /// Creates a modified UTF-8 constant from a Rust string.
    #[must_use]
    pub fn new(value: &str) -> Self {
        Self {
            text: value.to_owned(),
            utf16_units: value.encode_utf16().collect(),
        }
    }

    /// Creates a constant from exact Java UTF-16 code units.
    #[must_use]
    pub fn from_utf16(units: Vec<u16>) -> Self {
        Self {
            text: String::from_utf16_lossy(&units),
            utf16_units: units,
        }
    }

    /// Returns a Rust string view, replacing any unpaired surrogate with U+FFFD.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.text
    }

    /// Returns the exact UTF-16 code units encoded in the class file.
    #[must_use]
    pub fn utf16_units(&self) -> &[u16] {
        &self.utf16_units
    }

    /// Returns whether every surrogate belongs to a valid pair.
    #[must_use]
    pub fn is_valid_unicode(&self) -> bool {
        char::decode_utf16(self.utf16_units.iter().copied()).all(|value| value.is_ok())
    }
}

impl From<&str> for Utf8Constant {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

/// A class-file constant pool with JVM indices preserved.
#[derive(Debug, Clone)]
pub struct ConstantPool {
    entries: Vec<Constant>,
}

impl ConstantPool {
    /// Creates an empty constant pool containing only the reserved zero slot.
    #[must_use]
    pub fn new() -> Self {
        Self {
            entries: vec![Constant::Unusable],
        }
    }

    /// Appends a constant and returns its JVM index.
    ///
    /// Longs and doubles automatically reserve their required second slot.
    ///
    /// # Errors
    ///
    /// Returns an error for `Constant::Unusable` or if the pool would exceed the
    /// `u16` class-file slot limit.
    pub fn push(&mut self, constant: Constant) -> Result<u16> {
        if matches!(constant, Constant::Unusable) {
            return Err(Error::invalid_assembly(
                "unusable constant-pool slots are inserted automatically",
            ));
        }
        let slot_width = constant.slot_width();
        let added_slots = slot_width.slot_count();
        let new_len = self
            .entries
            .len()
            .checked_add(added_slots)
            .ok_or_else(|| Error::invalid_assembly("constant-pool size overflow"))?;
        if new_len > usize::from(u16::MAX) {
            return Err(Error::invalid_assembly(format!(
                "constant pool exceeds {} slots",
                u16::MAX
            )));
        }
        let index = u16::try_from(self.entries.len())
            .map_err(|_| Error::invalid_assembly("constant-pool index does not fit u16"))?;
        self.entries.push(constant);
        if slot_width == ConstantSlotWidth::Double {
            self.entries.push(Constant::Unusable);
        }
        Ok(index)
    }

    /// Appends a Rust string as a modified UTF-8 constant.
    ///
    /// # Errors
    ///
    /// Returns an error if the constant pool has no remaining slot.
    pub fn push_utf8(&mut self, value: &str) -> Result<u16> {
        self.push(Constant::Utf8(Utf8Constant::new(value)))
    }

    /// Replaces a constant without changing the pool's slot layout.
    ///
    /// # Errors
    ///
    /// Returns an error for index zero, an unusable slot, an explicit
    /// [`Constant::Unusable`], or a replacement with a different slot width.
    pub fn replace(&mut self, index: u16, constant: Constant) -> Result<Constant> {
        if matches!(constant, Constant::Unusable) {
            return Err(Error::invalid_assembly(
                "cannot explicitly place an unusable constant-pool slot",
            ));
        }
        let slot = self
            .entries
            .get_mut(usize::from(index))
            .ok_or_else(|| Error::invalid_assembly(format!("constant index #{index} is absent")))?;
        if matches!(slot, Constant::Unusable) {
            return Err(Error::invalid_assembly(format!(
                "constant index #{index} is an unusable slot"
            )));
        }
        if slot.slot_width() != constant.slot_width() {
            return Err(Error::invalid_assembly(format!(
                "replacing constant #{index} would change its slot width"
            )));
        }
        Ok(std::mem::replace(slot, constant))
    }

    pub(crate) fn parse(reader: &mut Reader<'_>) -> Result<Self> {
        let count_offset = reader.absolute_position();
        let count = reader.read_u16()?;
        if count < MIN_CONSTANT_POOL_COUNT {
            return Err(Error::invalid_class(
                count_offset,
                "constant_pool_count must be at least one",
            ));
        }

        let mut entries = Vec::with_capacity(usize::from(count));
        entries.push(Constant::Unusable);

        while entries.len() < usize::from(count) {
            let tag_offset = reader.absolute_position();
            let tag_byte = reader.read_u8()?;
            let tag = ConstantTag::from_byte(tag_byte).ok_or_else(|| {
                Error::invalid_class(tag_offset, format!("unknown constant-pool tag {tag_byte}"))
            })?;
            let constant = match tag {
                ConstantTag::Utf8 => {
                    let length = usize::from(reader.read_u16()?);
                    let string_offset = reader.absolute_position();
                    let bytes = reader.read_bytes(length)?;
                    let decoded = modified_utf8::decode(bytes, string_offset)?;
                    Constant::Utf8(Utf8Constant {
                        text: decoded.text,
                        utf16_units: decoded.units,
                    })
                }
                ConstantTag::Integer => Constant::Integer(reader.read_u32()?.cast_signed()),
                ConstantTag::Float => Constant::Float(f32::from_bits(reader.read_u32()?)),
                ConstantTag::Long => Constant::Long(reader.read_u64()?.cast_signed()),
                ConstantTag::Double => Constant::Double(f64::from_bits(reader.read_u64()?)),
                ConstantTag::Class => Constant::Class {
                    name_index: reader.read_u16()?,
                },
                ConstantTag::String => Constant::String {
                    string_index: reader.read_u16()?,
                },
                ConstantTag::FieldRef => Constant::FieldRef {
                    class_index: reader.read_u16()?,
                    name_and_type_index: reader.read_u16()?,
                },
                ConstantTag::MethodRef => Constant::MethodRef {
                    class_index: reader.read_u16()?,
                    name_and_type_index: reader.read_u16()?,
                },
                ConstantTag::InterfaceMethodRef => Constant::InterfaceMethodRef {
                    class_index: reader.read_u16()?,
                    name_and_type_index: reader.read_u16()?,
                },
                ConstantTag::NameAndType => Constant::NameAndType {
                    name_index: reader.read_u16()?,
                    descriptor_index: reader.read_u16()?,
                },
                ConstantTag::MethodHandle => {
                    let kind_offset = reader.absolute_position();
                    let kind_byte = reader.read_u8()?;
                    let reference_kind =
                        MethodHandleKind::from_byte(kind_byte).ok_or_else(|| {
                            Error::invalid_class(
                                kind_offset,
                                format!("invalid method-handle reference kind {kind_byte}"),
                            )
                        })?;
                    Constant::MethodHandle {
                        reference_kind,
                        reference_index: reader.read_u16()?,
                    }
                }
                ConstantTag::MethodType => Constant::MethodType {
                    descriptor_index: reader.read_u16()?,
                },
                ConstantTag::Dynamic => Constant::Dynamic {
                    bootstrap_method_attr_index: reader.read_u16()?,
                    name_and_type_index: reader.read_u16()?,
                },
                ConstantTag::InvokeDynamic => Constant::InvokeDynamic {
                    bootstrap_method_attr_index: reader.read_u16()?,
                    name_and_type_index: reader.read_u16()?,
                },
                ConstantTag::Module => Constant::Module {
                    name_index: reader.read_u16()?,
                },
                ConstantTag::Package => Constant::Package {
                    name_index: reader.read_u16()?,
                },
            };

            let slot_width = constant.slot_width();
            entries.push(constant);
            if slot_width == ConstantSlotWidth::Double {
                if entries.len() >= usize::from(count) {
                    return Err(Error::invalid_class(
                        tag_offset,
                        "long or double occupies a missing second constant-pool slot",
                    ));
                }
                entries.push(Constant::Unusable);
            }
        }

        Ok(Self { entries })
    }

    /// Returns the total slot count, including index zero and unusable slots.
    #[must_use]
    pub fn slot_count(&self) -> usize {
        self.entries.len()
    }

    /// Iterates through usable constants in index order.
    pub fn iter(&self) -> impl Iterator<Item = (u16, &Constant)> {
        self.entries
            .iter()
            .enumerate()
            .filter_map(|(index, value)| {
                if matches!(value, Constant::Unusable) {
                    None
                } else {
                    u16::try_from(index).ok().map(|index| (index, value))
                }
            })
    }

    /// Returns the constant at an index.
    ///
    /// # Errors
    ///
    /// Returns an error if the index is out of range or denotes an unusable slot.
    pub fn get(&self, index: u16) -> Result<&Constant> {
        match self.entries.get(usize::from(index)) {
            Some(Constant::Unusable) | None => Err(Error::invalid_class(
                UNAVAILABLE_SOURCE_OFFSET,
                format!("invalid or unusable constant-pool index #{index}"),
            )),
            Some(constant) => Ok(constant),
        }
    }

    /// Returns a UTF-8 constant as a string slice.
    ///
    /// # Errors
    ///
    /// Returns an error if the index does not refer to a UTF-8 constant.
    pub fn utf8(&self, index: u16) -> Result<&str> {
        match self.get(index)? {
            Constant::Utf8(value) => Ok(value.as_str()),
            constant => Err(Self::wrong_tag(index, "Utf8", constant)),
        }
    }

    /// Returns the exact modified UTF-8 constant at an index.
    ///
    /// # Errors
    ///
    /// Returns an error if the index does not refer to a UTF-8 constant.
    pub fn utf8_constant(&self, index: u16) -> Result<&Utf8Constant> {
        match self.get(index)? {
            Constant::Utf8(value) => Ok(value),
            constant => Err(Self::wrong_tag(index, "Utf8", constant)),
        }
    }

    /// Returns a mutable exact modified UTF-8 constant at an index.
    ///
    /// Mutating this value cannot disturb the constant-pool slot layout.
    ///
    /// # Errors
    ///
    /// Returns an error if the index does not refer to a UTF-8 constant.
    pub fn utf8_constant_mut(&mut self, index: u16) -> Result<&mut Utf8Constant> {
        match self.entries.get_mut(usize::from(index)) {
            Some(Constant::Utf8(value)) => Ok(value),
            Some(constant) => Err(Self::wrong_tag(index, "Utf8", constant)),
            None => Err(Error::invalid_class(
                UNAVAILABLE_SOURCE_OFFSET,
                format!("invalid constant-pool index #{index}"),
            )),
        }
    }

    /// Resolves a `CONSTANT_Class` to its internal JVM name.
    ///
    /// # Errors
    ///
    /// Returns an error if either referenced constant has the wrong tag.
    pub fn class_name(&self, index: u16) -> Result<&str> {
        match self.get(index)? {
            Constant::Class { name_index } => self.utf8(*name_index),
            constant => Err(Self::wrong_tag(index, "Class", constant)),
        }
    }

    /// Resolves a `CONSTANT_NameAndType` into its two UTF-8 strings.
    ///
    /// # Errors
    ///
    /// Returns an error if the index or either of its references has the wrong tag.
    pub fn name_and_type(&self, index: u16) -> Result<(&str, &str)> {
        match self.get(index)? {
            Constant::NameAndType {
                name_index,
                descriptor_index,
            } => Ok((self.utf8(*name_index)?, self.utf8(*descriptor_index)?)),
            constant => Err(Self::wrong_tag(index, "NameAndType", constant)),
        }
    }

    /// Produces a resolved, human-readable representation of a constant.
    ///
    /// # Errors
    ///
    /// Returns an error if the constant or one of its referenced constants is
    /// missing, unusable, or has the wrong tag.
    pub fn describe(&self, index: u16) -> Result<String> {
        self.describe_inner(index, INITIAL_REFERENCE_DEPTH)
    }

    fn describe_inner(&self, index: u16, depth: u8) -> Result<String> {
        if depth > MAX_REFERENCE_DISPLAY_DEPTH {
            return Ok(format!("#{index} <recursive reference>"));
        }
        let description = match self.get(index)? {
            Constant::Unusable => unreachable!("get rejects unusable slots"),
            Constant::Utf8(value) => format!("Utf8 {}", quote_utf8(value)),
            Constant::Integer(value) => format!("int {value}"),
            Constant::Float(value) => format!("float {value:?}"),
            Constant::Long(value) => format!("long {value}"),
            Constant::Double(value) => format!("double {value:?}"),
            Constant::Class { name_index } => format!("Class {}", self.utf8(*name_index)?),
            Constant::String { string_index } => match self.get(*string_index)? {
                Constant::Utf8(value) => format!("String {}", quote_utf8(value)),
                constant => return Err(Self::wrong_tag(*string_index, "Utf8", constant)),
            },
            Constant::FieldRef {
                class_index,
                name_and_type_index,
            } => format!(
                "Field {}.{}",
                self.class_name(*class_index)?,
                self.describe_name_and_type(*name_and_type_index)?
            ),
            Constant::MethodRef {
                class_index,
                name_and_type_index,
            } => format!(
                "Method {}.{}",
                self.class_name(*class_index)?,
                self.describe_name_and_type(*name_and_type_index)?
            ),
            Constant::InterfaceMethodRef {
                class_index,
                name_and_type_index,
            } => format!(
                "InterfaceMethod {}.{}",
                self.class_name(*class_index)?,
                self.describe_name_and_type(*name_and_type_index)?
            ),
            Constant::NameAndType { .. } => {
                format!("NameAndType {}", self.describe_name_and_type(index)?)
            }
            Constant::MethodHandle {
                reference_kind,
                reference_index,
            } => format!(
                "MethodHandle {} #{} ({})",
                reference_kind.name(),
                reference_index,
                self.describe_inner(*reference_index, depth + REFERENCE_DEPTH_STEP)?
            ),
            Constant::MethodType { descriptor_index } => {
                format!("MethodType {}", self.utf8(*descriptor_index)?)
            }
            Constant::Dynamic {
                bootstrap_method_attr_index,
                name_and_type_index,
            } => format!(
                "Dynamic bootstrap#{bootstrap_method_attr_index}:{}",
                self.describe_name_and_type(*name_and_type_index)?
            ),
            Constant::InvokeDynamic {
                bootstrap_method_attr_index,
                name_and_type_index,
            } => format!(
                "InvokeDynamic bootstrap#{bootstrap_method_attr_index}:{}",
                self.describe_name_and_type(*name_and_type_index)?
            ),
            Constant::Module { name_index } => format!("Module {}", self.utf8(*name_index)?),
            Constant::Package { name_index } => format!("Package {}", self.utf8(*name_index)?),
        };
        Ok(description)
    }

    fn describe_name_and_type(&self, index: u16) -> Result<String> {
        let (name, descriptor) = self.name_and_type(index)?;
        Ok(format!("{name}:{descriptor}"))
    }

    fn wrong_tag(index: u16, expected: &str, actual: &Constant) -> Error {
        Error::invalid_class(
            UNAVAILABLE_SOURCE_OFFSET,
            format!(
                "constant-pool index #{index} is {}, expected {expected}",
                actual.tag_name()
            ),
        )
    }

    pub(crate) fn expect_class(&self, index: u16) -> Result<()> {
        self.class_name(index).map(|_| ())
    }

    pub(crate) fn expect_utf8(&self, index: u16) -> Result<()> {
        self.utf8(index).map(|_| ())
    }

    fn expect_name_and_type(&self, index: u16) -> Result<()> {
        self.name_and_type(index).map(|_| ())
    }

    pub(crate) fn validate(&self) -> Result<()> {
        for (_, constant) in self.iter() {
            match constant {
                Constant::Unusable
                | Constant::Utf8(_)
                | Constant::Integer(_)
                | Constant::Float(_)
                | Constant::Long(_)
                | Constant::Double(_) => {}
                Constant::Class { name_index }
                | Constant::Module { name_index }
                | Constant::Package { name_index } => self.expect_utf8(*name_index)?,
                Constant::String { string_index } => self.expect_utf8(*string_index)?,
                Constant::FieldRef {
                    class_index,
                    name_and_type_index,
                }
                | Constant::MethodRef {
                    class_index,
                    name_and_type_index,
                }
                | Constant::InterfaceMethodRef {
                    class_index,
                    name_and_type_index,
                } => {
                    self.expect_class(*class_index)?;
                    self.expect_name_and_type(*name_and_type_index)?;
                }
                Constant::NameAndType {
                    name_index,
                    descriptor_index,
                } => {
                    self.expect_utf8(*name_index)?;
                    self.expect_utf8(*descriptor_index)?;
                }
                Constant::MethodHandle {
                    reference_index, ..
                } => {
                    self.get(*reference_index)?;
                }
                Constant::MethodType { descriptor_index } => {
                    self.expect_utf8(*descriptor_index)?;
                }
                Constant::Dynamic {
                    name_and_type_index,
                    ..
                }
                | Constant::InvokeDynamic {
                    name_and_type_index,
                    ..
                } => self.expect_name_and_type(*name_and_type_index)?,
            }
        }
        Ok(())
    }

    pub(crate) fn raw_entries(&self) -> &[Constant] {
        &self.entries
    }
}

impl Default for ConstantPool {
    fn default() -> Self {
        Self::new()
    }
}

fn quote_utf8(value: &Utf8Constant) -> String {
    let mut quoted = String::from("\"");
    for character in char::decode_utf16(value.utf16_units.iter().copied()) {
        match character {
            Ok(character) => quoted.extend(character.escape_default()),
            Err(error) => {
                write!(quoted, "\\u{:04x}", error.unpaired_surrogate())
                    .expect("writing to a String cannot fail");
            }
        }
    }
    quoted.push('"');
    quoted
}

//! Data structures and parser for the JVM `ClassFile` format.

mod access_flags;
mod assemble;
mod constant_pool;
mod io;
mod modified_utf8;
mod parser;
pub mod version;

pub use self::access_flags::{ClassAccessFlags, FieldAccessFlags, MethodAccessFlags};
pub use self::assemble::assemble_class;
pub use self::constant_pool::{
    Constant, ConstantPool, ConstantSlotWidth, ConstantTag, FIRST_USABLE_CONSTANT_POOL_INDEX,
    MethodHandleKind, RESERVED_CONSTANT_POOL_INDEX, Utf8Constant,
};
pub use self::version::JavaRelease;

use crate::Result;

/// The magic number at the start of every JVM class file.
pub const CLASS_MAGIC: u32 = 0xcafe_babe;
/// The constant-pool sentinel used when a class has no superclass.
pub const NO_SUPER_CLASS_INDEX: u16 = RESERVED_CONSTANT_POOL_INDEX;
/// The exception-table sentinel denoting a handler that catches every exception.
pub const CATCH_ALL_EXCEPTION_INDEX: u16 = RESERVED_CONSTANT_POOL_INDEX;
/// Maximum byte length of one JVM method's code array.
pub const MAX_CODE_LENGTH: usize = u16::MAX as usize;
/// Standard name of a method's executable-code attribute.
pub const CODE_ATTRIBUTE_NAME: &str = "Code";

const CLASS_MAGIC_OFFSET: usize = 0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AttributeLocation {
    Class,
    Field,
    Method,
    Code,
}

impl AttributeLocation {
    const fn allows_code(self) -> bool {
        matches!(self, Self::Method)
    }
}

/// A parsed JVM class file.
#[derive(Debug, Clone)]
pub struct ClassFile {
    /// Class-file minor version.
    pub minor_version: u16,
    /// Class-file major version.
    pub major_version: u16,
    /// Constant pool used by this class.
    pub constant_pool: ConstantPool,
    /// Access flags for the class declaration.
    pub access_flags: ClassAccessFlags,
    /// Constant-pool index of this class.
    pub this_class: u16,
    /// Constant-pool index of the superclass, or zero for `java/lang/Object`.
    pub super_class: u16,
    /// Constant-pool class indices for directly implemented interfaces.
    pub interfaces: Vec<u16>,
    /// Declared fields.
    pub fields: Vec<FieldInfo>,
    /// Declared methods.
    pub methods: Vec<MethodInfo>,
    /// Class-level attributes not interpreted by the parser.
    pub attributes: Vec<Attribute>,
}

impl ClassFile {
    /// Parses one complete class file from memory.
    ///
    /// # Errors
    ///
    /// Returns an error if the data is truncated, has a bad magic number,
    /// contains an unsupported constant tag, or violates structural references.
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        parser::parse_class(bytes)
    }

    /// Returns the internal JVM name of this class, such as `java/lang/String`.
    ///
    /// # Errors
    ///
    /// Returns an error if `this_class` does not resolve to a valid class name.
    pub fn class_name(&self) -> Result<&str> {
        self.constant_pool.class_name(self.this_class)
    }

    /// Returns the internal JVM name of the superclass, if there is one.
    ///
    /// # Errors
    ///
    /// Returns an error if `super_class` does not resolve to a valid class name.
    pub fn super_name(&self) -> Result<Option<&str>> {
        if self.super_class == NO_SUPER_CLASS_INDEX {
            Ok(None)
        } else {
            self.constant_pool.class_name(self.super_class).map(Some)
        }
    }

    /// Maps the class-file major version to the corresponding Java release.
    #[must_use]
    pub const fn java_release(&self) -> Option<u16> {
        match self.java_version() {
            Some(release) => Some(release.number()),
            _ => None,
        }
    }

    /// Returns the typed Java release represented by this class-file version.
    #[must_use]
    pub const fn java_version(&self) -> Option<JavaRelease> {
        JavaRelease::from_class_major(self.major_version)
    }

    /// Assembles this structured class file into JVM binary form.
    ///
    /// # Errors
    ///
    /// Returns an error if a count or payload exceeds a class-file limit or the
    /// structured constant pool contains invalid slots.
    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        assemble_class(self)
    }
}

/// A field declaration.
#[derive(Debug, Clone)]
pub struct FieldInfo {
    /// Field access flags.
    pub access_flags: FieldAccessFlags,
    /// Constant-pool index of the field name.
    pub name_index: u16,
    /// Constant-pool index of the field descriptor.
    pub descriptor_index: u16,
    /// Field attributes.
    pub attributes: Vec<Attribute>,
}

impl FieldInfo {
    /// Resolves the field name.
    ///
    /// # Errors
    ///
    /// Returns an error if `name_index` is not a UTF-8 constant.
    pub fn name<'a>(&self, pool: &'a ConstantPool) -> Result<&'a str> {
        pool.utf8(self.name_index)
    }

    /// Resolves the field descriptor.
    ///
    /// # Errors
    ///
    /// Returns an error if `descriptor_index` is not a UTF-8 constant.
    pub fn descriptor<'a>(&self, pool: &'a ConstantPool) -> Result<&'a str> {
        pool.utf8(self.descriptor_index)
    }
}

/// A method declaration.
#[derive(Debug, Clone)]
pub struct MethodInfo {
    /// Method access flags.
    pub access_flags: MethodAccessFlags,
    /// Constant-pool index of the method name.
    pub name_index: u16,
    /// Constant-pool index of the method descriptor.
    pub descriptor_index: u16,
    /// Method attributes, with executable code parsed into a structured form.
    pub attributes: Vec<Attribute>,
}

impl MethodInfo {
    /// Resolves the method name.
    ///
    /// # Errors
    ///
    /// Returns an error if `name_index` is not a UTF-8 constant.
    pub fn name<'a>(&self, pool: &'a ConstantPool) -> Result<&'a str> {
        pool.utf8(self.name_index)
    }

    /// Resolves the method descriptor.
    ///
    /// # Errors
    ///
    /// Returns an error if `descriptor_index` is not a UTF-8 constant.
    pub fn descriptor<'a>(&self, pool: &'a ConstantPool) -> Result<&'a str> {
        pool.utf8(self.descriptor_index)
    }

    /// Returns this method's code attribute, if present.
    #[must_use]
    pub fn code(&self) -> Option<&CodeAttribute> {
        self.attributes
            .iter()
            .find_map(|attribute| match attribute {
                Attribute::Code(code) => Some(code),
                Attribute::Raw(_) => None,
            })
    }

    /// Returns this method's mutable code attribute, if present.
    #[must_use]
    pub fn code_mut(&mut self) -> Option<&mut CodeAttribute> {
        self.attributes
            .iter_mut()
            .find_map(|attribute| match attribute {
                Attribute::Code(code) => Some(code),
                Attribute::Raw(_) => None,
            })
    }
}

/// A parsed or raw class-file attribute.
#[derive(Debug, Clone)]
pub enum Attribute {
    /// A method's JVM `Code` attribute.
    Code(CodeAttribute),
    /// Any attribute that the disassembler does not need to interpret.
    Raw(RawAttribute),
}

impl Attribute {
    /// Returns the attribute name.
    #[must_use]
    pub fn name(&self) -> &str {
        match self {
            Self::Code(_) => CODE_ATTRIBUTE_NAME,
            Self::Raw(attribute) => &attribute.name,
        }
    }
}

/// An uninterpreted class-file attribute.
#[derive(Debug, Clone)]
pub struct RawAttribute {
    /// Constant-pool index of the attribute name.
    pub name_index: u16,
    /// Resolved attribute name.
    pub name: String,
    /// Exact attribute payload bytes.
    pub info: Vec<u8>,
}

/// The executable body of a JVM method.
#[derive(Debug, Clone)]
pub struct CodeAttribute {
    /// Constant-pool index of the `Code` attribute name.
    pub name_index: u16,
    /// Maximum operand-stack depth required by the method.
    pub max_stack: u16,
    /// Number of local-variable slots used by the method.
    pub max_locals: u16,
    /// Raw JVM bytecode.
    pub code: Vec<u8>,
    /// Protected regions and handlers.
    pub exception_table: Vec<ExceptionHandler>,
    /// Nested code attributes such as line numbers and stack maps.
    pub attributes: Vec<RawAttribute>,
}

impl CodeAttribute {
    /// Decodes this attribute into typed instructions.
    ///
    /// # Errors
    ///
    /// Returns an error if the bytecode or exception-table boundaries are invalid.
    pub fn instructions(&self) -> Result<Vec<crate::bytecode::Instruction>> {
        crate::bytecode::decode_code(self)
    }

    /// Replaces this method body by assembling typed instructions.
    ///
    /// The old bytecode is retained if the new code invalidates an existing
    /// exception-table boundary.
    ///
    /// # Errors
    ///
    /// Returns an error if the instructions cannot be encoded or the resulting
    /// method body is structurally invalid.
    pub fn set_instructions(
        &mut self,
        instructions: &[crate::bytecode::Instruction],
    ) -> Result<()> {
        let new_code = crate::bytecode::encode(instructions)?;
        let old_code = std::mem::replace(&mut self.code, new_code);
        if let Err(error) = crate::bytecode::decode_code(self) {
            self.code = old_code;
            return Err(error);
        }
        Ok(())
    }
}

/// One entry in a `Code` attribute exception table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExceptionHandler {
    /// Inclusive bytecode start offset.
    pub start_pc: u16,
    /// Exclusive bytecode end offset.
    pub end_pc: u16,
    /// Bytecode offset of the handler.
    pub handler_pc: u16,
    /// Constant-pool class index of the caught exception, or zero for all.
    pub catch_type: u16,
}

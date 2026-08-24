//! Owned symbols produced by resolving DEX identifier operands and values.

use crate::file::MethodHandleKind;

/// Exact Java string content with a convenient lossy Unicode view.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ExactString {
    /// Lossy Rust string view; unpaired surrogates appear as U+FFFD.
    pub text: String,
    /// Exact Java UTF-16 code units.
    pub utf16_units: Vec<u16>,
}

/// Resolved DEX type descriptor.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TypeSymbol {
    /// Exact field-type descriptor text.
    pub descriptor: String,
}

/// Resolved overload-qualified field identity.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FieldSymbol {
    /// Declaring class descriptor.
    pub owner: String,
    /// Exact field name.
    pub name: ExactString,
    /// Field-type descriptor.
    pub descriptor: String,
}

/// Resolved overload-qualified method identity.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct MethodSymbol {
    /// Declaring class descriptor.
    pub owner: String,
    /// Exact method name.
    pub name: ExactString,
    /// JVM-compatible method descriptor.
    pub descriptor: String,
}

/// Resolved DEX prototype identity.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PrototypeSymbol {
    /// JVM-compatible method descriptor.
    pub descriptor: String,
}

/// Resolved field or method target of a DEX method handle.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum MethodHandleTargetSymbol {
    /// Field selected by a get or put handle.
    Field(FieldSymbol),
    /// Method selected by an invocation handle.
    Method(MethodSymbol),
}

/// Resolved DEX method handle.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct MethodHandleSymbol {
    /// Handle behavior.
    pub kind: MethodHandleKind,
    /// Resolved field or method target.
    pub target: MethodHandleTargetSymbol,
}

/// One recursively resolved annotation element.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnnotationElementSymbol {
    /// Exact element name.
    pub name: ExactString,
    /// Resolved element value.
    pub value: ResolvedValue,
}

/// One recursively resolved DEX annotation value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnnotationSymbol {
    /// Annotation class descriptor.
    pub descriptor: String,
    /// Elements in encoded order.
    pub elements: Vec<AnnotationElementSymbol>,
}

/// Recursively resolved DEX encoded value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolvedValue {
    /// Signed 8-bit integer.
    Byte(i8),
    /// Signed 16-bit integer.
    Short(i16),
    /// Unsigned UTF-16 code unit.
    Char(u16),
    /// Signed 32-bit integer.
    Int(i32),
    /// Signed 64-bit integer.
    Long(i64),
    /// IEEE-754 single-precision bits.
    Float(u32),
    /// IEEE-754 double-precision bits.
    Double(u64),
    /// Method prototype.
    MethodType(PrototypeSymbol),
    /// Method handle.
    MethodHandle(MethodHandleSymbol),
    /// Exact string.
    String(ExactString),
    /// Type descriptor.
    Type(TypeSymbol),
    /// Field identity.
    Field(FieldSymbol),
    /// Method identity.
    Method(MethodSymbol),
    /// Enum constant represented by its field identity.
    Enum(FieldSymbol),
    /// Nested array.
    Array(Vec<ResolvedValue>),
    /// Nested annotation.
    Annotation(AnnotationSymbol),
    /// Null reference.
    Null,
    /// Boolean value.
    Boolean(bool),
}

/// Fully resolved bootstrap call-site definition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallSiteSymbol {
    /// Bootstrap method handle.
    pub bootstrap_method: MethodHandleSymbol,
    /// Exact dynamic method name.
    pub method_name: ExactString,
    /// JVM-compatible dynamic method descriptor.
    pub descriptor: String,
    /// Additional resolved bootstrap arguments.
    pub arguments: Vec<ResolvedValue>,
}

/// Resolved value selected by an indexed instruction operand.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstructionReference {
    /// String-table value.
    String(ExactString),
    /// Type descriptor.
    Type(TypeSymbol),
    /// Field identity.
    Field(FieldSymbol),
    /// Method identity.
    Method(MethodSymbol),
    /// Method prototype.
    Prototype(PrototypeSymbol),
    /// Bootstrap call site.
    CallSite(CallSiteSymbol),
    /// Method handle.
    MethodHandle(MethodHandleSymbol),
}

/// Primary and optional polymorphic-prototype references of one instruction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstructionReferences {
    /// Value selected by the opcode's primary index, if indexed.
    pub primary: Option<InstructionReference>,
    /// Secondary prototype used by polymorphic invocation, if present.
    pub secondary_prototype: Option<PrototypeSymbol>,
}

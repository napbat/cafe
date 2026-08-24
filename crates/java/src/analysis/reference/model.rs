//! Owned JVM constant-pool symbols with exact Java string content.

use crate::classfile::MethodHandleKind;

/// Exact class-file modified UTF-8 content and its lossy text view.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ExactString {
    /// Lossy Rust string view; unpaired surrogates appear as U+FFFD.
    pub text: String,
    /// Exact Java UTF-16 code units.
    pub utf16_units: Vec<u16>,
}

/// Resolved class constant.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ClassSymbol {
    /// Internal class name or array descriptor.
    pub name: ExactString,
}

/// Resolved JVM field reference.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FieldSymbol {
    /// Declaring class or interface.
    pub owner: ClassSymbol,
    /// Exact field name.
    pub name: ExactString,
    /// JVM field descriptor.
    pub descriptor: String,
}

/// Kind of JVM method constant selected by an instruction or handle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MethodReferenceKind {
    /// `CONSTANT_Methodref`.
    Class,
    /// `CONSTANT_InterfaceMethodref`.
    Interface,
}

/// Resolved JVM method reference.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct MethodSymbol {
    /// Declaring class or interface.
    pub owner: ClassSymbol,
    /// Exact method name.
    pub name: ExactString,
    /// JVM method descriptor.
    pub descriptor: String,
    /// Constant-pool method-reference category.
    pub kind: MethodReferenceKind,
}

/// Resolved target of a JVM method-handle constant.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum MethodHandleTargetSymbol {
    /// Field reference.
    Field(FieldSymbol),
    /// Class or interface method reference.
    Method(MethodSymbol),
}

/// Resolved JVM method-handle constant.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct MethodHandleSymbol {
    /// JVM handle behavior.
    pub kind: MethodHandleKind,
    /// Resolved member target.
    pub target: MethodHandleTargetSymbol,
}

/// Resolved dynamic constant or call-site name/type pair.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DynamicSymbol {
    /// Index into the class-level bootstrap-method table.
    pub bootstrap_method: u16,
    /// Exact constant or call-site name.
    pub name: ExactString,
    /// Field descriptor for a dynamic constant or method descriptor for a call site.
    pub descriptor: String,
}

/// Constant value loadable by `ldc`, `ldc_w`, or `ldc2_w`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum LoadableConstant {
    /// Signed JVM integer.
    Integer(i32),
    /// IEEE-754 single-precision bits.
    Float(u32),
    /// Signed JVM long.
    Long(i64),
    /// IEEE-754 double-precision bits.
    Double(u64),
    /// Exact Java string contents.
    String(ExactString),
    /// Class or array literal.
    Class(ClassSymbol),
    /// Method-type descriptor.
    MethodType(String),
    /// Method handle and member target.
    MethodHandle(MethodHandleSymbol),
    /// Dynamically computed constant.
    Dynamic(DynamicSymbol),
}

/// Resolved symbolic reference carried by one JVM instruction.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum InstructionReference {
    /// Loadable constant.
    Constant(LoadableConstant),
    /// Field member.
    Field(FieldSymbol),
    /// Class or interface method member.
    Method(MethodSymbol),
    /// Class, interface, or array type.
    Class(ClassSymbol),
    /// Dynamically linked call site.
    DynamicCallSite(DynamicSymbol),
}

//! Models for fixed-shape and table-shaped standard attributes.

use super::super::{Attribute, InnerClassAccessFlags, MethodParameterAccessFlags};

/// A standard attribute containing one constant-pool index.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IndexAttribute {
    /// Constant-pool index of the attribute name.
    pub name_index: u16,
    /// Attribute-specific referenced constant-pool index.
    pub index: u16,
}

/// A standard attribute containing a counted list of constant-pool indices.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexListAttribute {
    /// Constant-pool index of the attribute name.
    pub name_index: u16,
    /// Attribute-specific referenced constant-pool indices.
    pub indices: Vec<u16>,
}

/// A zero-length marker attribute such as `Synthetic` or `Deprecated`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MarkerAttribute {
    /// Constant-pool index of the attribute name.
    pub name_index: u16,
}

/// An attribute whose payload is an uninterpreted byte string by definition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BytesAttribute {
    /// Constant-pool index of the attribute name.
    pub name_index: u16,
    /// Exact standard-attribute byte content.
    pub bytes: Vec<u8>,
}

/// Typed `InnerClasses` attribute.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InnerClassesAttribute {
    /// Constant-pool index of `InnerClasses`.
    pub name_index: u16,
    /// Ordered inner-class descriptors.
    pub classes: Vec<InnerClass>,
}

/// One entry in an `InnerClasses` attribute.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InnerClass {
    /// Class index of the nested class.
    pub inner_class_info_index: u16,
    /// Class index of the enclosing class, or zero when unavailable.
    pub outer_class_info_index: u16,
    /// UTF-8 index of the source-level simple name, or zero for anonymous types.
    pub inner_name_index: u16,
    /// Access flags declared for the nested class.
    pub access_flags: InnerClassAccessFlags,
}

/// Typed `EnclosingMethod` attribute.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EnclosingMethodAttribute {
    /// Constant-pool index of `EnclosingMethod`.
    pub name_index: u16,
    /// Class index of the immediately enclosing type.
    pub class_index: u16,
    /// Name-and-type index of the enclosing method, or zero when not enclosed
    /// by a method or constructor.
    pub method_index: u16,
}

/// Typed `LineNumberTable` attribute.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineNumberTableAttribute {
    /// Constant-pool index of `LineNumberTable`.
    pub name_index: u16,
    /// Ordered source-line mappings.
    pub lines: Vec<LineNumber>,
}

/// One source-line mapping for a bytecode range.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LineNumber {
    /// Bytecode offset at which this line becomes active.
    pub start_pc: u16,
    /// Source-file line number.
    pub line_number: u16,
}

/// Typed `LocalVariableTable` attribute.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalVariableTableAttribute {
    /// Constant-pool index of `LocalVariableTable`.
    pub name_index: u16,
    /// Local-variable scopes.
    pub variables: Vec<LocalVariable>,
}

/// One local-variable name, descriptor, slot, and live range.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LocalVariable {
    /// Inclusive bytecode start offset.
    pub start_pc: u16,
    /// Byte length of the variable's live range.
    pub length: u16,
    /// UTF-8 constant-pool index of the variable name.
    pub name_index: u16,
    /// UTF-8 constant-pool index of the field descriptor.
    pub descriptor_index: u16,
    /// Local-variable array slot.
    pub slot: u16,
}

/// Typed `LocalVariableTypeTable` attribute.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalVariableTypeTableAttribute {
    /// Constant-pool index of `LocalVariableTypeTable`.
    pub name_index: u16,
    /// Generic local-variable scopes.
    pub variables: Vec<LocalVariableType>,
}

/// One generic local-variable signature and live range.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LocalVariableType {
    /// Inclusive bytecode start offset.
    pub start_pc: u16,
    /// Byte length of the variable's live range.
    pub length: u16,
    /// UTF-8 constant-pool index of the variable name.
    pub name_index: u16,
    /// UTF-8 constant-pool index of the generic signature.
    pub signature_index: u16,
    /// Local-variable array slot.
    pub slot: u16,
}

/// Typed `BootstrapMethods` attribute.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BootstrapMethodsAttribute {
    /// Constant-pool index of `BootstrapMethods`.
    pub name_index: u16,
    /// Ordered bootstrap method specifiers.
    pub methods: Vec<BootstrapMethod>,
}

/// One bootstrap method handle and its static arguments.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BootstrapMethod {
    /// Constant-pool index of a method-handle constant.
    pub method_ref: u16,
    /// Constant-pool indices of static bootstrap arguments.
    pub arguments: Vec<u16>,
}

/// Typed `MethodParameters` attribute.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MethodParametersAttribute {
    /// Constant-pool index of `MethodParameters`.
    pub name_index: u16,
    /// Parameters in descriptor order.
    pub parameters: Vec<MethodParameter>,
}

/// One formal-method-parameter descriptor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MethodParameter {
    /// UTF-8 name index, or zero for an unnamed parameter.
    pub name_index: u16,
    /// Parameter declaration flags.
    pub access_flags: MethodParameterAccessFlags,
}

/// Typed `Record` attribute.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordAttribute {
    /// Constant-pool index of `Record`.
    pub name_index: u16,
    /// Record components in declaration order.
    pub components: Vec<RecordComponent>,
}

/// One record component and its nested attributes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordComponent {
    /// UTF-8 index of the component name.
    pub name_index: u16,
    /// UTF-8 index of the field descriptor.
    pub descriptor_index: u16,
    /// Component attributes, including signatures and annotations.
    pub attributes: Vec<Attribute>,
}

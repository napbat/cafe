//! DEX class-definition model.

use super::{AccessFlags, AnnotationDirectory, ClassData, EncodedValue, StringIndex, TypeIndex};

/// One DEX class definition and all associated data items.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClassDefinition {
    /// Defined class type.
    pub class: TypeIndex,
    /// Unmodified class access flags.
    pub access_flags: AccessFlags,
    /// Direct superclass, or `None` for `java.lang.Object`.
    pub superclass: Option<TypeIndex>,
    /// Direct interfaces in declaration order.
    pub interfaces: Vec<TypeIndex>,
    /// Optional source-file name.
    pub source_file: Option<StringIndex>,
    /// Class, field, method, and parameter annotations.
    pub annotations: AnnotationDirectory,
    /// Declared members and code items.
    pub class_data: Option<ClassData>,
    /// Initial values corresponding to leading static fields.
    pub static_values: Vec<EncodedValue>,
    /// Original class-definition table position.
    pub definition_index: u32,
}

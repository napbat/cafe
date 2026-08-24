//! DEX annotation visibility, sets, and definition associations.

use super::{EncodedAnnotation, FieldIndex, MethodIndex};

/// Runtime visibility assigned to an annotation item.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum AnnotationVisibility {
    /// Visible only to build tools.
    Build = 0,
    /// Visible to ordinary runtime reflection.
    Runtime = 1,
    /// Visible to the runtime system but not ordinary applications.
    System = 2,
}

impl AnnotationVisibility {
    /// Parses an encoded visibility byte.
    #[must_use]
    pub const fn from_byte(value: u8) -> Option<Self> {
        match value {
            0 => Some(Self::Build),
            1 => Some(Self::Runtime),
            2 => Some(Self::System),
            _ => None,
        }
    }

    /// Returns the exact encoded visibility byte.
    #[must_use]
    pub const fn byte(self) -> u8 {
        self as u8
    }
}

/// One annotation and its visibility.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnnotationItem {
    /// Visibility policy.
    pub visibility: AnnotationVisibility,
    /// Typed annotation contents.
    pub annotation: EncodedAnnotation,
    /// Original absolute `annotation_item` offset.
    pub data_offset: u32,
}

/// All annotation associations owned by one class definition.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AnnotationDirectory {
    /// Annotations applied to the class itself.
    pub class_annotations: Vec<AnnotationItem>,
    /// Field annotations sorted by field index.
    pub fields: Vec<FieldAnnotations>,
    /// Method annotations sorted by method index.
    pub methods: Vec<MethodAnnotations>,
    /// Parameter annotation lists sorted by method index.
    pub parameters: Vec<ParameterAnnotations>,
    /// Original absolute directory offset.
    pub data_offset: u32,
}

/// Annotations associated with one field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldAnnotations {
    /// Field identifier index.
    pub field: FieldIndex,
    /// Sorted annotation set.
    pub annotations: Vec<AnnotationItem>,
}

/// Annotations associated with one method.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MethodAnnotations {
    /// Method identifier index.
    pub method: MethodIndex,
    /// Sorted annotation set.
    pub annotations: Vec<AnnotationItem>,
}

/// Per-parameter annotation sets for one method.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParameterAnnotations {
    /// Method identifier index.
    pub method: MethodIndex,
    /// Annotation sets in formal-parameter order.
    pub parameters: Vec<Vec<AnnotationItem>>,
}

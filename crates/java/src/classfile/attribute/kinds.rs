//! Stable discriminators for querying typed attributes.

use super::KnownAttribute;
use crate::classfile::{
    AttributeLocation, JAVA_6_MAJOR_VERSION, JAVA_7_MAJOR_VERSION, JAVA_8_MAJOR_VERSION,
    JAVA_9_MAJOR_VERSION, JAVA_11_MAJOR_VERSION, JAVA_16_MAJOR_VERSION, JAVA_17_MAJOR_VERSION,
};

/// Kind of a recognized standard JVM attribute.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum KnownAttributeKind {
    /// `ConstantValue`.
    ConstantValue,
    /// `StackMapTable`.
    StackMapTable,
    /// `Exceptions`.
    Exceptions,
    /// `InnerClasses`.
    InnerClasses,
    /// `EnclosingMethod`.
    EnclosingMethod,
    /// `Synthetic`.
    Synthetic,
    /// `Signature`.
    Signature,
    /// `SourceFile`.
    SourceFile,
    /// `SourceDebugExtension`.
    SourceDebugExtension,
    /// `LineNumberTable`.
    LineNumberTable,
    /// `LocalVariableTable`.
    LocalVariableTable,
    /// `LocalVariableTypeTable`.
    LocalVariableTypeTable,
    /// `Deprecated`.
    Deprecated,
    /// `RuntimeVisibleAnnotations`.
    RuntimeVisibleAnnotations,
    /// `RuntimeInvisibleAnnotations`.
    RuntimeInvisibleAnnotations,
    /// `RuntimeVisibleParameterAnnotations`.
    RuntimeVisibleParameterAnnotations,
    /// `RuntimeInvisibleParameterAnnotations`.
    RuntimeInvisibleParameterAnnotations,
    /// `RuntimeVisibleTypeAnnotations`.
    RuntimeVisibleTypeAnnotations,
    /// `RuntimeInvisibleTypeAnnotations`.
    RuntimeInvisibleTypeAnnotations,
    /// `AnnotationDefault`.
    AnnotationDefault,
    /// `BootstrapMethods`.
    BootstrapMethods,
    /// `MethodParameters`.
    MethodParameters,
    /// `Module`.
    Module,
    /// `ModulePackages`.
    ModulePackages,
    /// `ModuleMainClass`.
    ModuleMainClass,
    /// `NestHost`.
    NestHost,
    /// `NestMembers`.
    NestMembers,
    /// `Record`.
    Record,
    /// `PermittedSubclasses`.
    PermittedSubclasses,
}

impl KnownAttributeKind {
    /// Every recognized standard attribute kind in specification order.
    pub const ALL: &[Self] = &[
        Self::ConstantValue,
        Self::StackMapTable,
        Self::Exceptions,
        Self::InnerClasses,
        Self::EnclosingMethod,
        Self::Synthetic,
        Self::Signature,
        Self::SourceFile,
        Self::SourceDebugExtension,
        Self::LineNumberTable,
        Self::LocalVariableTable,
        Self::LocalVariableTypeTable,
        Self::Deprecated,
        Self::RuntimeVisibleAnnotations,
        Self::RuntimeInvisibleAnnotations,
        Self::RuntimeVisibleParameterAnnotations,
        Self::RuntimeInvisibleParameterAnnotations,
        Self::RuntimeVisibleTypeAnnotations,
        Self::RuntimeInvisibleTypeAnnotations,
        Self::AnnotationDefault,
        Self::BootstrapMethods,
        Self::MethodParameters,
        Self::Module,
        Self::ModulePackages,
        Self::ModuleMainClass,
        Self::NestHost,
        Self::NestMembers,
        Self::Record,
        Self::PermittedSubclasses,
    ];

    /// Returns the class-file attribute name.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::ConstantValue => "ConstantValue",
            Self::StackMapTable => "StackMapTable",
            Self::Exceptions => "Exceptions",
            Self::InnerClasses => "InnerClasses",
            Self::EnclosingMethod => "EnclosingMethod",
            Self::Synthetic => "Synthetic",
            Self::Signature => "Signature",
            Self::SourceFile => "SourceFile",
            Self::SourceDebugExtension => "SourceDebugExtension",
            Self::LineNumberTable => "LineNumberTable",
            Self::LocalVariableTable => "LocalVariableTable",
            Self::LocalVariableTypeTable => "LocalVariableTypeTable",
            Self::Deprecated => "Deprecated",
            Self::RuntimeVisibleAnnotations => "RuntimeVisibleAnnotations",
            Self::RuntimeInvisibleAnnotations => "RuntimeInvisibleAnnotations",
            Self::RuntimeVisibleParameterAnnotations => "RuntimeVisibleParameterAnnotations",
            Self::RuntimeInvisibleParameterAnnotations => "RuntimeInvisibleParameterAnnotations",
            Self::RuntimeVisibleTypeAnnotations => "RuntimeVisibleTypeAnnotations",
            Self::RuntimeInvisibleTypeAnnotations => "RuntimeInvisibleTypeAnnotations",
            Self::AnnotationDefault => "AnnotationDefault",
            Self::BootstrapMethods => "BootstrapMethods",
            Self::MethodParameters => "MethodParameters",
            Self::Module => "Module",
            Self::ModulePackages => "ModulePackages",
            Self::ModuleMainClass => "ModuleMainClass",
            Self::NestHost => "NestHost",
            Self::NestMembers => "NestMembers",
            Self::Record => "Record",
            Self::PermittedSubclasses => "PermittedSubclasses",
        }
    }

    /// Resolves a standard attribute name to its typed kind.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|kind| kind.name() == name)
    }

    /// Returns the first class-file major version supporting this attribute.
    #[must_use]
    pub const fn minimum_major_version(self) -> Option<u16> {
        match self {
            Self::StackMapTable => Some(JAVA_6_MAJOR_VERSION),
            Self::BootstrapMethods => Some(JAVA_7_MAJOR_VERSION),
            Self::RuntimeVisibleTypeAnnotations
            | Self::RuntimeInvisibleTypeAnnotations
            | Self::MethodParameters => Some(JAVA_8_MAJOR_VERSION),
            Self::Module | Self::ModulePackages | Self::ModuleMainClass => {
                Some(JAVA_9_MAJOR_VERSION)
            }
            Self::NestHost | Self::NestMembers => Some(JAVA_11_MAJOR_VERSION),
            Self::Record => Some(JAVA_16_MAJOR_VERSION),
            Self::PermittedSubclasses => Some(JAVA_17_MAJOR_VERSION),
            _ => None,
        }
    }

    pub(super) const fn of(attribute: &KnownAttribute) -> Self {
        match attribute {
            KnownAttribute::ConstantValue(_) => Self::ConstantValue,
            KnownAttribute::StackMapTable(_) => Self::StackMapTable,
            KnownAttribute::Exceptions(_) => Self::Exceptions,
            KnownAttribute::InnerClasses(_) => Self::InnerClasses,
            KnownAttribute::EnclosingMethod(_) => Self::EnclosingMethod,
            KnownAttribute::Synthetic(_) => Self::Synthetic,
            KnownAttribute::Signature(_) => Self::Signature,
            KnownAttribute::SourceFile(_) => Self::SourceFile,
            KnownAttribute::SourceDebugExtension(_) => Self::SourceDebugExtension,
            KnownAttribute::LineNumberTable(_) => Self::LineNumberTable,
            KnownAttribute::LocalVariableTable(_) => Self::LocalVariableTable,
            KnownAttribute::LocalVariableTypeTable(_) => Self::LocalVariableTypeTable,
            KnownAttribute::Deprecated(_) => Self::Deprecated,
            KnownAttribute::RuntimeVisibleAnnotations(_) => Self::RuntimeVisibleAnnotations,
            KnownAttribute::RuntimeInvisibleAnnotations(_) => Self::RuntimeInvisibleAnnotations,
            KnownAttribute::RuntimeVisibleParameterAnnotations(_) => {
                Self::RuntimeVisibleParameterAnnotations
            }
            KnownAttribute::RuntimeInvisibleParameterAnnotations(_) => {
                Self::RuntimeInvisibleParameterAnnotations
            }
            KnownAttribute::RuntimeVisibleTypeAnnotations(_) => Self::RuntimeVisibleTypeAnnotations,
            KnownAttribute::RuntimeInvisibleTypeAnnotations(_) => {
                Self::RuntimeInvisibleTypeAnnotations
            }
            KnownAttribute::AnnotationDefault(_) => Self::AnnotationDefault,
            KnownAttribute::BootstrapMethods(_) => Self::BootstrapMethods,
            KnownAttribute::MethodParameters(_) => Self::MethodParameters,
            KnownAttribute::Module(_) => Self::Module,
            KnownAttribute::ModulePackages(_) => Self::ModulePackages,
            KnownAttribute::ModuleMainClass(_) => Self::ModuleMainClass,
            KnownAttribute::NestHost(_) => Self::NestHost,
            KnownAttribute::NestMembers(_) => Self::NestMembers,
            KnownAttribute::Record(_) => Self::Record,
            KnownAttribute::PermittedSubclasses(_) => Self::PermittedSubclasses,
        }
    }

    pub(super) const fn is_valid_at(self, location: AttributeLocation) -> bool {
        use AttributeLocation::{Class, Code, Field, Method, RecordComponent};
        match self {
            Self::ConstantValue => matches!(location, Field),
            Self::StackMapTable
            | Self::LineNumberTable
            | Self::LocalVariableTable
            | Self::LocalVariableTypeTable => matches!(location, Code),
            Self::Exceptions
            | Self::RuntimeVisibleParameterAnnotations
            | Self::RuntimeInvisibleParameterAnnotations
            | Self::AnnotationDefault
            | Self::MethodParameters => matches!(location, Method),
            Self::InnerClasses
            | Self::EnclosingMethod
            | Self::SourceFile
            | Self::SourceDebugExtension
            | Self::BootstrapMethods
            | Self::Module
            | Self::ModulePackages
            | Self::ModuleMainClass
            | Self::NestHost
            | Self::NestMembers
            | Self::Record
            | Self::PermittedSubclasses => matches!(location, Class),
            Self::Synthetic | Self::Deprecated => matches!(location, Class | Field | Method),
            Self::Signature
            | Self::RuntimeVisibleAnnotations
            | Self::RuntimeInvisibleAnnotations => {
                matches!(location, Class | Field | Method | RecordComponent)
            }
            Self::RuntimeVisibleTypeAnnotations | Self::RuntimeInvisibleTypeAnnotations => {
                matches!(location, Class | Field | Method | Code | RecordComponent)
            }
        }
    }
}

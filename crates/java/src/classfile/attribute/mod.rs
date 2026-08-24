//! Typed models, parser, and assembler for standard JVM attributes.

mod access;
mod annotations;
mod kinds;
mod models;
mod module;
mod parse;
mod stack_map;
mod write;

pub use self::annotations::{
    Annotation, AnnotationConstantKind, AnnotationDefaultAttribute, AnnotationElement,
    AnnotationsAttribute, ElementValue, ElementValueKind, LocalVariableTarget,
    ParameterAnnotationsAttribute, TypeAnnotation, TypeAnnotationTarget, TypeAnnotationTargetKind,
    TypeAnnotationsAttribute, TypePathEntry, TypePathKind,
};
pub use self::kinds::KnownAttributeKind;
pub use self::models::{
    BootstrapMethod, BootstrapMethodsAttribute, BytesAttribute, EnclosingMethodAttribute,
    IndexAttribute, IndexListAttribute, InnerClass, InnerClassesAttribute, LineNumber,
    LineNumberTableAttribute, LocalVariable, LocalVariableTableAttribute, LocalVariableType,
    LocalVariableTypeTableAttribute, MarkerAttribute, MethodParameter, MethodParametersAttribute,
    RecordAttribute, RecordComponent,
};
pub use self::module::{ModuleAttribute, ModuleExport, ModuleOpen, ModuleProvide, ModuleRequire};
pub use self::stack_map::{
    StackMapFrame, StackMapTableAttribute, VerificationType, VerificationTypeKind,
};

pub(crate) use self::stack_map::{STACK_MAP_OFFSET_DELTA_BIAS, StackMapFrameTag};

pub(crate) use self::parse::parse_attributes;
pub(crate) use self::write::{validate_known_model, write_attributes};

/// Standard JVM attribute with a parsed, editable payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KnownAttribute {
    /// `ConstantValue` field attribute.
    ConstantValue(IndexAttribute),
    /// `StackMapTable` code attribute.
    StackMapTable(StackMapTableAttribute),
    /// `Exceptions` method attribute.
    Exceptions(IndexListAttribute),
    /// `InnerClasses` class attribute.
    InnerClasses(InnerClassesAttribute),
    /// `EnclosingMethod` class attribute.
    EnclosingMethod(EnclosingMethodAttribute),
    /// `Synthetic` marker attribute.
    Synthetic(MarkerAttribute),
    /// Generic `Signature` attribute.
    Signature(IndexAttribute),
    /// `SourceFile` class attribute.
    SourceFile(IndexAttribute),
    /// `SourceDebugExtension` class attribute.
    SourceDebugExtension(BytesAttribute),
    /// `LineNumberTable` code attribute.
    LineNumberTable(LineNumberTableAttribute),
    /// `LocalVariableTable` code attribute.
    LocalVariableTable(LocalVariableTableAttribute),
    /// `LocalVariableTypeTable` code attribute.
    LocalVariableTypeTable(LocalVariableTypeTableAttribute),
    /// `Deprecated` marker attribute.
    Deprecated(MarkerAttribute),
    /// `RuntimeVisibleAnnotations` attribute.
    RuntimeVisibleAnnotations(AnnotationsAttribute),
    /// `RuntimeInvisibleAnnotations` attribute.
    RuntimeInvisibleAnnotations(AnnotationsAttribute),
    /// `RuntimeVisibleParameterAnnotations` method attribute.
    RuntimeVisibleParameterAnnotations(ParameterAnnotationsAttribute),
    /// `RuntimeInvisibleParameterAnnotations` method attribute.
    RuntimeInvisibleParameterAnnotations(ParameterAnnotationsAttribute),
    /// `RuntimeVisibleTypeAnnotations` attribute.
    RuntimeVisibleTypeAnnotations(TypeAnnotationsAttribute),
    /// `RuntimeInvisibleTypeAnnotations` attribute.
    RuntimeInvisibleTypeAnnotations(TypeAnnotationsAttribute),
    /// `AnnotationDefault` annotation-interface method attribute.
    AnnotationDefault(AnnotationDefaultAttribute),
    /// `BootstrapMethods` class attribute.
    BootstrapMethods(BootstrapMethodsAttribute),
    /// `MethodParameters` method attribute.
    MethodParameters(MethodParametersAttribute),
    /// `Module` module declaration attribute.
    Module(ModuleAttribute),
    /// `ModulePackages` module attribute.
    ModulePackages(IndexListAttribute),
    /// `ModuleMainClass` module attribute.
    ModuleMainClass(IndexAttribute),
    /// `NestHost` class attribute.
    NestHost(IndexAttribute),
    /// `NestMembers` class attribute.
    NestMembers(IndexListAttribute),
    /// `Record` class attribute.
    Record(RecordAttribute),
    /// `PermittedSubclasses` sealed-type attribute.
    PermittedSubclasses(IndexListAttribute),
}

impl KnownAttribute {
    /// Returns the standard attribute name.
    #[must_use]
    pub const fn name(&self) -> &'static str {
        self.kind().name()
    }

    /// Returns the constant-pool index of the attribute name.
    #[must_use]
    pub const fn name_index(&self) -> u16 {
        match self {
            Self::ConstantValue(attribute)
            | Self::Signature(attribute)
            | Self::SourceFile(attribute)
            | Self::ModuleMainClass(attribute)
            | Self::NestHost(attribute) => attribute.name_index,
            Self::Exceptions(attribute)
            | Self::ModulePackages(attribute)
            | Self::NestMembers(attribute)
            | Self::PermittedSubclasses(attribute) => attribute.name_index,
            Self::Synthetic(attribute) | Self::Deprecated(attribute) => attribute.name_index,
            Self::SourceDebugExtension(attribute) => attribute.name_index,
            Self::StackMapTable(attribute) => attribute.name_index,
            Self::InnerClasses(attribute) => attribute.name_index,
            Self::EnclosingMethod(attribute) => attribute.name_index,
            Self::LineNumberTable(attribute) => attribute.name_index,
            Self::LocalVariableTable(attribute) => attribute.name_index,
            Self::LocalVariableTypeTable(attribute) => attribute.name_index,
            Self::RuntimeVisibleAnnotations(attribute)
            | Self::RuntimeInvisibleAnnotations(attribute) => attribute.name_index,
            Self::RuntimeVisibleParameterAnnotations(attribute)
            | Self::RuntimeInvisibleParameterAnnotations(attribute) => attribute.name_index,
            Self::RuntimeVisibleTypeAnnotations(attribute)
            | Self::RuntimeInvisibleTypeAnnotations(attribute) => attribute.name_index,
            Self::AnnotationDefault(attribute) => attribute.name_index,
            Self::BootstrapMethods(attribute) => attribute.name_index,
            Self::MethodParameters(attribute) => attribute.name_index,
            Self::Module(attribute) => attribute.name_index,
            Self::Record(attribute) => attribute.name_index,
        }
    }

    /// Returns the stable kind discriminator for this standard attribute.
    #[must_use]
    pub const fn kind(&self) -> KnownAttributeKind {
        KnownAttributeKind::of(self)
    }

    pub(crate) const fn is_valid_at(&self, location: super::AttributeLocation) -> bool {
        self.kind().is_valid_at(location)
    }
}

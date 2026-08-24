//! Declaration, parameter, type-use, and default annotation models.

/// One JVM annotation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Annotation {
    /// UTF-8 constant-pool index of the annotation type descriptor.
    pub type_index: u16,
    /// Named element values in encoded order.
    pub elements: Vec<AnnotationElement>,
}

/// One named annotation element value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnnotationElement {
    /// UTF-8 constant-pool index of the element name.
    pub name_index: u16,
    /// Encoded element value.
    pub value: ElementValue,
}

/// Primitive/string kind used by a constant annotation element.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AnnotationConstantKind {
    /// `byte` (`B`).
    Byte,
    /// `char` (`C`).
    Char,
    /// `double` (`D`).
    Double,
    /// `float` (`F`).
    Float,
    /// `int` (`I`).
    Int,
    /// `long` (`J`).
    Long,
    /// `short` (`S`).
    Short,
    /// `boolean` (`Z`).
    Boolean,
    /// `String` (`s`).
    String,
}

impl AnnotationConstantKind {
    /// Returns the class-file element-value tag.
    #[must_use]
    pub const fn tag(self) -> u8 {
        match self {
            Self::Byte => b'B',
            Self::Char => b'C',
            Self::Double => b'D',
            Self::Float => b'F',
            Self::Int => b'I',
            Self::Long => b'J',
            Self::Short => b'S',
            Self::Boolean => b'Z',
            Self::String => b's',
        }
    }

    pub(crate) const fn from_tag(tag: u8) -> Option<Self> {
        match tag {
            b'B' => Some(Self::Byte),
            b'C' => Some(Self::Char),
            b'D' => Some(Self::Double),
            b'F' => Some(Self::Float),
            b'I' => Some(Self::Int),
            b'J' => Some(Self::Long),
            b'S' => Some(Self::Short),
            b'Z' => Some(Self::Boolean),
            b's' => Some(Self::String),
            _ => None,
        }
    }
}

/// Format discriminator for an annotation element value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ElementValueKind {
    /// Primitive or string constant.
    Constant(AnnotationConstantKind),
    /// Enum constant.
    Enum,
    /// Class literal.
    Class,
    /// Nested annotation.
    Annotation,
    /// Array of element values.
    Array,
}

impl ElementValueKind {
    /// Returns the class-file element-value tag.
    #[must_use]
    pub const fn tag(self) -> u8 {
        match self {
            Self::Constant(kind) => kind.tag(),
            Self::Enum => b'e',
            Self::Class => b'c',
            Self::Annotation => b'@',
            Self::Array => b'[',
        }
    }

    pub(crate) const fn from_tag(tag: u8) -> Option<Self> {
        if let Some(kind) = AnnotationConstantKind::from_tag(tag) {
            return Some(Self::Constant(kind));
        }
        match tag {
            tag if tag == Self::Enum.tag() => Some(Self::Enum),
            tag if tag == Self::Class.tag() => Some(Self::Class),
            tag if tag == Self::Annotation.tag() => Some(Self::Annotation),
            tag if tag == Self::Array.tag() => Some(Self::Array),
            _ => None,
        }
    }
}

/// Recursive value stored in an annotation element.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ElementValue {
    /// Primitive or string constant-pool value.
    Constant {
        /// Encoded primitive/string kind.
        kind: AnnotationConstantKind,
        /// Constant-pool index containing the value.
        constant_index: u16,
    },
    /// Enum constant name and enum type descriptor.
    Enum {
        /// UTF-8 index of the enum type descriptor.
        type_name_index: u16,
        /// UTF-8 index of the enum constant name.
        constant_name_index: u16,
    },
    /// Class literal represented by a UTF-8 return descriptor.
    Class(u16),
    /// Nested annotation value.
    Annotation(Box<Annotation>),
    /// Ordered array of annotation values.
    Array(Vec<ElementValue>),
}

impl ElementValue {
    /// Returns this element value's format discriminator.
    #[must_use]
    pub const fn kind(&self) -> ElementValueKind {
        match self {
            Self::Constant { kind, .. } => ElementValueKind::Constant(*kind),
            Self::Enum { .. } => ElementValueKind::Enum,
            Self::Class(_) => ElementValueKind::Class,
            Self::Annotation(_) => ElementValueKind::Annotation,
            Self::Array(_) => ElementValueKind::Array,
        }
    }
}

/// Typed declaration-annotation attribute.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnnotationsAttribute {
    /// Constant-pool index of the visible or invisible attribute name.
    pub name_index: u16,
    /// Annotations in encoded order.
    pub annotations: Vec<Annotation>,
}

/// Typed parameter-annotation attribute.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParameterAnnotationsAttribute {
    /// Constant-pool index of the visible or invisible attribute name.
    pub name_index: u16,
    /// Annotation arrays in formal-parameter order.
    pub parameters: Vec<Vec<Annotation>>,
}

/// Typed `AnnotationDefault` attribute.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnnotationDefaultAttribute {
    /// Constant-pool index of `AnnotationDefault`.
    pub name_index: u16,
    /// Default value for the annotation-interface element.
    pub value: ElementValue,
}

/// One local-variable range targeted by a type annotation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LocalVariableTarget {
    /// Inclusive bytecode start offset.
    pub start_pc: u16,
    /// Byte length of the live range.
    pub length: u16,
    /// Local-variable array slot.
    pub index: u16,
}

/// Precise target of a type-use annotation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TypeAnnotationTarget {
    /// Type parameter of a class or interface (`0x00`).
    ClassTypeParameter(u8),
    /// Type parameter of a method or constructor (`0x01`).
    MethodTypeParameter(u8),
    /// Superclass or superinterface type (`0x10`).
    ClassExtends(u16),
    /// Bound of a class or interface type parameter (`0x11`).
    ClassTypeParameterBound {
        /// Type-parameter table index.
        parameter_index: u8,
        /// Bound index within that type parameter.
        bound_index: u8,
    },
    /// Bound of a method or constructor type parameter (`0x12`).
    MethodTypeParameterBound {
        /// Type-parameter table index.
        parameter_index: u8,
        /// Bound index within that type parameter.
        bound_index: u8,
    },
    /// Field or record-component type (`0x13`).
    Field,
    /// Method return type or constructor result (`0x14`).
    MethodReturn,
    /// Receiver type (`0x15`).
    MethodReceiver,
    /// Formal-parameter type (`0x16`).
    MethodFormalParameter(u8),
    /// Declared thrown type (`0x17`).
    Throws(u16),
    /// Local-variable type (`0x40`).
    LocalVariable(Vec<LocalVariableTarget>),
    /// Resource-variable type (`0x41`).
    ResourceVariable(Vec<LocalVariableTarget>),
    /// Exception parameter (`0x42`).
    ExceptionParameter(u16),
    /// Type tested by `instanceof` (`0x43`).
    InstanceOf(u16),
    /// Type created by `new` (`0x44`).
    New(u16),
    /// Constructor reference receiver (`0x45`).
    ConstructorReference(u16),
    /// Method reference receiver (`0x46`).
    MethodReference(u16),
    /// Cast type argument (`0x47`).
    Cast {
        /// Bytecode instruction offset.
        offset: u16,
        /// Cast intersection-type component index.
        type_argument_index: u8,
    },
    /// Constructor invocation type argument (`0x48`).
    ConstructorInvocationTypeArgument {
        /// Bytecode instruction offset.
        offset: u16,
        /// Type-argument index.
        type_argument_index: u8,
    },
    /// Method invocation type argument (`0x49`).
    MethodInvocationTypeArgument {
        /// Bytecode instruction offset.
        offset: u16,
        /// Type-argument index.
        type_argument_index: u8,
    },
    /// Constructor reference type argument (`0x4a`).
    ConstructorReferenceTypeArgument {
        /// Bytecode instruction offset.
        offset: u16,
        /// Type-argument index.
        type_argument_index: u8,
    },
    /// Method reference type argument (`0x4b`).
    MethodReferenceTypeArgument {
        /// Bytecode instruction offset.
        offset: u16,
        /// Type-argument index.
        type_argument_index: u8,
    },
}

/// Format discriminator for a type annotation target.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum TypeAnnotationTargetKind {
    /// Type parameter of a class or interface.
    ClassTypeParameter = 0x00,
    /// Type parameter of a method or constructor.
    MethodTypeParameter = 0x01,
    /// Superclass or superinterface type.
    ClassExtends = 0x10,
    /// Bound of a class or interface type parameter.
    ClassTypeParameterBound = 0x11,
    /// Bound of a method or constructor type parameter.
    MethodTypeParameterBound = 0x12,
    /// Field or record-component type.
    Field = 0x13,
    /// Method return type or constructor result.
    MethodReturn = 0x14,
    /// Receiver type.
    MethodReceiver = 0x15,
    /// Formal-parameter type.
    MethodFormalParameter = 0x16,
    /// Declared thrown type.
    Throws = 0x17,
    /// Local-variable type.
    LocalVariable = 0x40,
    /// Resource-variable type.
    ResourceVariable = 0x41,
    /// Exception parameter.
    ExceptionParameter = 0x42,
    /// Type tested by `instanceof`.
    InstanceOf = 0x43,
    /// Type created by `new`.
    New = 0x44,
    /// Constructor reference receiver.
    ConstructorReference = 0x45,
    /// Method reference receiver.
    MethodReference = 0x46,
    /// Cast type argument.
    Cast = 0x47,
    /// Constructor invocation type argument.
    ConstructorInvocationTypeArgument = 0x48,
    /// Method invocation type argument.
    MethodInvocationTypeArgument = 0x49,
    /// Constructor reference type argument.
    ConstructorReferenceTypeArgument = 0x4a,
    /// Method reference type argument.
    MethodReferenceTypeArgument = 0x4b,
}

impl TypeAnnotationTargetKind {
    /// Returns the class-file target-type tag.
    #[must_use]
    pub const fn tag(self) -> u8 {
        self as u8
    }

    pub(crate) const fn from_tag(tag: u8) -> Option<Self> {
        match tag {
            tag if tag == Self::ClassTypeParameter.tag() => Some(Self::ClassTypeParameter),
            tag if tag == Self::MethodTypeParameter.tag() => Some(Self::MethodTypeParameter),
            tag if tag == Self::ClassExtends.tag() => Some(Self::ClassExtends),
            tag if tag == Self::ClassTypeParameterBound.tag() => {
                Some(Self::ClassTypeParameterBound)
            }
            tag if tag == Self::MethodTypeParameterBound.tag() => {
                Some(Self::MethodTypeParameterBound)
            }
            tag if tag == Self::Field.tag() => Some(Self::Field),
            tag if tag == Self::MethodReturn.tag() => Some(Self::MethodReturn),
            tag if tag == Self::MethodReceiver.tag() => Some(Self::MethodReceiver),
            tag if tag == Self::MethodFormalParameter.tag() => Some(Self::MethodFormalParameter),
            tag if tag == Self::Throws.tag() => Some(Self::Throws),
            tag if tag == Self::LocalVariable.tag() => Some(Self::LocalVariable),
            tag if tag == Self::ResourceVariable.tag() => Some(Self::ResourceVariable),
            tag if tag == Self::ExceptionParameter.tag() => Some(Self::ExceptionParameter),
            tag if tag == Self::InstanceOf.tag() => Some(Self::InstanceOf),
            tag if tag == Self::New.tag() => Some(Self::New),
            tag if tag == Self::ConstructorReference.tag() => Some(Self::ConstructorReference),
            tag if tag == Self::MethodReference.tag() => Some(Self::MethodReference),
            tag if tag == Self::Cast.tag() => Some(Self::Cast),
            tag if tag == Self::ConstructorInvocationTypeArgument.tag() => {
                Some(Self::ConstructorInvocationTypeArgument)
            }
            tag if tag == Self::MethodInvocationTypeArgument.tag() => {
                Some(Self::MethodInvocationTypeArgument)
            }
            tag if tag == Self::ConstructorReferenceTypeArgument.tag() => {
                Some(Self::ConstructorReferenceTypeArgument)
            }
            tag if tag == Self::MethodReferenceTypeArgument.tag() => {
                Some(Self::MethodReferenceTypeArgument)
            }
            _ => None,
        }
    }
}

impl TypeAnnotationTarget {
    /// Returns this target's format discriminator.
    #[must_use]
    pub const fn kind(&self) -> TypeAnnotationTargetKind {
        match self {
            Self::ClassTypeParameter(_) => TypeAnnotationTargetKind::ClassTypeParameter,
            Self::MethodTypeParameter(_) => TypeAnnotationTargetKind::MethodTypeParameter,
            Self::ClassExtends(_) => TypeAnnotationTargetKind::ClassExtends,
            Self::ClassTypeParameterBound { .. } => {
                TypeAnnotationTargetKind::ClassTypeParameterBound
            }
            Self::MethodTypeParameterBound { .. } => {
                TypeAnnotationTargetKind::MethodTypeParameterBound
            }
            Self::Field => TypeAnnotationTargetKind::Field,
            Self::MethodReturn => TypeAnnotationTargetKind::MethodReturn,
            Self::MethodReceiver => TypeAnnotationTargetKind::MethodReceiver,
            Self::MethodFormalParameter(_) => TypeAnnotationTargetKind::MethodFormalParameter,
            Self::Throws(_) => TypeAnnotationTargetKind::Throws,
            Self::LocalVariable(_) => TypeAnnotationTargetKind::LocalVariable,
            Self::ResourceVariable(_) => TypeAnnotationTargetKind::ResourceVariable,
            Self::ExceptionParameter(_) => TypeAnnotationTargetKind::ExceptionParameter,
            Self::InstanceOf(_) => TypeAnnotationTargetKind::InstanceOf,
            Self::New(_) => TypeAnnotationTargetKind::New,
            Self::ConstructorReference(_) => TypeAnnotationTargetKind::ConstructorReference,
            Self::MethodReference(_) => TypeAnnotationTargetKind::MethodReference,
            Self::Cast { .. } => TypeAnnotationTargetKind::Cast,
            Self::ConstructorInvocationTypeArgument { .. } => {
                TypeAnnotationTargetKind::ConstructorInvocationTypeArgument
            }
            Self::MethodInvocationTypeArgument { .. } => {
                TypeAnnotationTargetKind::MethodInvocationTypeArgument
            }
            Self::ConstructorReferenceTypeArgument { .. } => {
                TypeAnnotationTargetKind::ConstructorReferenceTypeArgument
            }
            Self::MethodReferenceTypeArgument { .. } => {
                TypeAnnotationTargetKind::MethodReferenceTypeArgument
            }
        }
    }
}

/// One step in a type annotation's nested-type path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TypePathEntry {
    /// Deeper into an array type (`0`).
    Array,
    /// Deeper into a nested type (`1`).
    Nested,
    /// Bound of a wildcard type argument (`2`).
    WildcardBound,
    /// Indexed type argument (`3`).
    TypeArgument(u8),
}

/// Format discriminator for a type annotation path entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum TypePathKind {
    /// Deeper into an array type.
    Array = 0,
    /// Deeper into a nested type.
    Nested = 1,
    /// Bound of a wildcard type argument.
    WildcardBound = 2,
    /// Indexed type argument.
    TypeArgument = 3,
}

const UNUSED_TYPE_PATH_ARGUMENT: u8 = 0;

impl TypePathKind {
    /// Returns the class-file type-path tag.
    #[must_use]
    pub const fn tag(self) -> u8 {
        self as u8
    }

    const fn from_tag(tag: u8) -> Option<Self> {
        match tag {
            tag if tag == Self::Array.tag() => Some(Self::Array),
            tag if tag == Self::Nested.tag() => Some(Self::Nested),
            tag if tag == Self::WildcardBound.tag() => Some(Self::WildcardBound),
            tag if tag == Self::TypeArgument.tag() => Some(Self::TypeArgument),
            _ => None,
        }
    }
}

impl TypePathEntry {
    /// Returns this path entry's format discriminator.
    #[must_use]
    pub const fn kind(self) -> TypePathKind {
        match self {
            Self::Array => TypePathKind::Array,
            Self::Nested => TypePathKind::Nested,
            Self::WildcardBound => TypePathKind::WildcardBound,
            Self::TypeArgument(_) => TypePathKind::TypeArgument,
        }
    }

    pub(crate) const fn from_encoded(kind: u8, argument: u8) -> Option<Self> {
        match TypePathKind::from_tag(kind) {
            Some(TypePathKind::Array) if argument == UNUSED_TYPE_PATH_ARGUMENT => Some(Self::Array),
            Some(TypePathKind::Nested) if argument == UNUSED_TYPE_PATH_ARGUMENT => {
                Some(Self::Nested)
            }
            Some(TypePathKind::WildcardBound) if argument == UNUSED_TYPE_PATH_ARGUMENT => {
                Some(Self::WildcardBound)
            }
            Some(TypePathKind::TypeArgument) => Some(Self::TypeArgument(argument)),
            _ => None,
        }
    }

    pub(crate) const fn encoded(self) -> (TypePathKind, u8) {
        match self {
            Self::Array => (TypePathKind::Array, UNUSED_TYPE_PATH_ARGUMENT),
            Self::Nested => (TypePathKind::Nested, UNUSED_TYPE_PATH_ARGUMENT),
            Self::WildcardBound => (TypePathKind::WildcardBound, UNUSED_TYPE_PATH_ARGUMENT),
            Self::TypeArgument(index) => (TypePathKind::TypeArgument, index),
        }
    }
}

/// One type-use annotation with target and nested-type path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypeAnnotation {
    /// Declaration, signature, or bytecode target.
    pub target: TypeAnnotationTarget,
    /// Path from the target type to the annotated type use.
    pub path: Vec<TypePathEntry>,
    /// Annotation type and element values.
    pub annotation: Annotation,
}

/// Typed visible or invisible type-annotation attribute.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypeAnnotationsAttribute {
    /// Constant-pool index of the visible or invisible attribute name.
    pub name_index: u16,
    /// Type annotations in encoded order.
    pub annotations: Vec<TypeAnnotation>,
}

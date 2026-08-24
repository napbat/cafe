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

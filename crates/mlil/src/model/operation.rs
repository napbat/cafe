//! Encoding-independent MLIL operation vocabulary.

use disassembler::{CatchType, Reference};

/// Literal value materialized by MLIL.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Constant {
    /// Null reference.
    Null,
    /// Signed Java computational integer.
    Integer(i32),
    /// Signed 64-bit integer.
    Long(i64),
    /// Exact IEEE-754 single-precision bits.
    Float(u32),
    /// Exact IEEE-754 double-precision bits.
    Double(u64),
    /// String, type, method-handle, method-type, or another indexed constant.
    Reference(Reference),
}

/// Unary primitive operator.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum UnaryOperator {
    /// Arithmetic negation.
    Negate,
    /// Bitwise complement.
    BitwiseNot,
}

/// Binary arithmetic, bitwise, or shift operator.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BinaryOperator {
    /// Addition.
    Add,
    /// Subtraction.
    Subtract,
    /// Literal minus value.
    ReverseSubtract,
    /// Multiplication.
    Multiply,
    /// Division.
    Divide,
    /// Remainder.
    Remainder,
    /// Bitwise conjunction.
    And,
    /// Bitwise disjunction.
    Or,
    /// Bitwise exclusive-or.
    Xor,
    /// Left shift.
    ShiftLeft,
    /// Arithmetic right shift.
    ShiftRight,
    /// Logical right shift.
    UnsignedShiftRight,
}

/// Primitive numeric conversion.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Conversion {
    /// `int` to `long`.
    IntToLong,
    /// `int` to `float`.
    IntToFloat,
    /// `int` to `double`.
    IntToDouble,
    /// `long` to `int`.
    LongToInt,
    /// `long` to `float`.
    LongToFloat,
    /// `long` to `double`.
    LongToDouble,
    /// `float` to `int`.
    FloatToInt,
    /// `float` to `long`.
    FloatToLong,
    /// `float` to `double`.
    FloatToDouble,
    /// `double` to `int`.
    DoubleToInt,
    /// `double` to `long`.
    DoubleToLong,
    /// `double` to `float`.
    DoubleToFloat,
    /// Narrow and sign-extend as a byte.
    IntToByte,
    /// Narrow and zero-extend as a char.
    IntToChar,
    /// Narrow and sign-extend as a short.
    IntToShort,
}

/// Three-way comparison behavior, including floating-point NaN ordering.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ThreeWayComparison {
    /// Signed long comparison.
    Long,
    /// Float comparison producing `-1` for NaN.
    FloatNanLow,
    /// Float comparison producing `1` for NaN.
    FloatNanHigh,
    /// Double comparison producing `-1` for NaN.
    DoubleNanLow,
    /// Double comparison producing `1` for NaN.
    DoubleNanHigh,
}

/// Relational predicate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Relation {
    /// Equal.
    Equal,
    /// Not equal.
    NotEqual,
    /// Less than.
    Less,
    /// Greater than or equal.
    GreaterOrEqual,
    /// Greater than.
    Greater,
    /// Less than or equal.
    LessOrEqual,
}

/// Shape of operands consumed by a conditional branch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BranchOperandKind {
    /// Compare one integer-like value with zero.
    IntegerZero,
    /// Compare two integer-like values.
    IntegerPair,
    /// Compare two references.
    ReferencePair,
    /// Compare one reference with null.
    ReferenceNull,
    /// Compare one already-materialized Boolean value with true.
    Boolean,
}

/// Complete encoding-independent branch predicate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BranchPredicate {
    /// Relation tested by the branch.
    pub relation: Relation,
    /// Operand shape.
    pub operands: BranchOperandKind,
}

/// Direction of an array operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ArrayAccess {
    /// Read an element.
    Get,
    /// Write an element.
    Put,
}

/// Java array or field computational element category.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ElementType {
    /// Unconstrained 32-bit value.
    Bits32,
    /// Unconstrained 64-bit value.
    Bits64,
    /// Integer-like value.
    Integer,
    /// Signed 64-bit integer.
    Long,
    /// IEEE-754 single-precision value.
    Float,
    /// IEEE-754 double-precision value.
    Double,
    /// Object or array reference.
    Reference,
    /// Boolean value.
    Boolean,
    /// Signed byte value.
    Byte,
    /// Byte or Boolean selected dynamically by the JVM array type.
    ByteOrBoolean,
    /// Unsigned UTF-16 code unit.
    Char,
    /// Signed short value.
    Short,
}

/// Static/instance and read/write field mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FieldAccess {
    /// Read an instance field.
    GetInstance,
    /// Write an instance field.
    PutInstance,
    /// Read a static field.
    GetStatic,
    /// Write a static field.
    PutStatic,
}

/// Semantic dispatch mode of a call.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CallKind {
    /// Virtual class dispatch.
    Virtual,
    /// Superclass dispatch.
    Super,
    /// Constructor, private, or otherwise direct dispatch.
    Direct,
    /// Static dispatch.
    Static,
    /// Interface dispatch.
    Interface,
    /// Signature-polymorphic invocation.
    Polymorphic,
    /// Dynamically linked call site.
    Dynamic,
}

/// Exact semantic array type with optional native operand provenance.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ArrayType {
    descriptor: String,
    source_reference: Option<Reference>,
}

impl ArrayType {
    /// Creates a semantic array type without source-table coupling.
    #[must_use]
    pub fn new(descriptor: impl Into<String>) -> Self {
        Self {
            descriptor: descriptor.into(),
            source_reference: None,
        }
    }

    /// Retains the native type operand that contributed this semantic type.
    #[must_use]
    pub fn with_source_reference(mut self, reference: Reference) -> Self {
        self.source_reference = Some(reference);
        self
    }

    /// Returns the exact JVM-compatible array descriptor.
    #[must_use]
    pub fn descriptor(&self) -> &str {
        &self.descriptor
    }

    /// Returns the optional source-native type operand retained as provenance.
    #[must_use]
    pub const fn source_reference(&self) -> Option<&Reference> {
        self.source_reference.as_ref()
    }
}

/// Type operand used by allocation operations.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum AllocationKind {
    /// Allocate an uninitialized object.
    Object(Reference),
    /// Allocate an array from one or more dimension lengths.
    Array {
        /// Exact semantic array type, independent of native encoding choice.
        array_type: ArrayType,
        /// Number of leading dimensions allocated by this operation.
        dimensions: u8,
    },
    /// Allocate and initialize an array from explicit element operands.
    InitializedArray {
        /// Exact semantic array type, independent of native encoding choice.
        array_type: ArrayType,
    },
}

/// Monitor synchronization operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MonitorAction {
    /// Acquire a monitor.
    Enter,
    /// Release a monitor.
    Exit,
}

/// One generic semantic MLIL operation.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Operation {
    /// No semantic effect.
    Nop,
    /// Copy one value.
    Copy,
    /// Simultaneously copy equally sized operand and result lists.
    ParallelCopy,
    /// Discard one or more values whose evaluation remains observable.
    Discard,
    /// Retain runtime values while refining their static analysis types.
    TypeRefine,
    /// Materialize a constant.
    Constant(Constant),
    /// Apply a unary primitive operator.
    Unary(UnaryOperator),
    /// Apply a binary primitive operator.
    Binary(BinaryOperator),
    /// Convert between primitive types.
    Convert(Conversion),
    /// Produce an integer three-way comparison result.
    Compare(ThreeWayComparison),
    /// Conditionally transfer control.
    Branch(BranchPredicate),
    /// Unconditionally transfer control.
    Jump,
    /// Dispatch on an integer selector. Keys are in edge order after default.
    Switch(Vec<i64>),
    /// Return zero or one value from the current method.
    Return,
    /// Throw one reference.
    Throw,
    /// Read or write an array element.
    ///
    /// Uses are `[array, index]` for reads and `[array, index, value]` for
    /// writes, independent of the native ISA's encoded or evaluation order.
    Array {
        /// Read or write mode.
        access: ArrayAccess,
        /// Encoded computational element category.
        element: ElementType,
    },
    /// Read an array length.
    ArrayLength,
    /// Read or write a field.
    ///
    /// Instance uses place the receiver first, followed by the stored value for
    /// a write. Static writes consume only the stored value.
    Field {
        /// Static/instance and read/write mode.
        access: FieldAccess,
        /// Resolved or unresolved native field reference.
        field: Reference,
    },
    /// Invoke a method or dynamic call site.
    ///
    /// Non-static calls place the receiver before descriptor-ordered arguments.
    /// Static and dynamic calls contain only descriptor-ordered arguments.
    /// Signature-polymorphic calls keep the declared method descriptor in the
    /// target symbol and the effective call-site descriptor separately.
    Call {
        /// Dispatch behavior.
        kind: CallKind,
        /// Resolved or unresolved native method/call-site reference.
        target: Reference,
        /// Effective JVM-compatible descriptor when resolved.
        descriptor: Option<String>,
    },
    /// Allocate an object or array.
    Allocate(AllocationKind),
    /// Initialize an existing primitive array from semantic constant values.
    InitializeArray {
        /// Exact semantic array type.
        array_type: ArrayType,
        /// Element values in array order.
        values: Vec<Constant>,
    },
    /// Check and retain a reference against a type.
    CheckCast(Reference),
    /// Test whether a reference is an instance of a type.
    InstanceOf(Reference),
    /// Enter or exit an object monitor.
    Monitor(MonitorAction),
    /// Materialize the exception delivered to a handler landing pad.
    CaughtException(CatchType),
    /// Explicitly retained implementation-defined semantic operation.
    Intrinsic(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ControlClass {
    Normal,
    Branch,
    Jump,
    Switch,
    Return,
    Throw,
}

impl Operation {
    pub(crate) const fn control_class(&self) -> ControlClass {
        match self {
            Self::Branch(_) => ControlClass::Branch,
            Self::Jump => ControlClass::Jump,
            Self::Switch(_) => ControlClass::Switch,
            Self::Return => ControlClass::Return,
            Self::Throw => ControlClass::Throw,
            Self::Nop
            | Self::Copy
            | Self::ParallelCopy
            | Self::Discard
            | Self::TypeRefine
            | Self::Constant(_)
            | Self::Unary(_)
            | Self::Binary(_)
            | Self::Convert(_)
            | Self::Compare(_)
            | Self::Array { .. }
            | Self::ArrayLength
            | Self::Field { .. }
            | Self::Call { .. }
            | Self::Allocate(_)
            | Self::InitializeArray { .. }
            | Self::CheckCast(_)
            | Self::InstanceOf(_)
            | Self::Monitor(_)
            | Self::CaughtException(_)
            | Self::Intrinsic(_) => ControlClass::Normal,
        }
    }

    /// Returns a compact semantic mnemonic.
    #[must_use]
    pub const fn mnemonic(&self) -> &'static str {
        match self {
            Self::Nop => "nop",
            Self::Copy => "copy",
            Self::ParallelCopy => "parallel-copy",
            Self::Discard => "discard",
            Self::TypeRefine => "type-refine",
            Self::Constant(_) => "const",
            Self::Unary(_) => "unary",
            Self::Binary(_) => "binary",
            Self::Convert(_) => "convert",
            Self::Compare(_) => "compare",
            Self::Branch(_) => "branch",
            Self::Jump => "jump",
            Self::Switch(_) => "switch",
            Self::Return => "return",
            Self::Throw => "throw",
            Self::Array { access, .. } => match access {
                ArrayAccess::Get => "array-get",
                ArrayAccess::Put => "array-put",
            },
            Self::ArrayLength => "array-length",
            Self::Field { access, .. } => match access {
                FieldAccess::GetInstance => "field-get",
                FieldAccess::PutInstance => "field-put",
                FieldAccess::GetStatic => "static-get",
                FieldAccess::PutStatic => "static-put",
            },
            Self::Call { .. } => "call",
            Self::Allocate(_) => "allocate",
            Self::InitializeArray { .. } => "array-initialize",
            Self::CheckCast(_) => "check-cast",
            Self::InstanceOf(_) => "instance-of",
            Self::Monitor(MonitorAction::Enter) => "monitor-enter",
            Self::Monitor(MonitorAction::Exit) => "monitor-exit",
            Self::CaughtException(_) => "caught-exception",
            Self::Intrinsic(_) => "intrinsic",
        }
    }
}

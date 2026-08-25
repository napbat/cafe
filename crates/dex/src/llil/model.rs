//! Dalvik LLIL instruction and semantic operation model.

use crate::analysis::{InstructionSemantics, RegisterOperand, ValueKind};
use crate::instruction::{
    ArrayDataPayload, IndexKind, InstructionData, PackedSwitchPayload, SparseSwitchPayload,
};

/// Semantic category of a DEX constant-producing instruction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ConstantKind {
    /// Single-register literal bits.
    Narrow,
    /// Wide literal bits occupying two registers.
    Wide,
    /// String identifier.
    String,
    /// Type/class identifier.
    Class,
    /// Method-handle identifier.
    MethodHandle,
    /// Method-prototype identifier.
    MethodType,
}

/// Dalvik monitor operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MonitorAction {
    /// Acquire an object's monitor.
    Enter,
    /// Release an object's monitor.
    Exit,
}

/// Relational predicate used by Dalvik conditional branches.
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

/// Dalvik three-way comparison behavior.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Comparison {
    /// Float comparison producing `-1` for NaN.
    FloatNanLow,
    /// Float comparison producing `1` for NaN.
    FloatNanHigh,
    /// Double comparison producing `-1` for NaN.
    DoubleNanLow,
    /// Double comparison producing `1` for NaN.
    DoubleNanHigh,
    /// Signed long comparison.
    Long,
}

/// Direction of a Dalvik array-element operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ArrayAccess {
    /// Read an element.
    Get,
    /// Write an element.
    Put,
}

/// Dalvik array or field value category.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ArrayElementKind {
    /// Unconstrained single-register value.
    Single,
    /// Wide value.
    Wide,
    /// Object or array reference.
    Reference,
    /// Boolean value.
    Boolean,
    /// Signed byte value.
    Byte,
    /// Unsigned char value.
    Char,
    /// Signed short value.
    Short,
}

/// Dalvik instance/static field access mode.
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

/// Dalvik invocation dispatch mode after list/range normalization.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Invocation {
    /// Virtual dispatch.
    Virtual,
    /// Superclass dispatch.
    Super,
    /// Direct dispatch for constructors and private methods.
    Direct,
    /// Static dispatch.
    Static,
    /// Interface dispatch.
    Interface,
    /// Signature-polymorphic dispatch carrying a secondary prototype.
    Polymorphic,
    /// Dynamically resolved custom call site.
    Custom,
}

/// Dalvik unary arithmetic operator.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum UnaryOperator {
    /// Arithmetic negation.
    Negate,
    /// Bitwise complement.
    Not,
}

/// Dalvik primitive conversion.
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
    /// `int` narrowed and sign-extended as a byte.
    IntToByte,
    /// `int` narrowed and zero-extended as a char.
    IntToChar,
    /// `int` narrowed and sign-extended as a short.
    IntToShort,
}

/// Dalvik binary arithmetic or bitwise operator.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ArithmeticOperator {
    /// Addition.
    Add,
    /// Subtraction.
    Subtract,
    /// Literal minus register, used by `rsub-int` encodings.
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

/// Encoding-independent semantic category of one executable Dalvik operation.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum OperationKind {
    /// No operation.
    Nop,
    /// Copy between registers.
    Move(ValueKind),
    /// Copy the implicit invocation/array result into a register.
    MoveResult(ValueKind),
    /// Materialize the current caught exception.
    MoveException,
    /// Return, optionally consuming a value of the supplied category.
    Return(Option<ValueKind>),
    /// Produce a literal or indexed constant.
    Constant(ConstantKind),
    /// Enter or exit a monitor.
    Monitor(MonitorAction),
    /// Runtime reference cast check.
    CheckCast,
    /// Runtime instance test.
    InstanceOf,
    /// Read an array's length.
    ArrayLength,
    /// Allocate an uninitialized instance.
    NewInstance,
    /// Allocate an array with a runtime length.
    NewArray,
    /// Allocate and initialize an array from argument registers.
    FilledNewArray,
    /// Copy an encoded payload into an existing array.
    FillArrayData,
    /// Throw a reference.
    Throw,
    /// Unconditional direct branch.
    Jump,
    /// Dense or sparse integer switch dispatch.
    Switch,
    /// Three-way comparison.
    Compare(Comparison),
    /// Conditional comparison against another register.
    BranchPair(Relation),
    /// Conditional comparison against zero or null.
    BranchZero(Relation),
    /// Array-element access.
    Array {
        /// Read or write mode.
        access: ArrayAccess,
        /// Element category.
        element: ArrayElementKind,
    },
    /// Field access.
    Field {
        /// Instance/static and read/write mode.
        access: FieldAccess,
        /// Encoded value category.
        value: ArrayElementKind,
    },
    /// Method or custom call-site invocation.
    Invoke(Invocation),
    /// Unary operation over one primitive category.
    Unary {
        /// Unary operator.
        operator: UnaryOperator,
        /// Operand/result category.
        kind: ValueKind,
    },
    /// Primitive numeric conversion.
    Convert(Conversion),
    /// Binary register or register/literal operation.
    Binary {
        /// Operator.
        operator: ArithmeticOperator,
        /// Primary operand/result category.
        kind: ValueKind,
    },
}

/// One normalized Dalvik LLIL operand.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Operand {
    /// Typed register definition.
    Definition(RegisterOperand),
    /// Typed register use.
    Use(RegisterOperand),
    /// Sign-extended literal at its semantic width.
    Literal(i64),
    /// Absolute code-unit target.
    Target(u32),
    /// Identifier-table reference.
    Reference {
        /// Selected identifier table.
        kind: IndexKind,
        /// Native table index.
        index: u32,
    },
}

/// One normalized executable Dalvik operation and its complete register effects.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Operation {
    /// Encoding-independent operation category.
    pub kind: OperationKind,
    /// Definitions, uses, and non-register operands in semantic order.
    pub operands: Vec<Operand>,
    /// Typed register, implicit-result, exception, and payload semantics.
    pub semantics: InstructionSemantics,
}

/// One DEX instruction-stream payload retained as LLIL data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Payload {
    /// Dense switch table.
    PackedSwitch(PackedSwitchPayload),
    /// Sparse switch table.
    SparseSwitch(SparseSwitchPayload),
    /// Raw array element data.
    ArrayData(ArrayDataPayload),
}

/// Executable operation or payload item in a DEX LLIL stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstructionKind {
    /// Executable Dalvik semantics.
    Operation(Operation),
    /// Non-executable payload selected by another instruction.
    Payload(Payload),
}

/// Exact native DEX encoding retained beside normalized LLIL semantics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeEncoding {
    /// Exact native opcode/operand or payload shape.
    pub data: InstructionData,
}

/// One Dalvik LLIL stream item with exact native provenance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Instruction {
    /// Native code-unit offset.
    pub offset: u32,
    /// Normalized executable semantics or payload data.
    pub kind: InstructionKind,
    /// Exact source encoding used for reversible lowering.
    pub encoding: NativeEncoding,
}

//! JVM LLIL instruction and semantic operation model.

use crate::bytecode::{ArrayType, Opcode, Operand as NativeOperand};

/// JVM computational value category retained before shared MLIL type recovery.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ValueKind {
    /// JVM integer computational type, including narrow integral values.
    Integer,
    /// JVM `long` computational type.
    Long,
    /// JVM `float` computational type.
    Float,
    /// JVM `double` computational type.
    Double,
    /// Object, array, or null reference.
    Reference,
    /// Legacy subroutine return address.
    ReturnAddress,
    /// Value accepted by `astore` before frame analysis distinguishes a
    /// reference from a legacy subroutine return address.
    ReferenceOrReturnAddress,
}

/// Width category of a constant-pool value loaded onto the operand stack.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ConstantWidth {
    /// One category-one operand-stack value.
    Single,
    /// One category-two operand-stack value.
    Double,
}

/// A constant pushed by JVM bytecode.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Constant {
    /// The null reference.
    Null,
    /// An immediate integer value.
    Integer(i32),
    /// An immediate long value.
    Long(i64),
    /// Exact IEEE-754 single-precision bits.
    Float(u32),
    /// Exact IEEE-754 double-precision bits.
    Double(u64),
    /// A constant-pool entry whose recursive value is resolved with class context.
    Pool {
        /// Constant-pool index.
        index: u16,
        /// Operand-stack width required by the selected load instruction.
        width: ConstantWidth,
    },
}

/// Direction of a local-variable-slot operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LocalAccess {
    /// Push a local value onto the operand stack.
    Load,
    /// Pop an operand-stack value into a local slot.
    Store,
}

/// Direction of an array-element operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ArrayAccess {
    /// Read one array element.
    Load,
    /// Write one array element.
    Store,
}

/// JVM array-element category encoded by the array load/store opcode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ArrayElementKind {
    /// `int` element.
    Integer,
    /// `long` element.
    Long,
    /// `float` element.
    Float,
    /// `double` element.
    Double,
    /// Reference element.
    Reference,
    /// `byte` or `boolean` element, distinguished by the array type.
    ByteOrBoolean,
    /// `char` element.
    Char,
    /// `short` element.
    Short,
}

/// JVM operand-stack permutation or discard operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StackOperation {
    /// Discard one category-one value.
    Pop,
    /// Discard one category-two value or two category-one values.
    Pop2,
    /// Duplicate the top category-one value.
    Dup,
    /// Duplicate and insert beneath one category-one value.
    DupX1,
    /// Duplicate and insert beneath two operand slots.
    DupX2,
    /// Duplicate one category-two value or two category-one values.
    Dup2,
    /// Duplicate two operand slots and insert beneath one slot.
    Dup2X1,
    /// Duplicate two operand slots and insert beneath two slots.
    Dup2X2,
    /// Exchange the top two category-one values.
    Swap,
}

/// Binary arithmetic operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ArithmeticOperator {
    /// Addition.
    Add,
    /// Subtraction.
    Subtract,
    /// Multiplication.
    Multiply,
    /// Division.
    Divide,
    /// Remainder.
    Remainder,
}

/// Shift operation with JVM-masked shift counts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ShiftOperator {
    /// Left shift.
    Left,
    /// Arithmetic right shift.
    Right,
    /// Logical right shift.
    UnsignedRight,
}

/// Integral bitwise operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BitwiseOperator {
    /// Bitwise conjunction.
    And,
    /// Bitwise disjunction.
    Or,
    /// Bitwise exclusive-or.
    Xor,
}

/// JVM numeric conversion opcode.
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

/// Three-way JVM comparison semantics.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Comparison {
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

/// Relational predicate used by a conditional branch.
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

/// Operand interpretation of a JVM conditional branch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BranchCondition {
    /// Compare one integer value with zero.
    IntegerZero(Relation),
    /// Compare two integer values.
    IntegerPair(Relation),
    /// Compare two references for equality or inequality.
    ReferencePair(Relation),
    /// Compare one reference with null.
    ReferenceNull(Relation),
}

/// One normalized JVM switch case.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SwitchCase {
    /// Signed integer selector.
    pub key: i32,
    /// Absolute bytecode target.
    pub target: i32,
}

/// A normalized JVM switch table independent of dense/sparse encoding.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Switch {
    /// Default absolute bytecode target.
    pub default: i32,
    /// Ordered signed selector and absolute-target pairs.
    pub cases: Vec<SwitchCase>,
}

/// JVM field access mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FieldAccess {
    /// Read a static field.
    GetStatic,
    /// Write a static field.
    PutStatic,
    /// Read an instance field.
    GetInstance,
    /// Write an instance field.
    PutInstance,
}

/// JVM invocation dispatch mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Invocation {
    /// Virtual class dispatch.
    Virtual,
    /// Special dispatch for constructors, private methods, and super calls.
    Special,
    /// Static dispatch.
    Static,
    /// Interface dispatch. The redundant native argument-slot count remains in
    /// the exact encoding sidecar.
    Interface,
    /// Dynamically linked call-site dispatch.
    Dynamic,
}

/// JVM monitor operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MonitorAction {
    /// Acquire an object's monitor.
    Enter,
    /// Release an object's monitor.
    Exit,
}

/// Reserved implementation-specific JVM instruction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Intrinsic {
    /// Debugger breakpoint opcode.
    Breakpoint,
    /// First implementation-dependent opcode.
    ImplementationDependent1,
    /// Second implementation-dependent opcode.
    ImplementationDependent2,
}

/// One normalized JVM LLIL operation.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Operation {
    /// No operation.
    Nop,
    /// Push a constant.
    Constant(Constant),
    /// Transfer a value between a local slot and the operand stack.
    Local {
        /// Transfer direction.
        access: LocalAccess,
        /// JVM computational category.
        kind: ValueKind,
        /// Explicit local slot, including slots encoded through shorthand opcodes.
        index: u16,
    },
    /// Increment an integer local.
    IncrementLocal {
        /// Local slot.
        index: u16,
        /// Signed increment.
        amount: i16,
    },
    /// Load or store an array element.
    Array {
        /// Access direction.
        access: ArrayAccess,
        /// Encoded element category.
        element: ArrayElementKind,
    },
    /// Manipulate operand-stack layout.
    Stack(StackOperation),
    /// Apply a binary arithmetic operator.
    Arithmetic {
        /// Arithmetic operator.
        operator: ArithmeticOperator,
        /// Operand and result category.
        kind: ValueKind,
    },
    /// Negate a numeric value.
    Negate(ValueKind),
    /// Shift an integer or long value.
    Shift {
        /// Shift behavior.
        operator: ShiftOperator,
        /// Shifted value category.
        kind: ValueKind,
    },
    /// Apply an integral bitwise operator.
    Bitwise {
        /// Bitwise operator.
        operator: BitwiseOperator,
        /// Integer or long category.
        kind: ValueKind,
    },
    /// Convert a numeric value.
    Convert(Conversion),
    /// Produce an integer three-way comparison result.
    Compare(Comparison),
    /// Conditionally branch to an absolute bytecode target.
    Branch {
        /// Predicate and operand interpretation.
        condition: BranchCondition,
        /// Absolute bytecode target.
        target: i32,
    },
    /// Unconditionally branch to an absolute bytecode target.
    Jump {
        /// Absolute bytecode target.
        target: i32,
    },
    /// Invoke a legacy bytecode subroutine.
    SubroutineCall {
        /// Absolute subroutine entry target.
        target: i32,
    },
    /// Return from a legacy bytecode subroutine through a local slot.
    SubroutineReturn {
        /// Local slot holding the return address.
        local: u16,
    },
    /// Dispatch through a dense or sparse integer switch.
    Switch(Switch),
    /// Return from the method, optionally consuming a value.
    Return(Option<ValueKind>),
    /// Access a constant-pool field reference.
    Field {
        /// Static/instance and read/write mode.
        access: FieldAccess,
        /// Constant-pool field reference.
        index: u16,
    },
    /// Invoke a constant-pool method or dynamic call site.
    Invoke {
        /// Dispatch mode.
        kind: Invocation,
        /// Constant-pool member or call-site index.
        index: u16,
    },
    /// Allocate an uninitialized object.
    NewObject {
        /// Constant-pool class index.
        index: u16,
    },
    /// Allocate a primitive array.
    NewPrimitiveArray(ArrayType),
    /// Allocate a reference array.
    NewReferenceArray {
        /// Constant-pool component class index.
        index: u16,
    },
    /// Read an array's length.
    ArrayLength,
    /// Throw the top operand-stack reference.
    Throw,
    /// Check and retain the top reference against a class.
    CheckCast {
        /// Constant-pool class index.
        index: u16,
    },
    /// Test whether the top reference is an instance of a class.
    InstanceOf {
        /// Constant-pool class index.
        index: u16,
    },
    /// Enter or exit an object monitor.
    Monitor(MonitorAction),
    /// Allocate a multidimensional reference array.
    NewMultiArray {
        /// Constant-pool array class index.
        index: u16,
        /// Number of leading dimensions to allocate.
        dimensions: u8,
    },
    /// Reserved implementation-specific behavior.
    Intrinsic(Intrinsic),
}

/// Exact native encoding retained beside a normalized JVM operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeEncoding {
    /// Effective JVM opcode; `wide` is recorded separately.
    pub opcode: Opcode,
    /// Whether the native instruction used the `wide` prefix.
    pub wide: bool,
    /// Exact decoded instruction width, including switch padding.
    pub size: usize,
    /// Exact native operand shape and values.
    pub operand: NativeOperand,
}

/// One JVM LLIL instruction with exact native provenance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Instruction {
    /// Native bytecode offset.
    pub offset: usize,
    /// Normalized JVM semantics.
    pub operation: Operation,
    /// Exact source encoding used for reversible lowering.
    pub encoding: NativeEncoding,
}

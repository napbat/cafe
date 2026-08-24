//! Typed JVM opcode and primitive-array encodings.

macro_rules! define_opcodes {
    ($($variant:ident = $byte:expr => $mnemonic:literal),+ $(,)?) => {
        /// A standardized JVM instruction opcode.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
        #[repr(u8)]
        pub enum Opcode {
            $(
                #[doc = concat!(
                    "The JVM `",
                    $mnemonic,
                    "` instruction (opcode ",
                    stringify!($byte),
                    ")."
                )]
                $variant = $byte
            ),+
        }

        impl Opcode {
            /// Every defined opcode in encoded order.
            pub const ALL: &[Self] = &[$(Self::$variant),+];

            /// Converts a class-file byte into a known opcode.
            #[must_use]
            #[allow(clippy::too_many_lines)]
            pub const fn from_byte(value: u8) -> Option<Self> {
                match value {
                    $($byte => Some(Self::$variant),)+
                    _ => None,
                }
            }

            /// Returns the encoded opcode byte.
            #[must_use]
            pub const fn byte(self) -> u8 {
                self as u8
            }

            /// Returns the standard JVM mnemonic.
            #[must_use]
            #[allow(clippy::too_many_lines)]
            pub const fn mnemonic(self) -> &'static str {
                match self {
                    $(Self::$variant => $mnemonic),+
                }
            }
        }
    };
}

define_opcodes! {
    Nop = 0x00 => "nop",
    AConstNull = 0x01 => "aconst_null",
    IConstM1 = 0x02 => "iconst_m1",
    IConst0 = 0x03 => "iconst_0",
    IConst1 = 0x04 => "iconst_1",
    IConst2 = 0x05 => "iconst_2",
    IConst3 = 0x06 => "iconst_3",
    IConst4 = 0x07 => "iconst_4",
    IConst5 = 0x08 => "iconst_5",
    LConst0 = 0x09 => "lconst_0",
    LConst1 = 0x0a => "lconst_1",
    FConst0 = 0x0b => "fconst_0",
    FConst1 = 0x0c => "fconst_1",
    FConst2 = 0x0d => "fconst_2",
    DConst0 = 0x0e => "dconst_0",
    DConst1 = 0x0f => "dconst_1",
    BiPush = 0x10 => "bipush",
    SiPush = 0x11 => "sipush",
    Ldc = 0x12 => "ldc",
    LdcW = 0x13 => "ldc_w",
    Ldc2W = 0x14 => "ldc2_w",
    ILoad = 0x15 => "iload",
    LLoad = 0x16 => "lload",
    FLoad = 0x17 => "fload",
    DLoad = 0x18 => "dload",
    ALoad = 0x19 => "aload",
    ILoad0 = 0x1a => "iload_0",
    ILoad1 = 0x1b => "iload_1",
    ILoad2 = 0x1c => "iload_2",
    ILoad3 = 0x1d => "iload_3",
    LLoad0 = 0x1e => "lload_0",
    LLoad1 = 0x1f => "lload_1",
    LLoad2 = 0x20 => "lload_2",
    LLoad3 = 0x21 => "lload_3",
    FLoad0 = 0x22 => "fload_0",
    FLoad1 = 0x23 => "fload_1",
    FLoad2 = 0x24 => "fload_2",
    FLoad3 = 0x25 => "fload_3",
    DLoad0 = 0x26 => "dload_0",
    DLoad1 = 0x27 => "dload_1",
    DLoad2 = 0x28 => "dload_2",
    DLoad3 = 0x29 => "dload_3",
    ALoad0 = 0x2a => "aload_0",
    ALoad1 = 0x2b => "aload_1",
    ALoad2 = 0x2c => "aload_2",
    ALoad3 = 0x2d => "aload_3",
    IALoad = 0x2e => "iaload",
    LALoad = 0x2f => "laload",
    FALoad = 0x30 => "faload",
    DALoad = 0x31 => "daload",
    AALoad = 0x32 => "aaload",
    BALoad = 0x33 => "baload",
    CALoad = 0x34 => "caload",
    SALoad = 0x35 => "saload",
    IStore = 0x36 => "istore",
    LStore = 0x37 => "lstore",
    FStore = 0x38 => "fstore",
    DStore = 0x39 => "dstore",
    AStore = 0x3a => "astore",
    IStore0 = 0x3b => "istore_0",
    IStore1 = 0x3c => "istore_1",
    IStore2 = 0x3d => "istore_2",
    IStore3 = 0x3e => "istore_3",
    LStore0 = 0x3f => "lstore_0",
    LStore1 = 0x40 => "lstore_1",
    LStore2 = 0x41 => "lstore_2",
    LStore3 = 0x42 => "lstore_3",
    FStore0 = 0x43 => "fstore_0",
    FStore1 = 0x44 => "fstore_1",
    FStore2 = 0x45 => "fstore_2",
    FStore3 = 0x46 => "fstore_3",
    DStore0 = 0x47 => "dstore_0",
    DStore1 = 0x48 => "dstore_1",
    DStore2 = 0x49 => "dstore_2",
    DStore3 = 0x4a => "dstore_3",
    AStore0 = 0x4b => "astore_0",
    AStore1 = 0x4c => "astore_1",
    AStore2 = 0x4d => "astore_2",
    AStore3 = 0x4e => "astore_3",
    IAStore = 0x4f => "iastore",
    LAStore = 0x50 => "lastore",
    FAStore = 0x51 => "fastore",
    DAStore = 0x52 => "dastore",
    AAStore = 0x53 => "aastore",
    BAStore = 0x54 => "bastore",
    CAStore = 0x55 => "castore",
    SAStore = 0x56 => "sastore",
    Pop = 0x57 => "pop",
    Pop2 = 0x58 => "pop2",
    Dup = 0x59 => "dup",
    DupX1 = 0x5a => "dup_x1",
    DupX2 = 0x5b => "dup_x2",
    Dup2 = 0x5c => "dup2",
    Dup2X1 = 0x5d => "dup2_x1",
    Dup2X2 = 0x5e => "dup2_x2",
    Swap = 0x5f => "swap",
    IAdd = 0x60 => "iadd",
    LAdd = 0x61 => "ladd",
    FAdd = 0x62 => "fadd",
    DAdd = 0x63 => "dadd",
    ISub = 0x64 => "isub",
    LSub = 0x65 => "lsub",
    FSub = 0x66 => "fsub",
    DSub = 0x67 => "dsub",
    IMul = 0x68 => "imul",
    LMul = 0x69 => "lmul",
    FMul = 0x6a => "fmul",
    DMul = 0x6b => "dmul",
    IDiv = 0x6c => "idiv",
    LDiv = 0x6d => "ldiv",
    FDiv = 0x6e => "fdiv",
    DDiv = 0x6f => "ddiv",
    IRem = 0x70 => "irem",
    LRem = 0x71 => "lrem",
    FRem = 0x72 => "frem",
    DRem = 0x73 => "drem",
    INeg = 0x74 => "ineg",
    LNeg = 0x75 => "lneg",
    FNeg = 0x76 => "fneg",
    DNeg = 0x77 => "dneg",
    IShl = 0x78 => "ishl",
    LShl = 0x79 => "lshl",
    IShr = 0x7a => "ishr",
    LShr = 0x7b => "lshr",
    IUShr = 0x7c => "iushr",
    LUShr = 0x7d => "lushr",
    IAnd = 0x7e => "iand",
    LAnd = 0x7f => "land",
    IOr = 0x80 => "ior",
    LOr = 0x81 => "lor",
    IXor = 0x82 => "ixor",
    LXor = 0x83 => "lxor",
    IInc = 0x84 => "iinc",
    I2L = 0x85 => "i2l",
    I2F = 0x86 => "i2f",
    I2D = 0x87 => "i2d",
    L2I = 0x88 => "l2i",
    L2F = 0x89 => "l2f",
    L2D = 0x8a => "l2d",
    F2I = 0x8b => "f2i",
    F2L = 0x8c => "f2l",
    F2D = 0x8d => "f2d",
    D2I = 0x8e => "d2i",
    D2L = 0x8f => "d2l",
    D2F = 0x90 => "d2f",
    I2B = 0x91 => "i2b",
    I2C = 0x92 => "i2c",
    I2S = 0x93 => "i2s",
    LCmp = 0x94 => "lcmp",
    FCmpL = 0x95 => "fcmpl",
    FCmpG = 0x96 => "fcmpg",
    DCmpL = 0x97 => "dcmpl",
    DCmpG = 0x98 => "dcmpg",
    IfEq = 0x99 => "ifeq",
    IfNe = 0x9a => "ifne",
    IfLt = 0x9b => "iflt",
    IfGe = 0x9c => "ifge",
    IfGt = 0x9d => "ifgt",
    IfLe = 0x9e => "ifle",
    IfICmpEq = 0x9f => "if_icmpeq",
    IfICmpNe = 0xa0 => "if_icmpne",
    IfICmpLt = 0xa1 => "if_icmplt",
    IfICmpGe = 0xa2 => "if_icmpge",
    IfICmpGt = 0xa3 => "if_icmpgt",
    IfICmpLe = 0xa4 => "if_icmple",
    IfACmpEq = 0xa5 => "if_acmpeq",
    IfACmpNe = 0xa6 => "if_acmpne",
    Goto = 0xa7 => "goto",
    Jsr = 0xa8 => "jsr",
    Ret = 0xa9 => "ret",
    TableSwitch = 0xaa => "tableswitch",
    LookupSwitch = 0xab => "lookupswitch",
    IReturn = 0xac => "ireturn",
    LReturn = 0xad => "lreturn",
    FReturn = 0xae => "freturn",
    DReturn = 0xaf => "dreturn",
    AReturn = 0xb0 => "areturn",
    Return = 0xb1 => "return",
    GetStatic = 0xb2 => "getstatic",
    PutStatic = 0xb3 => "putstatic",
    GetField = 0xb4 => "getfield",
    PutField = 0xb5 => "putfield",
    InvokeVirtual = 0xb6 => "invokevirtual",
    InvokeSpecial = 0xb7 => "invokespecial",
    InvokeStatic = 0xb8 => "invokestatic",
    InvokeInterface = 0xb9 => "invokeinterface",
    InvokeDynamic = 0xba => "invokedynamic",
    New = 0xbb => "new",
    NewArray = 0xbc => "newarray",
    ANewArray = 0xbd => "anewarray",
    ArrayLength = 0xbe => "arraylength",
    AThrow = 0xbf => "athrow",
    CheckCast = 0xc0 => "checkcast",
    InstanceOf = 0xc1 => "instanceof",
    MonitorEnter = 0xc2 => "monitorenter",
    MonitorExit = 0xc3 => "monitorexit",
    Wide = 0xc4 => "wide",
    MultiANewArray = 0xc5 => "multianewarray",
    IfNull = 0xc6 => "ifnull",
    IfNonNull = 0xc7 => "ifnonnull",
    GotoW = 0xc8 => "goto_w",
    JsrW = 0xc9 => "jsr_w",
    Breakpoint = 0xca => "breakpoint",
    ImpDep1 = 0xfe => "impdep1",
    ImpDep2 = 0xff => "impdep2",
}

impl Opcode {
    /// Returns whether this opcode is a conditional control-flow branch.
    #[must_use]
    pub const fn is_conditional_branch(self) -> bool {
        matches!(
            self,
            Self::IfEq
                | Self::IfNe
                | Self::IfLt
                | Self::IfGe
                | Self::IfGt
                | Self::IfLe
                | Self::IfICmpEq
                | Self::IfICmpNe
                | Self::IfICmpLt
                | Self::IfICmpGe
                | Self::IfICmpGt
                | Self::IfICmpLe
                | Self::IfACmpEq
                | Self::IfACmpNe
                | Self::IfNull
                | Self::IfNonNull
        )
    }

    /// Returns the conditional branch with the opposite predicate.
    ///
    /// This is used when a short conditional branch must be relaxed into an
    /// inverted short branch around a `goto_w` instruction.
    #[must_use]
    pub const fn inverted_conditional(self) -> Option<Self> {
        match self {
            Self::IfEq => Some(Self::IfNe),
            Self::IfNe => Some(Self::IfEq),
            Self::IfLt => Some(Self::IfGe),
            Self::IfGe => Some(Self::IfLt),
            Self::IfGt => Some(Self::IfLe),
            Self::IfLe => Some(Self::IfGt),
            Self::IfICmpEq => Some(Self::IfICmpNe),
            Self::IfICmpNe => Some(Self::IfICmpEq),
            Self::IfICmpLt => Some(Self::IfICmpGe),
            Self::IfICmpGe => Some(Self::IfICmpLt),
            Self::IfICmpGt => Some(Self::IfICmpLe),
            Self::IfICmpLe => Some(Self::IfICmpGt),
            Self::IfACmpEq => Some(Self::IfACmpNe),
            Self::IfACmpNe => Some(Self::IfACmpEq),
            Self::IfNull => Some(Self::IfNonNull),
            Self::IfNonNull => Some(Self::IfNull),
            _ => None,
        }
    }

    /// Returns whether this opcode is an unconditional direct branch.
    #[must_use]
    pub const fn is_unconditional_branch(self) -> bool {
        matches!(self, Self::Goto | Self::GotoW | Self::Jsr | Self::JsrW)
    }

    /// Returns whether this opcode terminates a method normally.
    #[must_use]
    pub const fn is_return(self) -> bool {
        matches!(
            self,
            Self::IReturn
                | Self::LReturn
                | Self::FReturn
                | Self::DReturn
                | Self::AReturn
                | Self::Return
        )
    }

    /// Returns whether this opcode performs integer switch dispatch.
    #[must_use]
    pub const fn is_switch(self) -> bool {
        matches!(self, Self::TableSwitch | Self::LookupSwitch)
    }
}

macro_rules! define_array_types {
    ($($(#[$metadata:meta])* $variant:ident = $byte:expr => $name:literal),+ $(,)?) => {
        /// Primitive element types accepted by the JVM `newarray` instruction.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
        #[repr(u8)]
        pub enum ArrayType {
            $(
                $(#[$metadata])*
                $variant = $byte,
            )+
        }

        impl ArrayType {
            /// Every primitive array type in encoded order.
            pub const ALL: &[Self] = &[$(Self::$variant),+];

            /// Converts the encoded `atype` byte to a primitive array type.
            #[must_use]
            pub const fn from_byte(value: u8) -> Option<Self> {
                match value {
                    $($byte => Some(Self::$variant),)+
                    _ => None,
                }
            }

            /// Returns the encoded `atype` byte.
            #[must_use]
            pub const fn byte(self) -> u8 {
                self as u8
            }

            /// Returns the Java primitive type name.
            #[must_use]
            pub const fn name(self) -> &'static str {
                match self {
                    $(Self::$variant => $name),+
                }
            }
        }
    };
}

define_array_types! {
    /// Java `boolean`.
    Boolean = 4 => "boolean",
    /// Java `char`.
    Char = 5 => "char",
    /// Java `float`.
    Float = 6 => "float",
    /// Java `double`.
    Double = 7 => "double",
    /// Java `byte`.
    Byte = 8 => "byte",
    /// Java `short`.
    Short = 9 => "short",
    /// Java `int`.
    Int = 10 => "int",
    /// Java `long`.
    Long = 11 => "long",
}

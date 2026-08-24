//! Complete standard Dalvik opcode and instruction-format table.

/// Binary layout of a standard Dalvik instruction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum InstructionFormat {
    /// One code unit, no operands (`10x`).
    F10x,
    /// Two four-bit registers (`12x`).
    F12x,
    /// Four-bit register and signed four-bit literal (`11n`).
    F11n,
    /// One eight-bit register (`11x`).
    F11x,
    /// Signed eight-bit branch (`10t`).
    F10t,
    /// Signed sixteen-bit branch (`20t`).
    F20t,
    /// Eight-bit and sixteen-bit registers (`22x`).
    F22x,
    /// Eight-bit register and signed sixteen-bit branch (`21t`).
    F21t,
    /// Eight-bit register and signed sixteen-bit literal (`21s`).
    F21s,
    /// Eight-bit register and high sixteen literal bits (`21h`).
    F21h,
    /// Eight-bit register and sixteen-bit index (`21c`).
    F21c,
    /// Three eight-bit registers (`23x`).
    F23x,
    /// Two four-bit registers and signed sixteen-bit branch (`22t`).
    F22t,
    /// Two four-bit registers and signed sixteen-bit literal (`22s`).
    F22s,
    /// Two four-bit registers and sixteen-bit index (`22c`).
    F22c,
    /// Eight-bit register, eight-bit register, and signed byte literal (`22b`).
    F22b,
    /// Signed thirty-two-bit branch (`30t`).
    F30t,
    /// Two sixteen-bit registers (`32x`).
    F32x,
    /// Eight-bit register and signed thirty-two-bit literal (`31i`).
    F31i,
    /// Eight-bit register and signed thirty-two-bit payload branch (`31t`).
    F31t,
    /// Eight-bit register and thirty-two-bit index (`31c`).
    F31c,
    /// Up to five four-bit registers and a sixteen-bit index (`35c`).
    F35c,
    /// Register range and a sixteen-bit index (`3rc`).
    F3rc,
    /// Register list with method and prototype indices (`45cc`).
    F45cc,
    /// Register range with method and prototype indices (`4rcc`).
    F4rcc,
    /// Eight-bit register and signed sixty-four-bit literal (`51l`).
    F51l,
}

impl InstructionFormat {
    /// Returns the fixed encoded width in 16-bit code units.
    #[must_use]
    pub const fn code_units(self) -> u32 {
        match self {
            Self::F10x | Self::F12x | Self::F11n | Self::F11x | Self::F10t => 1,
            Self::F20t
            | Self::F22x
            | Self::F21t
            | Self::F21s
            | Self::F21h
            | Self::F21c
            | Self::F23x
            | Self::F22t
            | Self::F22s
            | Self::F22c
            | Self::F22b => 2,
            Self::F30t
            | Self::F32x
            | Self::F31i
            | Self::F31t
            | Self::F31c
            | Self::F35c
            | Self::F3rc => 3,
            Self::F45cc | Self::F4rcc => 4,
            Self::F51l => 5,
        }
    }
}

/// Identifier table selected by an indexed opcode operand.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IndexKind {
    /// String identifier.
    String,
    /// Type identifier.
    Type,
    /// Field identifier.
    Field,
    /// Method identifier.
    Method,
    /// Prototype identifier.
    Prototype,
    /// Call-site identifier.
    CallSite,
    /// Method-handle identifier.
    MethodHandle,
}

macro_rules! define_opcodes {
    ($($variant:ident = $byte:literal, $mnemonic:literal, $format:ident $(, $index:ident)?;)+) => {
        /// Standard, non-optimized Dalvik opcode.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
        #[repr(u8)]
        pub enum Opcode {
            $(
                #[doc = concat!("`", $mnemonic, "` (`0x", stringify!($byte), "`).")]
                $variant = $byte,
            )+
        }

        impl Opcode {
            /// Every defined standard opcode in numeric order.
            pub const ALL: &[Self] = &[$(Self::$variant),+];

            /// Parses an opcode byte, rejecting unused and optimized encodings.
            #[must_use]
            pub const fn from_byte(byte: u8) -> Option<Self> {
                match byte {
                    $($byte => Some(Self::$variant),)+
                    _ => None,
                }
            }

            /// Returns the native opcode byte.
            #[must_use]
            pub const fn byte(self) -> u8 {
                self as u8
            }

            /// Returns the standardized mnemonic.
            #[must_use]
            pub const fn mnemonic(self) -> &'static str {
                match self {
                    $(Self::$variant => $mnemonic,)+
                }
            }

            /// Returns the encoded instruction format.
            #[must_use]
            pub const fn format(self) -> InstructionFormat {
                match self {
                    $(Self::$variant => InstructionFormat::$format,)+
                }
            }

            /// Returns the identifier-table kind of the primary index operand.
            #[must_use]
            pub const fn index_kind(self) -> Option<IndexKind> {
                match self {
                    $(Self::$variant => define_opcodes!(@index $($index)?),)+
                }
            }
        }
    };
    (@index) => { None };
    (@index $index:ident) => { Some(IndexKind::$index) };
}

define_opcodes! {
    Nop = 0x00, "nop", F10x;
    Move = 0x01, "move", F12x;
    MoveFrom16 = 0x02, "move/from16", F22x;
    Move16 = 0x03, "move/16", F32x;
    MoveWide = 0x04, "move-wide", F12x;
    MoveWideFrom16 = 0x05, "move-wide/from16", F22x;
    MoveWide16 = 0x06, "move-wide/16", F32x;
    MoveObject = 0x07, "move-object", F12x;
    MoveObjectFrom16 = 0x08, "move-object/from16", F22x;
    MoveObject16 = 0x09, "move-object/16", F32x;
    MoveResult = 0x0a, "move-result", F11x;
    MoveResultWide = 0x0b, "move-result-wide", F11x;
    MoveResultObject = 0x0c, "move-result-object", F11x;
    MoveException = 0x0d, "move-exception", F11x;
    ReturnVoid = 0x0e, "return-void", F10x;
    Return = 0x0f, "return", F11x;
    ReturnWide = 0x10, "return-wide", F11x;
    ReturnObject = 0x11, "return-object", F11x;
    Const4 = 0x12, "const/4", F11n;
    Const16 = 0x13, "const/16", F21s;
    Const = 0x14, "const", F31i;
    ConstHigh16 = 0x15, "const/high16", F21h;
    ConstWide16 = 0x16, "const-wide/16", F21s;
    ConstWide32 = 0x17, "const-wide/32", F31i;
    ConstWide = 0x18, "const-wide", F51l;
    ConstWideHigh16 = 0x19, "const-wide/high16", F21h;
    ConstString = 0x1a, "const-string", F21c, String;
    ConstStringJumbo = 0x1b, "const-string/jumbo", F31c, String;
    ConstClass = 0x1c, "const-class", F21c, Type;
    MonitorEnter = 0x1d, "monitor-enter", F11x;
    MonitorExit = 0x1e, "monitor-exit", F11x;
    CheckCast = 0x1f, "check-cast", F21c, Type;
    InstanceOf = 0x20, "instance-of", F22c, Type;
    ArrayLength = 0x21, "array-length", F12x;
    NewInstance = 0x22, "new-instance", F21c, Type;
    NewArray = 0x23, "new-array", F22c, Type;
    FilledNewArray = 0x24, "filled-new-array", F35c, Type;
    FilledNewArrayRange = 0x25, "filled-new-array/range", F3rc, Type;
    FillArrayData = 0x26, "fill-array-data", F31t;
    Throw = 0x27, "throw", F11x;
    Goto = 0x28, "goto", F10t;
    Goto16 = 0x29, "goto/16", F20t;
    Goto32 = 0x2a, "goto/32", F30t;
    PackedSwitch = 0x2b, "packed-switch", F31t;
    SparseSwitch = 0x2c, "sparse-switch", F31t;
    CmplFloat = 0x2d, "cmpl-float", F23x;
    CmpgFloat = 0x2e, "cmpg-float", F23x;
    CmplDouble = 0x2f, "cmpl-double", F23x;
    CmpgDouble = 0x30, "cmpg-double", F23x;
    CmpLong = 0x31, "cmp-long", F23x;
    IfEq = 0x32, "if-eq", F22t;
    IfNe = 0x33, "if-ne", F22t;
    IfLt = 0x34, "if-lt", F22t;
    IfGe = 0x35, "if-ge", F22t;
    IfGt = 0x36, "if-gt", F22t;
    IfLe = 0x37, "if-le", F22t;
    IfEqz = 0x38, "if-eqz", F21t;
    IfNez = 0x39, "if-nez", F21t;
    IfLtz = 0x3a, "if-ltz", F21t;
    IfGez = 0x3b, "if-gez", F21t;
    IfGtz = 0x3c, "if-gtz", F21t;
    IfLez = 0x3d, "if-lez", F21t;
    Aget = 0x44, "aget", F23x;
    AgetWide = 0x45, "aget-wide", F23x;
    AgetObject = 0x46, "aget-object", F23x;
    AgetBoolean = 0x47, "aget-boolean", F23x;
    AgetByte = 0x48, "aget-byte", F23x;
    AgetChar = 0x49, "aget-char", F23x;
    AgetShort = 0x4a, "aget-short", F23x;
    Aput = 0x4b, "aput", F23x;
    AputWide = 0x4c, "aput-wide", F23x;
    AputObject = 0x4d, "aput-object", F23x;
    AputBoolean = 0x4e, "aput-boolean", F23x;
    AputByte = 0x4f, "aput-byte", F23x;
    AputChar = 0x50, "aput-char", F23x;
    AputShort = 0x51, "aput-short", F23x;
    Iget = 0x52, "iget", F22c, Field;
    IgetWide = 0x53, "iget-wide", F22c, Field;
    IgetObject = 0x54, "iget-object", F22c, Field;
    IgetBoolean = 0x55, "iget-boolean", F22c, Field;
    IgetByte = 0x56, "iget-byte", F22c, Field;
    IgetChar = 0x57, "iget-char", F22c, Field;
    IgetShort = 0x58, "iget-short", F22c, Field;
    Iput = 0x59, "iput", F22c, Field;
    IputWide = 0x5a, "iput-wide", F22c, Field;
    IputObject = 0x5b, "iput-object", F22c, Field;
    IputBoolean = 0x5c, "iput-boolean", F22c, Field;
    IputByte = 0x5d, "iput-byte", F22c, Field;
    IputChar = 0x5e, "iput-char", F22c, Field;
    IputShort = 0x5f, "iput-short", F22c, Field;
    Sget = 0x60, "sget", F21c, Field;
    SgetWide = 0x61, "sget-wide", F21c, Field;
    SgetObject = 0x62, "sget-object", F21c, Field;
    SgetBoolean = 0x63, "sget-boolean", F21c, Field;
    SgetByte = 0x64, "sget-byte", F21c, Field;
    SgetChar = 0x65, "sget-char", F21c, Field;
    SgetShort = 0x66, "sget-short", F21c, Field;
    Sput = 0x67, "sput", F21c, Field;
    SputWide = 0x68, "sput-wide", F21c, Field;
    SputObject = 0x69, "sput-object", F21c, Field;
    SputBoolean = 0x6a, "sput-boolean", F21c, Field;
    SputByte = 0x6b, "sput-byte", F21c, Field;
    SputChar = 0x6c, "sput-char", F21c, Field;
    SputShort = 0x6d, "sput-short", F21c, Field;
    InvokeVirtual = 0x6e, "invoke-virtual", F35c, Method;
    InvokeSuper = 0x6f, "invoke-super", F35c, Method;
    InvokeDirect = 0x70, "invoke-direct", F35c, Method;
    InvokeStatic = 0x71, "invoke-static", F35c, Method;
    InvokeInterface = 0x72, "invoke-interface", F35c, Method;
    InvokeVirtualRange = 0x74, "invoke-virtual/range", F3rc, Method;
    InvokeSuperRange = 0x75, "invoke-super/range", F3rc, Method;
    InvokeDirectRange = 0x76, "invoke-direct/range", F3rc, Method;
    InvokeStaticRange = 0x77, "invoke-static/range", F3rc, Method;
    InvokeInterfaceRange = 0x78, "invoke-interface/range", F3rc, Method;
    NegInt = 0x7b, "neg-int", F12x;
    NotInt = 0x7c, "not-int", F12x;
    NegLong = 0x7d, "neg-long", F12x;
    NotLong = 0x7e, "not-long", F12x;
    NegFloat = 0x7f, "neg-float", F12x;
    NegDouble = 0x80, "neg-double", F12x;
    IntToLong = 0x81, "int-to-long", F12x;
    IntToFloat = 0x82, "int-to-float", F12x;
    IntToDouble = 0x83, "int-to-double", F12x;
    LongToInt = 0x84, "long-to-int", F12x;
    LongToFloat = 0x85, "long-to-float", F12x;
    LongToDouble = 0x86, "long-to-double", F12x;
    FloatToInt = 0x87, "float-to-int", F12x;
    FloatToLong = 0x88, "float-to-long", F12x;
    FloatToDouble = 0x89, "float-to-double", F12x;
    DoubleToInt = 0x8a, "double-to-int", F12x;
    DoubleToLong = 0x8b, "double-to-long", F12x;
    DoubleToFloat = 0x8c, "double-to-float", F12x;
    IntToByte = 0x8d, "int-to-byte", F12x;
    IntToChar = 0x8e, "int-to-char", F12x;
    IntToShort = 0x8f, "int-to-short", F12x;
    AddInt = 0x90, "add-int", F23x;
    SubInt = 0x91, "sub-int", F23x;
    MulInt = 0x92, "mul-int", F23x;
    DivInt = 0x93, "div-int", F23x;
    RemInt = 0x94, "rem-int", F23x;
    AndInt = 0x95, "and-int", F23x;
    OrInt = 0x96, "or-int", F23x;
    XorInt = 0x97, "xor-int", F23x;
    ShlInt = 0x98, "shl-int", F23x;
    ShrInt = 0x99, "shr-int", F23x;
    UshrInt = 0x9a, "ushr-int", F23x;
    AddLong = 0x9b, "add-long", F23x;
    SubLong = 0x9c, "sub-long", F23x;
    MulLong = 0x9d, "mul-long", F23x;
    DivLong = 0x9e, "div-long", F23x;
    RemLong = 0x9f, "rem-long", F23x;
    AndLong = 0xa0, "and-long", F23x;
    OrLong = 0xa1, "or-long", F23x;
    XorLong = 0xa2, "xor-long", F23x;
    ShlLong = 0xa3, "shl-long", F23x;
    ShrLong = 0xa4, "shr-long", F23x;
    UshrLong = 0xa5, "ushr-long", F23x;
    AddFloat = 0xa6, "add-float", F23x;
    SubFloat = 0xa7, "sub-float", F23x;
    MulFloat = 0xa8, "mul-float", F23x;
    DivFloat = 0xa9, "div-float", F23x;
    RemFloat = 0xaa, "rem-float", F23x;
    AddDouble = 0xab, "add-double", F23x;
    SubDouble = 0xac, "sub-double", F23x;
    MulDouble = 0xad, "mul-double", F23x;
    DivDouble = 0xae, "div-double", F23x;
    RemDouble = 0xaf, "rem-double", F23x;
    AddInt2Addr = 0xb0, "add-int/2addr", F12x;
    SubInt2Addr = 0xb1, "sub-int/2addr", F12x;
    MulInt2Addr = 0xb2, "mul-int/2addr", F12x;
    DivInt2Addr = 0xb3, "div-int/2addr", F12x;
    RemInt2Addr = 0xb4, "rem-int/2addr", F12x;
    AndInt2Addr = 0xb5, "and-int/2addr", F12x;
    OrInt2Addr = 0xb6, "or-int/2addr", F12x;
    XorInt2Addr = 0xb7, "xor-int/2addr", F12x;
    ShlInt2Addr = 0xb8, "shl-int/2addr", F12x;
    ShrInt2Addr = 0xb9, "shr-int/2addr", F12x;
    UshrInt2Addr = 0xba, "ushr-int/2addr", F12x;
    AddLong2Addr = 0xbb, "add-long/2addr", F12x;
    SubLong2Addr = 0xbc, "sub-long/2addr", F12x;
    MulLong2Addr = 0xbd, "mul-long/2addr", F12x;
    DivLong2Addr = 0xbe, "div-long/2addr", F12x;
    RemLong2Addr = 0xbf, "rem-long/2addr", F12x;
    AndLong2Addr = 0xc0, "and-long/2addr", F12x;
    OrLong2Addr = 0xc1, "or-long/2addr", F12x;
    XorLong2Addr = 0xc2, "xor-long/2addr", F12x;
    ShlLong2Addr = 0xc3, "shl-long/2addr", F12x;
    ShrLong2Addr = 0xc4, "shr-long/2addr", F12x;
    UshrLong2Addr = 0xc5, "ushr-long/2addr", F12x;
    AddFloat2Addr = 0xc6, "add-float/2addr", F12x;
    SubFloat2Addr = 0xc7, "sub-float/2addr", F12x;
    MulFloat2Addr = 0xc8, "mul-float/2addr", F12x;
    DivFloat2Addr = 0xc9, "div-float/2addr", F12x;
    RemFloat2Addr = 0xca, "rem-float/2addr", F12x;
    AddDouble2Addr = 0xcb, "add-double/2addr", F12x;
    SubDouble2Addr = 0xcc, "sub-double/2addr", F12x;
    MulDouble2Addr = 0xcd, "mul-double/2addr", F12x;
    DivDouble2Addr = 0xce, "div-double/2addr", F12x;
    RemDouble2Addr = 0xcf, "rem-double/2addr", F12x;
    AddIntLit16 = 0xd0, "add-int/lit16", F22s;
    RsubInt = 0xd1, "rsub-int", F22s;
    MulIntLit16 = 0xd2, "mul-int/lit16", F22s;
    DivIntLit16 = 0xd3, "div-int/lit16", F22s;
    RemIntLit16 = 0xd4, "rem-int/lit16", F22s;
    AndIntLit16 = 0xd5, "and-int/lit16", F22s;
    OrIntLit16 = 0xd6, "or-int/lit16", F22s;
    XorIntLit16 = 0xd7, "xor-int/lit16", F22s;
    AddIntLit8 = 0xd8, "add-int/lit8", F22b;
    RsubIntLit8 = 0xd9, "rsub-int/lit8", F22b;
    MulIntLit8 = 0xda, "mul-int/lit8", F22b;
    DivIntLit8 = 0xdb, "div-int/lit8", F22b;
    RemIntLit8 = 0xdc, "rem-int/lit8", F22b;
    AndIntLit8 = 0xdd, "and-int/lit8", F22b;
    OrIntLit8 = 0xde, "or-int/lit8", F22b;
    XorIntLit8 = 0xdf, "xor-int/lit8", F22b;
    ShlIntLit8 = 0xe0, "shl-int/lit8", F22b;
    ShrIntLit8 = 0xe1, "shr-int/lit8", F22b;
    UshrIntLit8 = 0xe2, "ushr-int/lit8", F22b;
    InvokePolymorphic = 0xfa, "invoke-polymorphic", F45cc, Method;
    InvokePolymorphicRange = 0xfb, "invoke-polymorphic/range", F4rcc, Method;
    InvokeCustom = 0xfc, "invoke-custom", F35c, CallSite;
    InvokeCustomRange = 0xfd, "invoke-custom/range", F3rc, CallSite;
    ConstMethodHandle = 0xfe, "const-method-handle", F21c, MethodHandle;
    ConstMethodType = 0xff, "const-method-type", F21c, Prototype;
}

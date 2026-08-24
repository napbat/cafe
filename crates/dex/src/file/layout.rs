//! Typed widths, alignments, and field positions from the DEX file format.

/// Required byte alignment of a DEX structure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Alignment {
    Byte,
    CodeUnit,
    Word,
}

impl Alignment {
    pub(crate) const fn bytes_u32(self) -> u32 {
        match self {
            Self::Byte => 1,
            Self::CodeUnit => 2,
            Self::Word => 4,
        }
    }

    pub(crate) const fn bytes(self) -> usize {
        match self {
            Self::Byte => 1,
            Self::CodeUnit => 2,
            Self::Word => 4,
        }
    }
}

/// Width of one fixed or minimally encoded DEX item.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ItemWidth(usize);

impl ItemWidth {
    pub(crate) const BYTE: Self = Self(1);
    pub(crate) const CODE_UNIT: Self = Self(2);
    pub(crate) const WORD: Self = Self(4);
    pub(crate) const ENCODED_FIELD_MINIMUM: Self = Self(2);
    pub(crate) const ENCODED_METHOD_MINIMUM: Self = Self(3);
    pub(crate) const ANNOTATION_ELEMENT_MINIMUM: Self = Self(2);
    pub(crate) const EXCEPTION_HANDLER_MINIMUM: Self = Self(1);
    pub(crate) const TYPED_CATCH_MINIMUM: Self = Self(2);
    pub(crate) const STRING_ID: Self = Self(4);
    pub(crate) const TYPE_ID: Self = Self(4);
    pub(crate) const PROTOTYPE_ID: Self = Self(12);
    pub(crate) const FIELD_ID: Self = Self(8);
    pub(crate) const METHOD_ID: Self = Self(8);
    pub(crate) const CLASS_DEFINITION: Self = Self(32);
    pub(crate) const CALL_SITE_ID: Self = Self(4);
    pub(crate) const METHOD_HANDLE: Self = Self(8);
    pub(crate) const MAP_ITEM: Self = Self(12);
    pub(crate) const TRY_ITEM: Self = Self(8);
    pub(crate) const ANNOTATION_ASSOCIATION: Self = Self(8);
    pub(crate) const CODE_HEADER: Self = Self(16);
    pub(crate) const ANNOTATION_DIRECTORY_HEADER: Self = Self(16);

    pub(crate) const fn bytes(self) -> usize {
        self.0
    }
}

macro_rules! fields {
    ($name:ident, {$($variant:ident = $offset:literal),+ $(,)?}) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub(crate) enum $name {
            $($variant),+
        }

        impl $name {
            pub(crate) const fn offset(self) -> usize {
                match self {
                    $(Self::$variant => $offset),+
                }
            }

        }
    };
}

macro_rules! writer_fields {
    ($name:ident, {$($variant:ident = $offset:literal),+ $(,)?}) => {
        fields!($name, {$($variant = $offset),+});

        impl $name {
            pub(crate) const fn offset_u32(self) -> u32 {
                match self {
                    $(Self::$variant => $offset),+
                }
            }
        }
    };
}

fields!(MapField, {
    Type = 0,
    Unused = 2,
    Size = 4,
    Offset = 8,
});
writer_fields!(PrototypeField, {
    Shorty = 0,
    ReturnType = 4,
    ParametersOffset = 8,
});
fields!(FieldIdField, {
    Class = 0,
    Type = 2,
    Name = 4,
});
fields!(MethodIdField, {
    Class = 0,
    Prototype = 2,
    Name = 4,
});
fields!(MethodHandleField, {
    Kind = 0,
    FirstUnused = 2,
    Target = 4,
    SecondUnused = 6,
});
writer_fields!(ClassField, {
    Class = 0,
    AccessFlags = 4,
    Superclass = 8,
    InterfacesOffset = 12,
    SourceFile = 16,
    AnnotationsOffset = 20,
    ClassDataOffset = 24,
    StaticValuesOffset = 28,
});
fields!(CodeField, {
    RegistersSize = 0,
    InsSize = 2,
    OutsSize = 4,
    TriesSize = 6,
    DebugInfoOffset = 8,
    InstructionsSize = 12,
    Instructions = 16,
});
writer_fields!(TryField, {
    StartAddress = 0,
    InstructionCount = 4,
    HandlerOffset = 6,
});
writer_fields!(AnnotationDirectoryField, {
    ClassAnnotationsOffset = 0,
    FieldsSize = 4,
    MethodsSize = 8,
    ParametersSize = 12,
    Associations = 16,
});
writer_fields!(AnnotationAssociationField, {
    Identity = 0,
    AnnotationsOffset = 4,
});
fields!(ListField, {
    Size = 0,
    Entries = 4,
});
writer_fields!(HiddenApiField, {
    Size = 0,
    ClassOffsets = 4,
});

/// Required zero value of reserved fields.
pub(crate) const UNUSED_FIELD_VALUE: u16 = 0;
/// Count used for singleton header, map, and hidden-API sections.
pub(crate) const SINGLE_ITEM_COUNT: u32 = 1;
/// Count used by an empty section.
pub(crate) const EMPTY_ITEM_COUNT: u32 = 0;
/// Platform-sized count used by an empty in-memory section.
pub(crate) const EMPTY_ITEM_COUNT_USIZE: usize = 0;

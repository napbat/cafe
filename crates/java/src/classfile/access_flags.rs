//! Type-safe access flags used by JVM class, field, and method declarations.

macro_rules! define_access_flags {
    (
        $(#[$metadata:meta])*
        $name:ident {
            $($(#[$flag_metadata:meta])* $flag:ident = $value:expr),+ $(,)?
        }
    ) => {
        $(#[$metadata])*
        #[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
        pub struct $name(u16);

        impl $name {
            $(
                $(#[$flag_metadata])*
                pub const $flag: Self = Self($value);
            )+

            /// Every access flag recognized for this declaration kind.
            pub const ALL: &[Self] = &[$(Self::$flag),+];

            /// Bit mask containing every recognized access flag.
            pub const KNOWN_BITS: u16 = 0 $(| $value)+;

            /// Retains every bit, including flags introduced by newer JVM versions.
            #[must_use]
            pub const fn from_bits_retain(bits: u16) -> Self {
                Self(bits)
            }

            /// Returns the raw class-file bit representation.
            #[must_use]
            pub const fn bits(self) -> u16 {
                self.0
            }

            /// Tests whether all bits in `other` are set.
            #[must_use]
            pub const fn contains(self, other: Self) -> bool {
                self.0 & other.0 == other.0
            }

            /// Returns whether no access bits are set.
            #[must_use]
            pub const fn is_empty(self) -> bool {
                self.0 == 0
            }

            /// Returns bits retained from a newer or otherwise unknown JVM flag.
            #[must_use]
            pub const fn unknown_bits(self) -> u16 {
                self.0 & !Self::KNOWN_BITS
            }
        }

        impl std::ops::BitOr for $name {
            type Output = Self;

            fn bitor(self, rhs: Self) -> Self::Output {
                Self(self.0 | rhs.0)
            }
        }

        impl std::ops::BitOrAssign for $name {
            fn bitor_assign(&mut self, rhs: Self) {
                self.0 |= rhs.0;
            }
        }

        impl From<u16> for $name {
            fn from(bits: u16) -> Self {
                Self::from_bits_retain(bits)
            }
        }

        impl From<$name> for u16 {
            fn from(flags: $name) -> Self {
                flags.bits()
            }
        }
    };
}

define_access_flags! {
    /// Type-safe class declaration access flags.
    ClassAccessFlags {
        /// `ACC_PUBLIC`.
        PUBLIC = 0x0001,
        /// `ACC_FINAL`.
        FINAL = 0x0010,
        /// `ACC_SUPER`.
        SUPER = 0x0020,
        /// `ACC_INTERFACE`.
        INTERFACE = 0x0200,
        /// `ACC_ABSTRACT`.
        ABSTRACT = 0x0400,
        /// `ACC_SYNTHETIC`.
        SYNTHETIC = 0x1000,
        /// `ACC_ANNOTATION`.
        ANNOTATION = 0x2000,
        /// `ACC_ENUM`.
        ENUM = 0x4000,
        /// `ACC_MODULE`.
        MODULE = 0x8000,
    }
}

define_access_flags! {
    /// Type-safe field declaration access flags.
    FieldAccessFlags {
        /// `ACC_PUBLIC`.
        PUBLIC = 0x0001,
        /// `ACC_PRIVATE`.
        PRIVATE = 0x0002,
        /// `ACC_PROTECTED`.
        PROTECTED = 0x0004,
        /// `ACC_STATIC`.
        STATIC = 0x0008,
        /// `ACC_FINAL`.
        FINAL = 0x0010,
        /// `ACC_VOLATILE`.
        VOLATILE = 0x0040,
        /// `ACC_TRANSIENT`.
        TRANSIENT = 0x0080,
        /// `ACC_SYNTHETIC`.
        SYNTHETIC = 0x1000,
        /// `ACC_ENUM`.
        ENUM = 0x4000,
    }
}

define_access_flags! {
    /// Type-safe method declaration access flags.
    MethodAccessFlags {
        /// `ACC_PUBLIC`.
        PUBLIC = 0x0001,
        /// `ACC_PRIVATE`.
        PRIVATE = 0x0002,
        /// `ACC_PROTECTED`.
        PROTECTED = 0x0004,
        /// `ACC_STATIC`.
        STATIC = 0x0008,
        /// `ACC_FINAL`.
        FINAL = 0x0010,
        /// `ACC_SYNCHRONIZED`.
        SYNCHRONIZED = 0x0020,
        /// `ACC_BRIDGE`.
        BRIDGE = 0x0040,
        /// `ACC_VARARGS`.
        VARARGS = 0x0080,
        /// `ACC_NATIVE`.
        NATIVE = 0x0100,
        /// `ACC_ABSTRACT`.
        ABSTRACT = 0x0400,
        /// `ACC_STRICT`.
        STRICT = 0x0800,
        /// `ACC_SYNTHETIC`.
        SYNTHETIC = 0x1000,
    }
}

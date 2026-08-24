//! Crate-level error and result types.

use thiserror::Error;

use crate::descriptor::DescriptorError;
use crate::method::NativeMethodId;
use crate::symbol::{NativeSymbol, SymbolError};
use crate::text::JavaText;

/// Error produced while modeling a JNI declaration.
#[derive(Debug, Error)]
pub enum Error {
    /// A JVM method descriptor is malformed.
    #[error(transparent)]
    Descriptor(#[from] DescriptorError),
    /// A declaration occurs more than once in a native-method collection.
    #[error("duplicate native method `{method}`")]
    DuplicateNativeMethod {
        /// Repeated declaration identity.
        method: Box<NativeMethodId>,
    },
    /// Two declarations require the same exported native symbol.
    #[error("native methods `{first}` and `{second}` both map to `{symbol}`")]
    NativeSymbolCollision {
        /// Colliding exported symbol.
        symbol: NativeSymbol,
        /// First declaration assigned to the symbol.
        first: Box<NativeMethodId>,
        /// Second declaration assigned to the symbol.
        second: Box<NativeMethodId>,
    },
    /// A declaration cannot use JNI's dynamic symbol lookup mapping.
    #[error(transparent)]
    Symbol(#[from] SymbolError),
    /// A Java class-file operation failed.
    #[error(transparent)]
    Java(Box<::java::Error>),
    /// A DEX or APK operation failed.
    #[error(transparent)]
    Dex(Box<::dex::Error>),
    /// A DEX method owner is not encoded as an object descriptor.
    #[error("DEX native method owner `{descriptor}` is not an object descriptor")]
    InvalidDexDeclaringType {
        /// Exact invalid DEX type descriptor.
        descriptor: Box<JavaText>,
    },
}

impl From<::java::Error> for Error {
    fn from(error: ::java::Error) -> Self {
        Self::Java(Box::new(error))
    }
}

impl From<::dex::Error> for Error {
    fn from(error: ::dex::Error) -> Self {
        Self::Dex(Box::new(error))
    }
}

/// Result alias used by JNI operations.
pub type Result<T> = std::result::Result<T, Error>;

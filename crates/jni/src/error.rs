//! Crate-level error and result types.

use thiserror::Error;

use crate::descriptor::DescriptorError;
use crate::method::NativeMethodId;
use crate::symbol::{NativeSymbol, SymbolError};

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
}

/// Result alias used by JNI operations.
pub type Result<T> = std::result::Result<T, Error>;

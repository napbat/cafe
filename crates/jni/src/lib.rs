//! Safe, typed models for the Java Native Interface boundary.
//!
//! This crate describes native declarations, JVM descriptors, JNI ABI types,
//! and native symbol names. It deliberately does not load native libraries,
//! expose raw pointers, or analyze the machine code behind a native method.

mod error;

pub mod binding;
pub mod descriptor;
pub mod dex;
pub mod header;
pub mod java;
pub mod method;
pub mod report;
pub mod symbol;
pub mod text;

pub use self::binding::{
    NativeBinding, NativeImplementation, NativeMethods, RegisterNativesEntry, RegisterNativesTable,
    ResolvedRegistration, ResolvedRegistrationTable,
};
pub use self::descriptor::{
    ArrayDimensions, ArrayElement, ArrayType, JavaType, MethodDescriptor, NativeType,
    PrimitiveType, ReturnType,
};
pub use self::error::{Error, Result};
pub use self::header::{CHeaderPolicy, CLinkage, render_c_header};
pub use self::method::{
    InvocationKind, NativeMethod, NativeMethodId, NativeParameter, NativeParameterRole,
    NativePrototype, NativeRegistration, ParameterIndex, ReceiverType,
};
pub use self::report::{
    NativeBindingReport, NativeOrigin, ProvenancedNativeMethod, ReportedNativeBinding,
};
pub use self::symbol::{LookupSymbols, NativeSymbol, SymbolComponent, SymbolStyle};
pub use self::text::JavaText;

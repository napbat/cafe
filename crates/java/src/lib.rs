//! JVM class-file assembly, shared disassembly lowering, and JAR utilities.

mod error;

pub mod analysis;
pub mod bytecode;
pub mod classfile;
pub mod corpus;
pub mod descriptor;
pub mod disassemble;
pub mod disassembly;
pub mod jar;
pub mod jimage;
pub mod jmod;
pub mod program;

pub use self::program::{
    DisplayJavaReferenceResolver, JavaEmissionError, JavaEmissionOptions, JavaEmitter,
    JavaReferenceResolutionError, JavaReferenceResolver, emit_module,
};
pub use self::program::{MethodBodyMode, ProgramOptions, lower_class, lower_class_with_options};

/// Compatibility facade for JVM class-file assembly.
pub mod assemble {
    pub use crate::classfile::assemble_class;
}

/// Compatibility facade for JVM opcode and primitive-array encodings.
pub mod opcode {
    pub use crate::bytecode::opcode::*;
}

/// Compatibility facade for class-file version types and constants.
pub mod version {
    pub use crate::classfile::version::*;
}

pub use error::{Error, Result};

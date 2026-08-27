//! JVM class-file assembly, reversible JVM LLIL, bidirectional MLIL adaptation,
//! shared disassembly lifting, and JAR utilities.

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
pub mod llil;
pub mod mlil;
pub mod program;
pub mod rtl;

pub use self::program::{
    DisplayJavaReferenceResolver, JavaEmissionError, JavaEmissionOptions, JavaEmitter,
    JavaReferenceResolutionError, JavaReferenceResolver, emit_module,
};
pub use self::program::{MethodBodyMode, ProgramOptions, lift_class, lift_class_with_options};

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

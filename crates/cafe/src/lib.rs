//! Owned program model for Java bytecode tooling.
//!
//! `cafe` turns raw disassembly artifacts into an object model
//! organized as [`Program`], [`Module`], [`TypeDefinition`],
//! [`FieldDefinition`], and [`MethodDefinition`]. Format crates can enrich the
//! same model with metadata that is not part of instruction disassembly.
//! Tools can then traverse, resolve, and edit definitions without depending on
//! JVM class-file or Android DEX implementation details.

mod definition;
mod error;
mod identity;
mod module;
mod program;
mod resolution;
mod source;

/// Raw disassembly types retained by method definitions.
pub mod disassembly {
    pub use disassembler::{
        AddressRange, AddressUnit, CatchType, CodeAddress, CodeSize, ControlFlowGraph,
        ExceptionHandler, FunctionBody, GraphError, Immediate, Instruction, InstructionFlow,
        Operand, Reference, ReferenceKind, SwitchCase, SwitchTable,
    };
}

pub use disassembler::{BinaryFormat, GraphError, RawAccessFlags};

pub use self::definition::{FieldDefinition, MethodDefinition, TypeDefinition};
pub use self::error::{DefinitionKind, Error, Result, SymbolComponent};
pub use self::identity::{FieldId, MethodId, ModuleId, TypeId};
pub use self::module::Module;
pub use self::program::Program;
pub use self::resolution::Resolution;
pub use self::source::ModuleSource;

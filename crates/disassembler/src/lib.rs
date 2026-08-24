//! Shared Java-ecosystem bytecode disassembly models and control-flow graphs.
//!
//! Java format crates lower their native decoded representation into this crate's
//! [`Disassembly`] model by implementing [`DisassemblySource`]. The shared
//! instruction model deliberately retains native opcodes, mnemonics,
//! references, addresses, and signatures while presenting one stable boundary
//! to graphing and downstream analysis tools. Shared CFG edges retain exact
//! switch, handler, throw-site, and continuation provenance in caller-owned
//! cfglib payloads and expose zero-copy normal-only views.

mod diagnostic;
pub mod graph;
mod model;
mod source;
mod source_map;

/// The graph library used by this crate's public control-flow representation.
pub use cfglib;
pub use diagnostic::{
    Diagnostic, DiagnosticLevel, DiagnosticLocation, DiagnosticNote, Diagnostics,
};
pub use graph::{
    ControlFlowEdge, ControlFlowEdgeRole, ControlFlowGraph, ExceptionHandlerIndex, GraphError,
    NormalControlFlow, build_control_flow_graph,
};
pub use model::{
    AddressRange, AddressUnit, BinaryFormat, CatchType, CodeAddress, CodeSize, Disassembly,
    ExceptionHandler, Function, FunctionBody, FunctionSymbol, Immediate, Instruction,
    InstructionFlow, Operand, RawAccessFlags, Reference, ReferenceKind, SwitchCase, SwitchTable,
};
pub use source::DisassemblySource;
pub use source_map::{FunctionCoordinate, SourceMap, SourceMapEntry, SourceMapError};

//! Shared disassembly intermediate representation.

mod address;
mod artifact;
mod flow;
mod instruction;
mod operand;

pub use self::address::{AddressRange, AddressUnit, CodeAddress, CodeSize};
pub use self::artifact::{
    BinaryFormat, CatchType, Disassembly, ExceptionHandler, Function, FunctionBody, FunctionSymbol,
    RawAccessFlags, RegisterResources,
};
pub use self::flow::InstructionFlow;
pub use self::instruction::{ExceptionBehavior, Instruction};
pub use self::operand::{
    ExactText, Immediate, Operand, Reference, ReferenceKind, ReferenceSymbol, SwitchCase,
    SwitchTable,
};

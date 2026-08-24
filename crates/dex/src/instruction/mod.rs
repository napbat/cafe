//! Typed Dalvik instructions and payload pseudo-instructions.
//!
//! DEX addresses and instruction widths are measured in 16-bit code units.
//! Decoding resolves relative branches to absolute code-unit offsets. Encoding
//! converts those offsets back to the narrowest representation selected by the
//! opcode and rejects values which do not fit.

mod decode;
mod encode;
mod model;
mod opcode;

pub use self::decode::decode;
pub use self::encode::encode;
pub use self::model::{
    ArrayDataPayload, Instruction, InstructionData, Operands, PackedSwitchPayload,
    SparseSwitchPayload,
};
pub use self::opcode::{IndexKind, InstructionFormat, Opcode};

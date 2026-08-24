//! Bounded endian-aware binary primitives.

mod leb128;
mod reader;
mod writer;

pub(super) use self::reader::{Cursor, Reader};
pub(super) use self::writer::Writer;

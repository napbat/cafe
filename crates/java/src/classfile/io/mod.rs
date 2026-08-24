//! Binary input and output primitives for the JVM class-file format.

mod reader;
mod writer;

pub(super) use reader::Reader;
pub(super) use writer::Writer;

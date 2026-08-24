//! ART `CompactDex` headers, split sections, offset tables, and code items.
//!
//! `CompactDex` uses the same identifier and instruction concepts as DEX but
//! stores data items in a separately addressable section and compresses code
//! item headers. This module retains that physical distinction explicitly.

mod code;
mod file;
mod header;
mod offsets;

pub use self::code::{
    CompactCodeItem, EncodedCompactCodeItem, decode_code_item, encode_code_item,
    encode_code_item_at,
};
pub use self::file::{CompactDexFile, CompactDexSections, CompactMethodLocation};
pub use self::header::{
    COMPACT_DEX_HEADER_SIZE, COMPACT_DEX_MAGIC, CompactDexFeatureFlags, CompactDexHeader,
    CompactDexVersion,
};
pub use self::offsets::CompactOffsetTable;

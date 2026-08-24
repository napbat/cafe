//! DEX adapter for the shared Cafe object model.

mod adapter;

pub use self::adapter::{
    CafeOptions, MethodBodyMode, lower_file, lower_file_named, lower_file_named_with_options,
    lower_file_with_options,
};

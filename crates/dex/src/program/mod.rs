//! DEX adapter for the shared owned program model.

mod adapter;

pub use self::adapter::{
    MethodBodyMode, ProgramOptions, lower_file, lower_file_named, lower_file_named_with_options,
    lower_file_with_options,
};

//! Safe, typed models for the Java Native Interface boundary.
//!
//! This crate describes native declarations, JVM descriptors, JNI ABI types,
//! and native symbol names. It deliberately does not load native libraries,
//! expose raw pointers, or analyze the machine code behind a native method.

mod error;

pub mod descriptor;
pub mod text;

pub use self::error::{Error, Result};

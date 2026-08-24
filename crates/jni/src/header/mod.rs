//! Portable C header rendering from typed JNI ABI declarations.

use std::fmt::Write as _;

use crate::binding::NativeMethods;
use crate::method::NativeParameterRole;
use crate::{Error, Result};

const JNI_HEADER_NAME: &str = "jni.h";
const JNI_EXPORT_MACRO: &str = "JNIEXPORT";
const JNI_CALL_MACRO: &str = "JNICALL";
const CPLUSPLUS_MACRO: &str = "__cplusplus";

/// C versus guarded C++ linkage in a generated header.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum CLinkage {
    /// Emit plain C declarations.
    C,
    /// Wrap declarations in an `extern "C"` guard for C++ consumers.
    #[default]
    CAndCpp,
}

/// Explicit naming and formatting policy for portable JNI declarations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CHeaderPolicy {
    guard: String,
    linkage: CLinkage,
    include_jni: bool,
    use_jni_macros: bool,
}

impl CHeaderPolicy {
    /// Creates a default policy with a validated C preprocessor guard.
    ///
    /// # Errors
    ///
    /// Returns an error unless the guard is a portable ASCII C identifier.
    pub fn new(guard: impl Into<String>) -> Result<Self> {
        let guard = guard.into();
        validate_identifier(&guard)?;
        Ok(Self {
            guard,
            linkage: CLinkage::default(),
            include_jni: true,
            use_jni_macros: true,
        })
    }

    /// Selects C-only or guarded C++ linkage.
    #[must_use]
    pub const fn with_linkage(mut self, linkage: CLinkage) -> Self {
        self.linkage = linkage;
        self
    }

    /// Selects whether the renderer emits `#include <jni.h>`.
    #[must_use]
    pub const fn with_jni_include(mut self, include: bool) -> Self {
        self.include_jni = include;
        self
    }

    /// Selects whether declarations use `JNIEXPORT` and `JNICALL`.
    #[must_use]
    pub const fn with_jni_macros(mut self, enabled: bool) -> Self {
        self.use_jni_macros = enabled;
        self
    }

    /// Returns the exact include guard.
    #[must_use]
    pub fn guard(&self) -> &str {
        &self.guard
    }
}

/// Renders one deterministic portable header from overload-aware bindings.
///
/// # Errors
///
/// Returns an error if JNI symbol escaping or overload collision detection
/// fails.
pub fn render_c_header(methods: &NativeMethods, policy: &CHeaderPolicy) -> Result<String> {
    let bindings = methods.bindings()?;
    let mut output = String::new();
    writeln!(output, "#ifndef {}", policy.guard).expect("String writes cannot fail");
    writeln!(output, "#define {}", policy.guard).expect("String writes cannot fail");
    if policy.include_jni {
        writeln!(output, "\n#include <{JNI_HEADER_NAME}>").expect("String writes cannot fail");
    }
    if policy.linkage == CLinkage::CAndCpp {
        write!(
            output,
            "\n#ifdef {CPLUSPLUS_MACRO}\nextern \"C\" {{\n#endif\n"
        )
        .expect("String writes cannot fail");
    }
    for binding in bindings {
        let prototype = binding.method().prototype();
        output.push('\n');
        if policy.use_jni_macros {
            write!(output, "{JNI_EXPORT_MACRO} ").expect("String writes cannot fail");
        }
        write!(output, "{} ", prototype.return_type().c_name()).expect("String writes cannot fail");
        if policy.use_jni_macros {
            write!(output, "{JNI_CALL_MACRO} ").expect("String writes cannot fail");
        }
        writeln!(output, "{}(", binding.symbol()).expect("String writes cannot fail");
        for (position, parameter) in prototype.parameters().iter().enumerate() {
            let name = match parameter.role() {
                NativeParameterRole::Environment => "env".to_owned(),
                NativeParameterRole::Receiver(_) => "receiver".to_owned(),
                NativeParameterRole::Argument(index) => format!("arg{}", index.get()),
            };
            let suffix = if position + 1 == prototype.parameters().len() {
                ""
            } else {
                ","
            };
            writeln!(
                output,
                "    {} {name}{suffix}",
                parameter.native_type().c_name()
            )
            .expect("String writes cannot fail");
        }
        writeln!(output, ");").expect("String writes cannot fail");
    }
    if policy.linkage == CLinkage::CAndCpp {
        write!(output, "\n#ifdef {CPLUSPLUS_MACRO}\n}}\n#endif\n")
            .expect("String writes cannot fail");
    }
    writeln!(output, "\n#endif /* {} */", policy.guard).expect("String writes cannot fail");
    Ok(output)
}

fn validate_identifier(value: &str) -> Result<()> {
    let mut bytes = value.bytes();
    let Some(first) = bytes.next() else {
        return Err(Error::InvalidCIdentifier(value.to_owned()));
    };
    if !(first.is_ascii_alphabetic() || first == b'_')
        || !bytes.all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
    {
        return Err(Error::InvalidCIdentifier(value.to_owned()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::method::{InvocationKind, NativeMethod};

    use super::*;

    #[test]
    fn renders_overload_qualified_portable_declarations() -> Result<()> {
        let methods = NativeMethods::from_methods([
            NativeMethod::new("sample/Native", "read", "()I", InvocationKind::Instance)?,
            NativeMethod::new("sample/Native", "read", "(J)J", InvocationKind::Static)?,
        ])?;
        let header = render_c_header(&methods, &CHeaderPolicy::new("SAMPLE_NATIVE_H")?)?;

        assert!(header.contains("#include <jni.h>"));
        assert!(header.contains("Java_sample_Native_read__("));
        assert!(header.contains("Java_sample_Native_read__J("));
        assert!(header.contains("JNIEnv * env"));
        assert!(header.contains("jclass receiver"));
        assert!(header.contains("jlong arg0"));
        Ok(())
    }

    #[test]
    fn rejects_header_injection() {
        assert!(CHeaderPolicy::new("BAD\n#define PWNED").is_err());
    }
}

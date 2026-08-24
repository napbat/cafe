//! Java module native-access requirements introduced by JEP 472.

use ::java::classfile::{Attribute, ClassFile, Constant, KnownAttribute};

use crate::binding::NativeMethods;
use crate::text::JavaText;
use crate::{Error, Result};

/// First Java feature release that warns for unauthorized JNI binding.
pub const JNI_NATIVE_ACCESS_RELEASE: u16 = 24;
/// Launcher token granting native access to every class-path module.
pub const ALL_UNNAMED_NATIVE_ACCESS: &str = "ALL-UNNAMED";

/// Java module identity relevant to `--enable-native-access`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ModuleIdentity {
    /// Explicitly named module from `module-info.class`.
    Named(JavaText),
    /// Class-path code in an unnamed module.
    Unnamed,
}

impl ModuleIdentity {
    /// Returns the launcher value used to grant this module native access.
    #[must_use]
    pub fn launcher_value(&self) -> &str {
        match self {
            Self::Named(name) => name.as_str(),
            Self::Unnamed => ALL_UNNAMED_NATIVE_ACCESS,
        }
    }
}

/// Runtime treatment known for a target Java release.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NativeAccessMode {
    /// JNI binding is not governed by the module native-access gate.
    Unrestricted,
    /// Unauthorized binding proceeds with a warning by default.
    WarnUnlessEnabled,
}

/// Native-access requirement inferred from declarations and module metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeAccessRequirement {
    /// Module whose native methods will be bound.
    pub module: ModuleIdentity,
    /// Java feature release used for the policy decision.
    pub target_release: u16,
    /// Number of native declarations contributing to the report.
    pub native_methods: usize,
    /// Known default runtime treatment for the target release.
    pub mode: NativeAccessMode,
    /// Exact launcher operand, if access should be enabled.
    pub enable_native_access: Option<String>,
}

impl NativeAccessRequirement {
    /// Returns whether native access should be granted to avoid diagnostics or
    /// future denial when at least one native method is bound.
    #[must_use]
    pub const fn requires_enablement(&self) -> bool {
        self.enable_native_access.is_some()
    }

    /// Renders the complete launcher option when enablement is required.
    #[must_use]
    pub fn launcher_option(&self) -> Option<String> {
        self.enable_native_access
            .as_ref()
            .map(|value| format!("--enable-native-access={value}"))
    }
}

/// Resolves a named module from `module-info.class`, or returns the unnamed
/// module identity when no descriptor is supplied.
///
/// # Errors
///
/// Returns an error when the class does not contain exactly one valid `Module`
/// attribute or its constant-pool module name is invalid.
pub fn module_identity(module_info: Option<&ClassFile>) -> Result<ModuleIdentity> {
    let Some(class) = module_info else {
        return Ok(ModuleIdentity::Unnamed);
    };
    let mut modules = class
        .attributes
        .iter()
        .filter_map(|attribute| match attribute {
            Attribute::Known(KnownAttribute::Module(module)) => Some(module),
            _ => None,
        });
    let module = modules.next().ok_or_else(|| {
        Error::InvalidModuleMetadata("module-info.class has no Module attribute".to_owned())
    })?;
    if modules.next().is_some() {
        return Err(Error::InvalidModuleMetadata(
            "module-info.class contains more than one Module attribute".to_owned(),
        ));
    }
    let Constant::Module { name_index } = class.constant_pool.get(module.module_name_index)? else {
        return Err(Error::InvalidModuleMetadata(format!(
            "constant {} is not a module name",
            module.module_name_index
        )));
    };
    let name = class.constant_pool.utf8_constant(*name_index)?;
    Ok(ModuleIdentity::Named(JavaText::from_utf16(
        name.utf16_units().to_vec(),
    )))
}

/// Reports the target-release native-access requirement for declarations.
///
/// Java 24 through the currently modeled releases warn by default when JNI
/// native-method binding occurs without access. This API does not speculate
/// about a future release that may change the default to denial.
#[must_use]
pub fn native_access_requirement(
    module: ModuleIdentity,
    methods: &NativeMethods,
    target_release: u16,
) -> NativeAccessRequirement {
    let restricted = target_release >= JNI_NATIVE_ACCESS_RELEASE;
    let enable_native_access =
        (restricted && !methods.is_empty()).then(|| module.launcher_value().to_owned());
    NativeAccessRequirement {
        module,
        target_release,
        native_methods: methods.len(),
        mode: if restricted {
            NativeAccessMode::WarnUnlessEnabled
        } else {
            NativeAccessMode::Unrestricted
        },
        enable_native_access,
    }
}

#[cfg(test)]
mod tests {
    use crate::method::{InvocationKind, NativeMethod};

    use super::*;

    #[test]
    fn reports_named_and_unnamed_launcher_operands() -> Result<()> {
        let methods = NativeMethods::from_methods([NativeMethod::new(
            "sample/Native",
            "open",
            "()V",
            InvocationKind::Static,
        )?])?;
        let named = native_access_requirement(
            ModuleIdentity::Named(JavaText::new("sample.module")),
            &methods,
            24,
        );
        let unnamed = native_access_requirement(ModuleIdentity::Unnamed, &methods, 26);
        let legacy = native_access_requirement(ModuleIdentity::Unnamed, &methods, 23);

        assert_eq!(
            named.launcher_option().as_deref(),
            Some("--enable-native-access=sample.module")
        );
        assert_eq!(
            unnamed.launcher_option().as_deref(),
            Some("--enable-native-access=ALL-UNNAMED")
        );
        assert!(!legacy.requires_enablement());
        assert_eq!(legacy.mode, NativeAccessMode::Unrestricted);
        Ok(())
    }
}

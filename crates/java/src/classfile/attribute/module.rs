//! Java Platform Module System attribute models.

use super::super::{ModuleAccessFlags, ModuleExportsFlags, ModuleOpensFlags, ModuleRequiresFlags};

/// Typed `Module` attribute from a `module-info.class` file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModuleAttribute {
    /// Constant-pool index of `Module`.
    pub name_index: u16,
    /// Module constant-pool index naming the declared module.
    pub module_name_index: u16,
    /// Module declaration flags.
    pub module_flags: ModuleAccessFlags,
    /// UTF-8 module version index, or zero when absent.
    pub module_version_index: u16,
    /// Required modules.
    pub requires: Vec<ModuleRequire>,
    /// Exported packages.
    pub exports: Vec<ModuleExport>,
    /// Open packages.
    pub opens: Vec<ModuleOpen>,
    /// Class indices of consumed service interfaces.
    pub uses: Vec<u16>,
    /// Service implementations provided by this module.
    pub provides: Vec<ModuleProvide>,
}

/// One module `requires` directive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModuleRequire {
    /// Module constant-pool index of the dependency.
    pub module_index: u16,
    /// Dependency flags.
    pub flags: ModuleRequiresFlags,
    /// UTF-8 dependency version index, or zero when absent.
    pub version_index: u16,
}

/// One module `exports` directive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModuleExport {
    /// Package constant-pool index of the exported package.
    pub package_index: u16,
    /// Export flags.
    pub flags: ModuleExportsFlags,
    /// Module indices receiving a qualified export; empty means unqualified.
    pub to_modules: Vec<u16>,
}

/// One module `opens` directive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModuleOpen {
    /// Package constant-pool index of the opened package.
    pub package_index: u16,
    /// Open flags.
    pub flags: ModuleOpensFlags,
    /// Module indices receiving a qualified open; empty means unqualified.
    pub to_modules: Vec<u16>,
}

/// One module `provides ... with ...` directive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModuleProvide {
    /// Class index of the service interface.
    pub service_index: u16,
    /// Non-empty class indices of service implementations.
    pub implementation_indices: Vec<u16>,
}

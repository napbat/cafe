//! Aggregate native binding plans with exact frontend provenance.

use ::dex::apk::DexOrdinal;

use crate::binding::NativeMethods;
use crate::method::NativeMethod;
use crate::symbol::{NativeSymbol, SymbolStyle};
use crate::{Error, Result};

/// Exact artifact and container origin of one native declaration.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum NativeOrigin {
    /// Standalone JVM class file.
    ClassFile {
        /// Caller-supplied artifact identity.
        artifact: String,
    },
    /// Effective class selected from a possibly multi-release JAR.
    JarClass {
        /// Caller-supplied JAR identity.
        artifact: String,
        /// Logical class entry visible at the target release.
        logical_entry: String,
        /// Exact physical ZIP member selected.
        physical_entry: String,
        /// Requested Java feature release.
        target_release: u16,
        /// Selected override release, or `None` for the base tree.
        selected_release: Option<u16>,
    },
    /// Standalone DEX file and native class-definition position.
    DexFile {
        /// Caller-supplied artifact identity.
        artifact: String,
        /// Native class-definition position.
        class_definition: u32,
    },
    /// DEX class within an APK multidex member.
    ApkDex {
        /// Caller-supplied APK identity.
        artifact: String,
        /// Exact APK entry name.
        entry: String,
        /// Numeric multidex ordinal.
        ordinal: DexOrdinal,
        /// Native class-definition position.
        class_definition: u32,
    },
    /// DEX class within an App Bundle module.
    AabDex {
        /// Caller-supplied App Bundle identity.
        artifact: String,
        /// Bundle module name.
        module: String,
        /// Exact bundle entry name.
        entry: String,
        /// Numeric multidex ordinal inside the module.
        ordinal: DexOrdinal,
        /// Native class-definition position.
        class_definition: u32,
    },
}

/// One native declaration paired with its exact source origin.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ProvenancedNativeMethod {
    origin: NativeOrigin,
    method: NativeMethod,
}

impl ProvenancedNativeMethod {
    /// Creates one provenance/declaration pair.
    #[must_use]
    pub const fn new(origin: NativeOrigin, method: NativeMethod) -> Self {
        Self { origin, method }
    }

    /// Returns exact artifact provenance.
    #[must_use]
    pub const fn origin(&self) -> &NativeOrigin {
        &self.origin
    }

    /// Returns the typed native declaration.
    #[must_use]
    pub const fn method(&self) -> &NativeMethod {
        &self.method
    }
}

/// Owned overload-aware binding paired with declaration provenance.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ReportedNativeBinding {
    origin: NativeOrigin,
    method: NativeMethod,
    symbol: NativeSymbol,
    style: SymbolStyle,
}

impl ReportedNativeBinding {
    /// Returns exact artifact provenance.
    #[must_use]
    pub const fn origin(&self) -> &NativeOrigin {
        &self.origin
    }

    /// Returns the typed native declaration.
    #[must_use]
    pub const fn method(&self) -> &NativeMethod {
        &self.method
    }

    /// Returns the selected exported symbol.
    #[must_use]
    pub const fn symbol(&self) -> &NativeSymbol {
        &self.symbol
    }

    /// Returns the short or overload-qualified symbol choice.
    #[must_use]
    pub const fn style(&self) -> SymbolStyle {
        self.style
    }
}

/// Complete aggregate declaration and binding report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeBindingReport {
    declarations: Vec<ProvenancedNativeMethod>,
    bindings: Vec<ReportedNativeBinding>,
}

impl NativeBindingReport {
    /// Builds an overload plan while retaining every declaration's origin.
    ///
    /// # Errors
    ///
    /// Returns an error for duplicate JVM identities, an unescapable JNI name,
    /// or an export collision.
    pub fn new(declarations: Vec<ProvenancedNativeMethod>) -> Result<Self> {
        let methods = NativeMethods::from_methods(
            declarations
                .iter()
                .map(|declaration| declaration.method.clone()),
        )?;
        let bindings = methods
            .bindings()?
            .into_iter()
            .map(|binding| {
                let declaration = declarations
                    .iter()
                    .find(|declaration| declaration.method.id() == binding.method().id())
                    .ok_or_else(|| Error::RegistrationNotFound {
                        owner: Box::new(binding.method().owner().clone()),
                        name: Box::new(binding.method().name().clone()),
                        descriptor: Box::new(binding.method().descriptor().text().clone()),
                    })?;
                Ok(ReportedNativeBinding {
                    origin: declaration.origin.clone(),
                    method: declaration.method.clone(),
                    symbol: binding.symbol().clone(),
                    style: binding.style(),
                })
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(Self {
            declarations,
            bindings,
        })
    }

    /// Returns declarations in source traversal order.
    #[must_use]
    pub fn declarations(&self) -> &[ProvenancedNativeMethod] {
        &self.declarations
    }

    /// Returns overload-aware bindings in the same order.
    #[must_use]
    pub fn bindings(&self) -> &[ReportedNativeBinding] {
        &self.bindings
    }

    /// Converts the report back to a declaration collection.
    ///
    /// # Errors
    ///
    /// Returns an error only if the report was constructed inconsistently.
    pub fn native_methods(&self) -> Result<NativeMethods> {
        NativeMethods::from_methods(
            self.declarations
                .iter()
                .map(|declaration| declaration.method.clone()),
        )
    }
}

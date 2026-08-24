//! Safe resolution of caller-supplied `RegisterNatives` metadata.

use std::collections::BTreeSet;

use crate::binding::NativeMethods;
use crate::descriptor::MethodDescriptor;
use crate::method::NativeMethod;
use crate::text::JavaText;
use crate::{Error, Result};

/// Opaque, safe identifier for a consumer-known native implementation.
///
/// This is deliberately not an address or raw pointer. A consumer can use a
/// linker symbol, generated-function key, or another stable metadata identity.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct NativeImplementation(String);

impl NativeImplementation {
    /// Creates a nonempty implementation key without NUL bytes.
    ///
    /// # Errors
    ///
    /// Returns an error when the key cannot be safely transported as metadata.
    pub fn new(value: impl Into<String>) -> Result<Self> {
        let value = value.into();
        if value.is_empty() || value.contains('\0') {
            return Err(Error::InvalidImplementationKey(value));
        }
        Ok(Self(value))
    }

    /// Returns the caller-defined implementation key.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// One unresolved name, signature, and implementation triple.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RegisterNativesEntry {
    name: JavaText,
    descriptor: MethodDescriptor,
    implementation: NativeImplementation,
}

impl RegisterNativesEntry {
    /// Creates an entry from valid Unicode metadata.
    ///
    /// # Errors
    ///
    /// Returns an error for a malformed descriptor or implementation key.
    pub fn new(
        name: impl Into<JavaText>,
        descriptor: &str,
        implementation: impl Into<String>,
    ) -> Result<Self> {
        Ok(Self {
            name: name.into(),
            descriptor: MethodDescriptor::parse(descriptor)?,
            implementation: NativeImplementation::new(implementation)?,
        })
    }

    /// Creates an entry from exact Java UTF-16 metadata.
    ///
    /// # Errors
    ///
    /// Returns an error for a malformed descriptor or implementation key.
    pub fn from_utf16(
        name: Vec<u16>,
        descriptor: Vec<u16>,
        implementation: impl Into<String>,
    ) -> Result<Self> {
        Ok(Self {
            name: JavaText::from_utf16(name),
            descriptor: MethodDescriptor::from_utf16(descriptor)?,
            implementation: NativeImplementation::new(implementation)?,
        })
    }

    /// Returns the exact Java method name.
    #[must_use]
    pub const fn name(&self) -> &JavaText {
        &self.name
    }

    /// Returns the parsed exact method descriptor.
    #[must_use]
    pub const fn descriptor(&self) -> &MethodDescriptor {
        &self.descriptor
    }

    /// Returns the consumer-known implementation key.
    #[must_use]
    pub const fn implementation(&self) -> &NativeImplementation {
        &self.implementation
    }
}

/// Caller-supplied explicit-registration table for one declaring class.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegisterNativesTable {
    owner: JavaText,
    entries: Vec<RegisterNativesEntry>,
}

impl RegisterNativesTable {
    /// Creates an ordered explicit-registration table.
    #[must_use]
    pub fn new(owner: impl Into<JavaText>, entries: Vec<RegisterNativesEntry>) -> Self {
        Self {
            owner: owner.into(),
            entries,
        }
    }

    /// Returns the exact declaring class name.
    #[must_use]
    pub const fn owner(&self) -> &JavaText {
        &self.owner
    }

    /// Returns entries in caller-supplied order.
    #[must_use]
    pub fn entries(&self) -> &[RegisterNativesEntry] {
        &self.entries
    }

    /// Resolves every key against a reliable native declaration set.
    ///
    /// # Errors
    ///
    /// Returns an error for a duplicate key or a key that does not identify an
    /// exact native method in this table's owner.
    pub fn resolve(&self, methods: &NativeMethods) -> Result<ResolvedRegistrationTable> {
        let mut keys = BTreeSet::new();
        let mut resolved = Vec::with_capacity(self.entries.len());
        for entry in &self.entries {
            let key = (entry.name.clone(), entry.descriptor.text().clone());
            if !keys.insert(key) {
                return Err(Error::DuplicateRegistration {
                    owner: Box::new(self.owner.clone()),
                    name: Box::new(entry.name.clone()),
                    descriptor: Box::new(entry.descriptor.text().clone()),
                });
            }
            let method = methods
                .iter()
                .find(|method| {
                    method.owner() == &self.owner
                        && method.name() == &entry.name
                        && method.descriptor().text() == entry.descriptor.text()
                })
                .cloned()
                .ok_or_else(|| Error::RegistrationNotFound {
                    owner: Box::new(self.owner.clone()),
                    name: Box::new(entry.name.clone()),
                    descriptor: Box::new(entry.descriptor.text().clone()),
                })?;
            resolved.push(ResolvedRegistration {
                method,
                implementation: entry.implementation.clone(),
            });
        }
        Ok(ResolvedRegistrationTable {
            owner: self.owner.clone(),
            entries: resolved,
        })
    }
}

/// One exact native declaration paired with a supplied implementation key.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ResolvedRegistration {
    method: NativeMethod,
    implementation: NativeImplementation,
}

impl ResolvedRegistration {
    /// Returns the resolved native declaration.
    #[must_use]
    pub const fn method(&self) -> &NativeMethod {
        &self.method
    }

    /// Returns the consumer-known implementation key.
    #[must_use]
    pub const fn implementation(&self) -> &NativeImplementation {
        &self.implementation
    }
}

/// Fully resolved explicit-registration table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedRegistrationTable {
    owner: JavaText,
    entries: Vec<ResolvedRegistration>,
}

impl ResolvedRegistrationTable {
    /// Returns the exact declaring class.
    #[must_use]
    pub const fn owner(&self) -> &JavaText {
        &self.owner
    }

    /// Returns resolved entries in registration order.
    #[must_use]
    pub fn entries(&self) -> &[ResolvedRegistration] {
        &self.entries
    }
}

#[cfg(test)]
mod tests {
    use crate::method::{InvocationKind, NativeMethod};

    use super::*;

    #[test]
    fn resolves_overloads_by_exact_registration_key() -> Result<()> {
        let methods = NativeMethods::from_methods([
            NativeMethod::new("sample/Native", "read", "()I", InvocationKind::Instance)?,
            NativeMethod::new("sample/Native", "read", "(I)I", InvocationKind::Instance)?,
        ])?;
        let table = RegisterNativesTable::new(
            "sample/Native",
            vec![RegisterNativesEntry::new(
                "read",
                "(I)I",
                "native_read_int",
            )?],
        );
        let resolved = table.resolve(&methods)?;

        assert_eq!(resolved.entries().len(), 1);
        assert_eq!(
            resolved.entries()[0].implementation().as_str(),
            "native_read_int"
        );
        assert_eq!(
            resolved.entries()[0].method().descriptor().text().as_str(),
            "(I)I"
        );
        Ok(())
    }

    #[test]
    fn rejects_missing_and_duplicate_keys() -> Result<()> {
        let methods = NativeMethods::from_methods([NativeMethod::new(
            "sample/Native",
            "read",
            "()I",
            InvocationKind::Instance,
        )?])?;
        let entry = RegisterNativesEntry::new("read", "(I)I", "implementation")?;
        let missing = RegisterNativesTable::new("sample/Native", vec![entry.clone()]);
        assert!(matches!(
            missing.resolve(&methods),
            Err(Error::RegistrationNotFound { .. })
        ));
        let registered = RegisterNativesEntry::new("read", "()I", "implementation")?;
        let duplicate =
            RegisterNativesTable::new("sample/Native", vec![registered.clone(), registered]);
        assert!(matches!(
            duplicate.resolve(&methods),
            Err(Error::DuplicateRegistration { .. })
        ));
        Ok(())
    }
}

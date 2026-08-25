//! Canonical class names and declarations.

use std::collections::BTreeSet;
use std::fmt;

use disassembler::BinaryFormat;

use crate::{Error, Result};

/// Canonical object type name stored as a DEX-style `Lname;` descriptor.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ClassDescriptor(String);

impl ClassDescriptor {
    /// Normalizes one JVM internal class name.
    ///
    /// # Errors
    ///
    /// Returns an error for an empty name, an array descriptor, or a name that
    /// is already wrapped as a DEX object descriptor.
    pub fn from_jvm_internal(name: impl Into<String>) -> Result<Self> {
        let name = name.into();
        if !valid_internal_name(&name) {
            return Err(Error::InvalidClassName {
                format: BinaryFormat::JavaClass,
                name,
            });
        }
        Ok(Self(format!("L{name};")))
    }

    /// Validates one DEX object descriptor.
    ///
    /// # Errors
    ///
    /// Returns an error unless the value is a nonempty `Lname;` descriptor.
    pub fn from_dex_descriptor(descriptor: impl Into<String>) -> Result<Self> {
        let descriptor = descriptor.into();
        let valid = descriptor
            .strip_prefix('L')
            .and_then(|value| value.strip_suffix(';'))
            .is_some_and(valid_internal_name);
        if !valid {
            return Err(Error::InvalidClassName {
                format: BinaryFormat::Dex,
                name: descriptor,
            });
        }
        Ok(Self(descriptor))
    }

    /// Returns the canonical DEX-style object descriptor.
    #[must_use]
    pub fn as_descriptor(&self) -> &str {
        &self.0
    }

    /// Returns the equivalent JVM internal class name.
    ///
    /// # Panics
    ///
    /// Cannot panic for a value created by this type's validated constructors.
    #[must_use]
    pub fn as_jvm_internal(&self) -> &str {
        self.0
            .strip_prefix('L')
            .and_then(|value| value.strip_suffix(';'))
            .expect("ClassDescriptor construction preserves its wrapper")
    }
}

fn valid_internal_name(name: &str) -> bool {
    !name.is_empty()
        && !name.contains(['.', ';', '['])
        && name.split('/').all(|component| !component.is_empty())
}

impl fmt::Display for ClassDescriptor {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_descriptor())
    }
}

/// Canonical direct superclass and interface declaration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirectParents {
    /// Direct superclass, absent only for `java/lang/Object`.
    pub superclass: Option<ClassDescriptor>,
    /// Direct interfaces in deterministic canonical order.
    pub interfaces: Vec<ClassDescriptor>,
}

impl DirectParents {
    pub(crate) fn new(
        superclass: Option<ClassDescriptor>,
        interfaces: impl IntoIterator<Item = ClassDescriptor>,
    ) -> Self {
        let mut interfaces = interfaces.into_iter().collect::<Vec<_>>();
        interfaces.sort();
        interfaces.dedup();
        Self {
            superclass,
            interfaces,
        }
    }

    pub(crate) fn iter(&self) -> impl Iterator<Item = &ClassDescriptor> {
        self.superclass.iter().chain(&self.interfaces)
    }
}

/// One merged class declaration and the native formats that supplied it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClassDeclaration {
    pub(crate) descriptor: ClassDescriptor,
    pub(crate) parents: DirectParents,
    pub(crate) formats: BTreeSet<BinaryFormat>,
}

impl ClassDeclaration {
    /// Returns the canonical type descriptor.
    #[must_use]
    pub const fn descriptor(&self) -> &ClassDescriptor {
        &self.descriptor
    }

    /// Returns the normalized direct superclass and interfaces.
    #[must_use]
    pub const fn parents(&self) -> &DirectParents {
        &self.parents
    }

    /// Returns native formats that supplied an equivalent declaration.
    #[must_use]
    pub fn formats(&self) -> impl ExactSizeIterator<Item = BinaryFormat> + '_ {
        self.formats.iter().copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_non_class_names_in_both_native_spellings() {
        for name in ["", "[Ljava/lang/Object;", "sample//Type", "sample.Type"] {
            assert!(ClassDescriptor::from_jvm_internal(name).is_err());
        }
        for descriptor in ["", "I", "L;", "Lsample//Type;", "Lsample.Type;"] {
            assert!(ClassDescriptor::from_dex_descriptor(descriptor).is_err());
        }
    }
}

//! Classpath method declarations needed by Java source rendering.

use std::collections::BTreeMap;

use java::classfile::{ClassFile, KnownAttribute, KnownAttributeKind};

use crate::Result;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct MethodKey {
    owner: String,
    name: String,
    descriptor: String,
}

/// Declared exception metadata for methods available on a decompilation
/// classpath.
///
/// The catalog distinguishes a missing method from a known method with no
/// `Exceptions` entries. That distinction lets the renderer omit synthetic
/// checked-exception laundering only when the declaration proves it safe.
#[derive(Debug, Clone, Default)]
pub struct MethodExceptionCatalog {
    declarations: BTreeMap<MethodKey, Vec<String>>,
}

impl MethodExceptionCatalog {
    /// Builds a catalog from parsed class declarations.
    ///
    /// # Errors
    ///
    /// Returns an error when a class, method, descriptor, or declared exception
    /// has an invalid constant-pool reference.
    pub fn from_classes<'a>(classes: impl IntoIterator<Item = &'a ClassFile>) -> Result<Self> {
        let mut declarations = BTreeMap::new();
        for class in classes {
            let owner = class.class_name()?.to_owned();
            for method in &class.methods {
                let key = MethodKey {
                    owner: owner.clone(),
                    name: method.name(&class.constant_pool)?.to_owned(),
                    descriptor: method.descriptor(&class.constant_pool)?.to_owned(),
                };
                let exceptions = match method.known_attribute(KnownAttributeKind::Exceptions) {
                    Some(KnownAttribute::Exceptions(attribute)) => attribute
                        .indices
                        .iter()
                        .map(|&index| class.constant_pool.class_name(index).map(str::to_owned))
                        .collect::<java::Result<Vec<_>>>()?,
                    _ => Vec::new(),
                };
                declarations.insert(key, exceptions);
            }
        }
        Ok(Self { declarations })
    }

    /// Returns the declared exception internal names for one exact method.
    ///
    /// `None` means the declaration is absent from the catalog; `Some([])`
    /// proves that the method declares no checked exceptions.
    #[must_use]
    pub fn declared_exceptions(
        &self,
        owner: &str,
        name: &str,
        descriptor: &str,
    ) -> Option<&[String]> {
        self.declarations
            .get(&MethodKey {
                owner: owner.to_owned(),
                name: name.to_owned(),
                descriptor: descriptor.to_owned(),
            })
            .map(Vec::as_slice)
    }

    pub(crate) fn declares_no_exceptions(&self, owner: &str, name: &str, descriptor: &str) -> bool {
        self.declared_exceptions(owner, name, descriptor)
            .is_some_and(<[String]>::is_empty)
    }
}

#[cfg(test)]
mod tests {
    use java::classfile::{
        Attribute, ClassAccessFlags, ClassFile, IndexListAttribute, JAVA_8_MAJOR_VERSION,
        KnownAttribute, KnownAttributeKind, MethodAccessFlags,
    };

    use super::MethodExceptionCatalog;

    #[test]
    fn distinguishes_missing_empty_and_declared_exception_methods() {
        let mut class = ClassFile::new(
            JAVA_8_MAJOR_VERSION,
            "sample/Calls",
            Some("java/lang/Object"),
            ClassAccessFlags::PUBLIC | ClassAccessFlags::SUPER,
        )
        .expect("class");
        class
            .add_method(MethodAccessFlags::PUBLIC, "plain", "()V")
            .expect("plain method");
        let throwing = class
            .add_method(MethodAccessFlags::PUBLIC, "throwing", "()V")
            .expect("throwing method");
        let name_index = class
            .constant_pool
            .intern_utf8(KnownAttributeKind::Exceptions.name())
            .expect("attribute name");
        let exception = class
            .constant_pool
            .intern_class("java/io/IOException")
            .expect("exception class");
        class.methods[throwing]
            .attributes
            .push(Attribute::Known(KnownAttribute::Exceptions(
                IndexListAttribute {
                    name_index,
                    indices: vec![exception],
                },
            )));

        let catalog = MethodExceptionCatalog::from_classes([&class]).expect("catalog");

        assert!(catalog.declares_no_exceptions("sample/Calls", "plain", "()V"));
        assert!(!catalog.declares_no_exceptions("sample/Calls", "throwing", "()V"));
        assert!(
            catalog
                .declared_exceptions("sample/Calls", "throwing", "()V")
                .is_some_and(|exceptions| exceptions == ["java/io/IOException"])
        );
        assert_eq!(
            catalog.declared_exceptions("sample/Missing", "plain", "()V"),
            None
        );
    }
}

//! JVM verification-type hierarchy and array assignability.

use core::ops::ControlFlow;
use std::collections::{BTreeMap, BTreeSet};

use cfglib::{OpenBfsConfig, OpenBfsEvent, Visit, open_breadth_first_events};

use crate::Result;
use crate::classfile::ClassFile;

/// Internal name of the root Java object class.
pub const JAVA_LANG_OBJECT_NAME: &str = "java/lang/Object";
/// Internal name of the marker interface implemented by every array.
pub const JAVA_LANG_CLONEABLE_NAME: &str = "java/lang/Cloneable";
/// Internal name of the serialization marker implemented by every array.
pub const JAVA_IO_SERIALIZABLE_NAME: &str = "java/io/Serializable";

/// Reference-type relation used by JVM frame transfer and merging.
///
/// Object values use JVM internal names and arrays use JVM descriptors.
pub trait ReferenceHierarchy {
    /// Returns whether a value of `source` can be assigned to `target`.
    fn is_assignable(&self, source: &str, target: &str) -> bool;

    /// Returns a conservative common verification supertype.
    fn common_supertype(&self, left: &str, right: &str) -> Option<String>;
}

/// Editable class hierarchy assembled from class files or caller metadata.
///
/// Missing external types conservatively have only `java/lang/Object` as a
/// known ancestor. Callers analyzing a full classpath should insert every
/// available superclass and interface relationship.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ClassHierarchy {
    parents: BTreeMap<String, Vec<String>>,
}

impl ClassHierarchy {
    /// Creates an empty open-classpath hierarchy.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            parents: BTreeMap::new(),
        }
    }

    /// Builds a hierarchy from class declarations.
    ///
    /// # Errors
    ///
    /// Returns an error when a declaration has invalid class-pool indices.
    pub fn from_classes<'a>(classes: impl IntoIterator<Item = &'a ClassFile>) -> Result<Self> {
        let mut hierarchy = Self::new();
        for class in classes {
            let name = class.class_name()?.to_owned();
            let superclass = class.super_name()?.map(str::to_owned);
            let interfaces = class
                .interfaces
                .iter()
                .map(|&index| class.constant_pool.class_name(index).map(str::to_owned))
                .collect::<Result<Vec<_>>>()?;
            hierarchy.insert(name, superclass, interfaces);
        }
        Ok(hierarchy)
    }

    /// Inserts or replaces one class's direct superclass and interfaces.
    pub fn insert(
        &mut self,
        name: impl Into<String>,
        superclass: Option<impl Into<String>>,
        interfaces: impl IntoIterator<Item = impl Into<String>>,
    ) {
        let name = name.into();
        let mut direct = Vec::new();
        if let Some(superclass) = superclass {
            direct.push(superclass.into());
        } else if name != JAVA_LANG_OBJECT_NAME {
            direct.push(JAVA_LANG_OBJECT_NAME.to_owned());
        }
        for interface in interfaces {
            let interface = interface.into();
            if !direct.contains(&interface) {
                direct.push(interface);
            }
        }
        self.parents.insert(name, direct);
    }

    fn ancestors(&self, name: &str) -> Vec<String> {
        let mut output = Vec::new();
        let interrupted = open_breadth_first_events::<_, ()>(
            [name.to_owned()],
            OpenBfsConfig::new(),
            |current, parents| {
                if let Some(direct) = self.parents.get(current) {
                    parents.extend(direct.iter().cloned());
                } else if current != JAVA_LANG_OBJECT_NAME {
                    parents.push(JAVA_LANG_OBJECT_NAME.to_owned());
                }
            },
            |event| {
                if let OpenBfsEvent::Discover(current, _) = event {
                    output.push(current.clone());
                }
                ControlFlow::Continue(Visit::Descend)
            },
        );
        debug_assert!(interrupted.is_none(), "the ancestor walk never breaks");
        output
    }
}

impl ReferenceHierarchy for ClassHierarchy {
    fn is_assignable(&self, source: &str, target: &str) -> bool {
        if source == target || target == JAVA_LANG_OBJECT_NAME {
            return true;
        }
        if source.starts_with('[') {
            if matches!(target, JAVA_LANG_CLONEABLE_NAME | JAVA_IO_SERIALIZABLE_NAME) {
                return true;
            }
            if let (Some(source_component), Some(target_component)) =
                (array_component(source), array_component(target))
            {
                return match (source_component, target_component) {
                    (ArrayComponent::Reference(source), ArrayComponent::Reference(target)) => {
                        self.is_assignable(source, target)
                    }
                    (ArrayComponent::Primitive(source), ArrayComponent::Primitive(target)) => {
                        source == target
                    }
                    _ => false,
                };
            }
        }
        self.ancestors(source)
            .iter()
            .any(|ancestor| ancestor == target)
    }

    fn common_supertype(&self, left: &str, right: &str) -> Option<String> {
        if self.is_assignable(left, right) {
            return Some(right.to_owned());
        }
        if self.is_assignable(right, left) {
            return Some(left.to_owned());
        }
        if let (Some(ArrayComponent::Reference(left)), Some(ArrayComponent::Reference(right))) =
            (array_component(left), array_component(right))
            && let Some(component) = self.common_supertype(left, right)
        {
            return Some(array_of(&component));
        }
        let left_ancestors = self.ancestors(left).into_iter().collect::<BTreeSet<_>>();
        self.ancestors(right)
            .into_iter()
            .find(|ancestor| left_ancestors.contains(ancestor))
            .or_else(|| Some(JAVA_LANG_OBJECT_NAME.to_owned()))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ArrayComponent<'a> {
    Primitive(u8),
    Reference(&'a str),
}

fn array_component(descriptor: &str) -> Option<ArrayComponent<'_>> {
    let component = descriptor.strip_prefix('[')?;
    if component.starts_with('[') {
        Some(ArrayComponent::Reference(component))
    } else if let Some(name) = component
        .strip_prefix('L')
        .and_then(|name| name.strip_suffix(';'))
    {
        Some(ArrayComponent::Reference(name))
    } else if component.len() == 1 {
        Some(ArrayComponent::Primitive(component.as_bytes()[0]))
    } else {
        None
    }
}

fn array_of(component: &str) -> String {
    if component.starts_with('[') {
        format!("[{component}")
    } else {
        format!("[L{component};")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classes_and_arrays_follow_jvm_assignability() {
        let mut hierarchy = ClassHierarchy::new();
        hierarchy.insert("sample/Base", Some("java/lang/Object"), [] as [&str; 0]);
        hierarchy.insert("sample/Sub", Some("sample/Base"), [] as [&str; 0]);
        assert!(hierarchy.is_assignable("sample/Sub", "sample/Base"));
        assert!(!hierarchy.is_assignable("sample/Base", "sample/Sub"));
        assert_eq!(
            hierarchy.common_supertype("sample/Sub", "sample/Base"),
            Some("sample/Base".to_owned())
        );
        assert!(hierarchy.is_assignable("[I", JAVA_LANG_CLONEABLE_NAME));
        assert_eq!(
            hierarchy.common_supertype("[Lsample/Sub;", "[Lsample/Base;"),
            Some("[Lsample/Base;".to_owned())
        );
        assert_eq!(
            hierarchy.common_supertype("[I", "[J"),
            Some(JAVA_LANG_OBJECT_NAME.to_owned())
        );
    }
}

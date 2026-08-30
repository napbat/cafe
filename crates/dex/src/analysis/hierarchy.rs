//! Conservative reference hierarchy derived from DEX class definitions.

use core::ops::ControlFlow;
use std::collections::{BTreeMap, BTreeSet};

use cfglib::{OpenBfsConfig, OpenBfsEvent, Visit, open_breadth_first_events};

use crate::Result;
use crate::file::DexFile;

/// DEX descriptor of the root Java object class.
pub const JAVA_LANG_OBJECT_DESCRIPTOR: &str = "Ljava/lang/Object;";
/// DEX descriptor of the marker interface implemented by every array.
pub const JAVA_LANG_CLONEABLE_DESCRIPTOR: &str = "Ljava/lang/Cloneable;";
/// DEX descriptor of the serialization marker implemented by every array.
pub const JAVA_IO_SERIALIZABLE_DESCRIPTOR: &str = "Ljava/io/Serializable;";

/// Reference-type relation used when data-flow paths merge.
pub trait ReferenceHierarchy {
    /// Returns whether a value of `source` can be assigned to `target`.
    fn is_assignable(&self, source: &str, target: &str) -> bool;

    /// Returns a conservative common supertype descriptor.
    fn common_supertype(&self, left: &str, right: &str) -> Option<String>;
}

/// Hierarchy formed from classes declared by one DEX file.
///
/// Unknown external relationships conservatively merge to `java/lang/Object`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DexHierarchy {
    parents: BTreeMap<String, Vec<String>>,
}

impl DexHierarchy {
    /// Creates an empty open-classpath hierarchy.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            parents: BTreeMap::new(),
        }
    }

    /// Resolves class, superclass, and interface descriptors from a DEX file.
    ///
    /// # Errors
    ///
    /// Returns an error when a class definition contains an invalid type index.
    pub fn from_file(file: &DexFile) -> Result<Self> {
        Self::from_files([file])
    }

    /// Resolves declarations from every DEX file in one classpath.
    ///
    /// Later definitions of the same descriptor replace earlier definitions,
    /// matching [`Self::insert`]. Use `classpath::ClasspathHierarchy` when
    /// conflicting cross-artifact declarations must be diagnosed.
    ///
    /// # Errors
    ///
    /// Returns an error when a class definition contains an invalid type index.
    pub fn from_files<'a>(files: impl IntoIterator<Item = &'a DexFile>) -> Result<Self> {
        let mut hierarchy = Self::new();
        for file in files {
            hierarchy.extend_file(file)?;
        }
        Ok(hierarchy)
    }

    /// Adds every declaration from one DEX file.
    ///
    /// # Errors
    ///
    /// Returns an error when a class definition contains an invalid type index.
    pub fn extend_file(&mut self, file: &DexFile) -> Result<()> {
        for class in file.classes() {
            let name = file.type_descriptor(class.class)?.to_owned();
            let superclass = class
                .superclass
                .map(|index| file.type_descriptor(index).map(str::to_owned))
                .transpose()?;
            let interfaces = class
                .interfaces
                .iter()
                .map(|&index| file.type_descriptor(index).map(str::to_owned))
                .collect::<Result<Vec<_>>>()?;
            self.insert(name, superclass, interfaces);
        }
        Ok(())
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
        } else if name != JAVA_LANG_OBJECT_DESCRIPTOR {
            direct.push(JAVA_LANG_OBJECT_DESCRIPTOR.to_owned());
        }
        for interface in interfaces {
            let interface = interface.into();
            if !direct.contains(&interface) {
                direct.push(interface);
            }
        }
        self.parents.insert(name, direct);
    }

    fn ancestors(&self, descriptor: &str) -> Vec<String> {
        let mut output = Vec::new();
        let interrupted = open_breadth_first_events::<_, ()>(
            [descriptor.to_owned()],
            OpenBfsConfig::new(),
            |current, parents| {
                if let Some(direct) = self.parents.get(current) {
                    parents.extend(direct.iter().cloned());
                } else if current != JAVA_LANG_OBJECT_DESCRIPTOR {
                    parents.push(JAVA_LANG_OBJECT_DESCRIPTOR.to_owned());
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

impl ReferenceHierarchy for DexHierarchy {
    fn is_assignable(&self, source: &str, target: &str) -> bool {
        if source == target || target == JAVA_LANG_OBJECT_DESCRIPTOR {
            return true;
        }
        if source.starts_with('[') {
            if matches!(
                target,
                JAVA_LANG_CLONEABLE_DESCRIPTOR | JAVA_IO_SERIALIZABLE_DESCRIPTOR
            ) {
                return true;
            }
            if let (Some(source_component), Some(target_component)) = (
                reference_array_component(source),
                reference_array_component(target),
            ) {
                return self.is_assignable(source_component, target_component);
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
        if let (Some(left_component), Some(right_component)) = (
            reference_array_component(left),
            reference_array_component(right),
        ) && let Some(component) = self.common_supertype(left_component, right_component)
        {
            return Some(format!("[{component}"));
        }
        let left_ancestors = self.ancestors(left).into_iter().collect::<BTreeSet<_>>();
        self.ancestors(right)
            .into_iter()
            .find(|ancestor| left_ancestors.contains(ancestor))
            .or_else(|| Some(JAVA_LANG_OBJECT_DESCRIPTOR.to_owned()))
    }
}

fn reference_array_component(descriptor: &str) -> Option<&str> {
    let component = descriptor.strip_prefix('[')?;
    (component.starts_with('L') || component.starts_with('[')).then_some(component)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arrays_use_jvm_reference_merge_rules() {
        let hierarchy = DexHierarchy::new();
        assert!(hierarchy.is_assignable("[I", JAVA_LANG_OBJECT_DESCRIPTOR));
        assert!(hierarchy.is_assignable("[I", JAVA_LANG_CLONEABLE_DESCRIPTOR));
        assert_eq!(
            hierarchy.common_supertype("[Ljava/lang/String;", "[Ljava/lang/Integer;"),
            Some("[Ljava/lang/Object;".to_owned())
        );
        assert_eq!(
            hierarchy.common_supertype("[I", "[J"),
            Some(JAVA_LANG_OBJECT_DESCRIPTOR.to_owned())
        );
    }
}

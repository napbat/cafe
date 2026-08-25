//! Unified hierarchy storage and native analysis views.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use crate::{ClassDeclaration, ClassDescriptor};

const OBJECT_DESCRIPTOR: &str = "Ljava/lang/Object;";
const CLONEABLE_DESCRIPTOR: &str = "Ljava/lang/Cloneable;";
const SERIALIZABLE_DESCRIPTOR: &str = "Ljava/io/Serializable;";
const MAX_ARRAY_DIMENSIONS: usize = 255;

/// Unified declaration hierarchy across JVM class files and DEX artifacts.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ClasspathHierarchy {
    pub(crate) declarations: BTreeMap<ClassDescriptor, ClassDeclaration>,
}

impl ClasspathHierarchy {
    /// Creates an empty open-classpath hierarchy.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            declarations: BTreeMap::new(),
        }
    }

    /// Returns the declaration for a canonical object descriptor.
    #[must_use]
    pub fn declaration(&self, descriptor: &ClassDescriptor) -> Option<&ClassDeclaration> {
        self.declarations.get(descriptor)
    }

    /// Iterates through declarations in canonical descriptor order.
    #[must_use]
    pub fn declarations(&self) -> impl ExactSizeIterator<Item = &ClassDeclaration> {
        self.declarations.values()
    }

    /// Returns the number of distinct canonical classes.
    #[must_use]
    pub fn len(&self) -> usize {
        self.declarations.len()
    }

    /// Whether the classpath contains no declarations.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.declarations.is_empty()
    }

    /// Returns a JVM-name adapter for frame analysis.
    #[must_use]
    pub const fn jvm_view(&self) -> JvmHierarchyView<'_> {
        JvmHierarchyView { hierarchy: self }
    }

    /// Returns a DEX-descriptor adapter for register analysis.
    #[must_use]
    pub const fn dex_view(&self) -> DexHierarchyView<'_> {
        DexHierarchyView { hierarchy: self }
    }

    fn ancestors(&self, descriptor: &str) -> Vec<String> {
        let mut output = Vec::new();
        let mut seen = BTreeSet::new();
        let mut queue = VecDeque::from([descriptor.to_owned()]);
        while let Some(current) = queue.pop_front() {
            if !seen.insert(current.clone()) {
                continue;
            }
            output.push(current.clone());
            let parents = ClassDescriptor::from_dex_descriptor(current.clone())
                .ok()
                .and_then(|name| self.declarations.get(&name));
            if let Some(declaration) = parents {
                queue.extend(
                    declaration
                        .parents
                        .iter()
                        .map(|parent| parent.as_descriptor().to_owned()),
                );
            } else if current != OBJECT_DESCRIPTOR {
                queue.push_back(OBJECT_DESCRIPTOR.to_owned());
            }
        }
        output
    }

    fn is_assignable_canonical(&self, source: &str, target: &str) -> bool {
        if source == target || target == OBJECT_DESCRIPTOR {
            return true;
        }
        if source.starts_with('[') {
            if matches!(target, CLONEABLE_DESCRIPTOR | SERIALIZABLE_DESCRIPTOR) {
                return true;
            }
            if let (Some(source), Some(target)) = (array_component(source), array_component(target))
            {
                return match (source, target) {
                    (ArrayComponent::Reference(source), ArrayComponent::Reference(target)) => {
                        self.is_assignable_canonical(source, target)
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

    fn common_supertype_canonical(&self, left: &str, right: &str) -> String {
        if self.is_assignable_canonical(left, right) {
            return right.to_owned();
        }
        if self.is_assignable_canonical(right, left) {
            return left.to_owned();
        }
        if let (Some(ArrayComponent::Reference(left)), Some(ArrayComponent::Reference(right))) =
            (array_component(left), array_component(right))
        {
            return format!("[{}", self.common_supertype_canonical(left, right));
        }
        let left_ancestors = self.ancestors(left).into_iter().collect::<BTreeSet<_>>();
        self.ancestors(right)
            .into_iter()
            .find(|ancestor| left_ancestors.contains(ancestor))
            .unwrap_or_else(|| OBJECT_DESCRIPTOR.to_owned())
    }
}

/// Borrowed JVM internal-name view over a unified classpath.
#[derive(Debug, Clone, Copy)]
pub struct JvmHierarchyView<'a> {
    hierarchy: &'a ClasspathHierarchy,
}

impl java::analysis::ReferenceHierarchy for JvmHierarchyView<'_> {
    fn is_assignable(&self, source: &str, target: &str) -> bool {
        let (Some(source), Some(target)) = (
            normalize_jvm_reference(source),
            normalize_jvm_reference(target),
        ) else {
            return false;
        };
        self.hierarchy.is_assignable_canonical(&source, &target)
    }

    fn common_supertype(&self, left: &str, right: &str) -> Option<String> {
        let (Some(left), Some(right)) = (
            normalize_jvm_reference(left),
            normalize_jvm_reference(right),
        ) else {
            return Some(java::analysis::JAVA_LANG_OBJECT_NAME.to_owned());
        };
        Some(render_jvm_reference(
            self.hierarchy.common_supertype_canonical(&left, &right),
        ))
    }
}

/// Borrowed DEX descriptor view over a unified classpath.
#[derive(Debug, Clone, Copy)]
pub struct DexHierarchyView<'a> {
    hierarchy: &'a ClasspathHierarchy,
}

impl dex::analysis::ReferenceHierarchy for DexHierarchyView<'_> {
    fn is_assignable(&self, source: &str, target: &str) -> bool {
        if !valid_dex_reference(source) || !valid_dex_reference(target) {
            return false;
        }
        self.hierarchy.is_assignable_canonical(source, target)
    }

    fn common_supertype(&self, left: &str, right: &str) -> Option<String> {
        if !valid_dex_reference(left) || !valid_dex_reference(right) {
            return Some(OBJECT_DESCRIPTOR.to_owned());
        }
        Some(self.hierarchy.common_supertype_canonical(left, right))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ArrayComponent<'a> {
    Primitive(u8),
    Reference(&'a str),
}

fn array_component(descriptor: &str) -> Option<ArrayComponent<'_>> {
    let component = descriptor.strip_prefix('[')?;
    if component.starts_with('[') && valid_dex_reference(component)
        || ClassDescriptor::from_dex_descriptor(component.to_owned()).is_ok()
    {
        Some(ArrayComponent::Reference(component))
    } else if component.len() == 1
        && matches!(
            component.as_bytes()[0],
            b'B' | b'C' | b'D' | b'F' | b'I' | b'J' | b'S' | b'Z'
        )
    {
        Some(ArrayComponent::Primitive(component.as_bytes()[0]))
    } else {
        None
    }
}

fn normalize_jvm_reference(name: &str) -> Option<String> {
    if name.starts_with('[') {
        valid_dex_reference(name).then(|| name.to_owned())
    } else {
        ClassDescriptor::from_jvm_internal(name.to_owned())
            .ok()
            .map(|descriptor| descriptor.as_descriptor().to_owned())
    }
}

fn render_jvm_reference(descriptor: String) -> String {
    if descriptor.starts_with('[') {
        descriptor
    } else {
        descriptor
            .strip_prefix('L')
            .and_then(|value| value.strip_suffix(';'))
            .expect("canonical object descriptor")
            .to_owned()
    }
}

fn valid_dex_reference(descriptor: &str) -> bool {
    if descriptor.starts_with('[') {
        let dimensions = descriptor.bytes().take_while(|&byte| byte == b'[').count();
        return dimensions <= MAX_ARRAY_DIMENSIONS && array_component(descriptor).is_some();
    }
    ClassDescriptor::from_dex_descriptor(descriptor.to_owned()).is_ok()
}

//! Uniform attribute lookup and replacement on every attribute-bearing model.

use super::{KnownAttribute, KnownAttributeKind};
use crate::classfile::{
    Attribute, ClassFile, CodeAttribute, FieldInfo, MethodInfo, RecordComponent,
};

macro_rules! impl_attribute_access {
    ($owner:ty) => {
        impl $owner {
            /// Finds the first attribute with this class-file name.
            #[must_use]
            pub fn attribute(&self, name: &str) -> Option<&Attribute> {
                self.attributes
                    .iter()
                    .find(|attribute| attribute.name() == name)
            }

            /// Finds the first mutable attribute with this class-file name.
            #[must_use]
            pub fn attribute_mut(&mut self, name: &str) -> Option<&mut Attribute> {
                self.attributes
                    .iter_mut()
                    .find(|attribute| attribute.name() == name)
            }

            /// Finds the first recognized attribute of a typed kind.
            #[must_use]
            pub fn known_attribute(&self, kind: KnownAttributeKind) -> Option<&KnownAttribute> {
                self.attributes
                    .iter()
                    .find_map(|attribute| match attribute {
                        Attribute::Known(attribute) if attribute.kind() == kind => Some(attribute),
                        Attribute::Code(_) | Attribute::Known(_) | Attribute::Raw(_) => None,
                    })
            }

            /// Finds the first mutable recognized attribute of a typed kind.
            #[must_use]
            pub fn known_attribute_mut(
                &mut self,
                kind: KnownAttributeKind,
            ) -> Option<&mut KnownAttribute> {
                self.attributes
                    .iter_mut()
                    .find_map(|attribute| match attribute {
                        Attribute::Known(attribute) if attribute.kind() == kind => Some(attribute),
                        Attribute::Code(_) | Attribute::Known(_) | Attribute::Raw(_) => None,
                    })
            }

            /// Replaces all same-named attributes with one value at their first position.
            ///
            /// Returns the removed values in their original order. If no matching
            /// value exists, the new attribute is appended.
            pub fn set_attribute(&mut self, attribute: Attribute) -> Vec<Attribute> {
                set_attribute(&mut self.attributes, attribute)
            }

            /// Removes every attribute with this class-file name.
            pub fn remove_attributes(&mut self, name: &str) -> Vec<Attribute> {
                remove_attributes(&mut self.attributes, name)
            }
        }
    };
}

impl_attribute_access!(ClassFile);
impl_attribute_access!(FieldInfo);
impl_attribute_access!(MethodInfo);
impl_attribute_access!(CodeAttribute);
impl_attribute_access!(RecordComponent);

fn set_attribute(attributes: &mut Vec<Attribute>, attribute: Attribute) -> Vec<Attribute> {
    let name = attribute.name().to_owned();
    let insertion = attributes
        .iter()
        .position(|existing| existing.name() == name)
        .unwrap_or(attributes.len());
    let removed = remove_attributes(attributes, &name);
    attributes.insert(insertion, attribute);
    removed
}

fn remove_attributes(attributes: &mut Vec<Attribute>, name: &str) -> Vec<Attribute> {
    let mut removed = Vec::new();
    let mut retained = Vec::with_capacity(attributes.len());
    for attribute in std::mem::take(attributes) {
        if attribute.name() == name {
            removed.push(attribute);
        } else {
            retained.push(attribute);
        }
    }
    *attributes = retained;
    removed
}

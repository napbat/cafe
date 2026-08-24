//! Native declaration collections and overload-aware export plans.

use std::collections::HashMap;

use crate::method::{NativeMethod, NativeMethodId};
use crate::symbol::{NativeSymbol, SymbolStyle};
use crate::{Error, Result};

/// One native declaration paired with the symbol it should export.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct NativeBinding<'a> {
    method: &'a NativeMethod,
    symbol: NativeSymbol,
    style: SymbolStyle,
}

impl<'a> NativeBinding<'a> {
    /// Returns the Java native declaration.
    #[must_use]
    pub const fn method(&self) -> &'a NativeMethod {
        self.method
    }

    /// Returns the symbol to export from a native library.
    #[must_use]
    pub const fn symbol(&self) -> &NativeSymbol {
        &self.symbol
    }

    /// Returns why the short or overload-qualified form was selected.
    #[must_use]
    pub const fn style(&self) -> SymbolStyle {
        self.style
    }
}

/// Ordered set of native declarations with unique JVM identities.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NativeMethods {
    methods: Vec<NativeMethod>,
}

impl NativeMethods {
    /// Creates an empty native declaration set.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            methods: Vec::new(),
        }
    }

    /// Creates a declaration set while checking exact identity uniqueness.
    ///
    /// # Errors
    ///
    /// Returns an error for the first repeated class, name, and descriptor.
    pub fn from_methods(methods: impl IntoIterator<Item = NativeMethod>) -> Result<Self> {
        let mut collection = Self::new();
        collection.extend(methods)?;
        Ok(collection)
    }

    /// Returns the number of native declarations.
    #[must_use]
    pub fn len(&self) -> usize {
        self.methods.len()
    }

    /// Returns whether there are no native declarations.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.methods.is_empty()
    }

    /// Returns declarations in source order.
    #[must_use]
    pub fn as_slice(&self) -> &[NativeMethod] {
        &self.methods
    }

    /// Iterates declarations in source order.
    #[must_use]
    pub fn iter(&self) -> impl ExactSizeIterator<Item = &NativeMethod> {
        self.methods.iter()
    }

    /// Adds one declaration if its exact JVM identity is new.
    ///
    /// # Errors
    ///
    /// Returns an error if the declaration is already present.
    pub fn insert(&mut self, method: NativeMethod) -> Result<()> {
        if self.methods.iter().any(|value| value.id() == method.id()) {
            return Err(Error::DuplicateNativeMethod {
                method: Box::new(method.id().clone()),
            });
        }
        self.methods.push(method);
        Ok(())
    }

    /// Adds declarations in order.
    ///
    /// # Errors
    ///
    /// Returns an error for the first repeated identity. Declarations preceding
    /// the duplicate remain inserted.
    pub fn extend(&mut self, methods: impl IntoIterator<Item = NativeMethod>) -> Result<()> {
        for method in methods {
            self.insert(method)?;
        }
        Ok(())
    }

    /// Selects short symbols for unique native names and long symbols for
    /// overloaded native names within the same declaring class.
    ///
    /// Only declarations in this collection participate in overload detection,
    /// so ordinary non-native Java overloads do not force a long export.
    ///
    /// # Errors
    ///
    /// Returns an error if JNI escaping fails or two declarations map to the
    /// same required export symbol.
    pub fn bindings(&self) -> Result<Vec<NativeBinding<'_>>> {
        let mut bindings = Vec::with_capacity(self.methods.len());
        let mut symbols = HashMap::<NativeSymbol, NativeMethodId>::new();
        for method in &self.methods {
            let style = if self.is_native_name_overloaded(method) {
                SymbolStyle::Long
            } else {
                SymbolStyle::Short
            };
            let symbol = method.symbol(style)?;
            if let Some(first) = symbols.insert(symbol.clone(), method.id().clone()) {
                return Err(Error::NativeSymbolCollision {
                    symbol,
                    first: Box::new(first),
                    second: Box::new(method.id().clone()),
                });
            }
            bindings.push(NativeBinding {
                method,
                symbol,
                style,
            });
        }
        Ok(bindings)
    }

    fn is_native_name_overloaded(&self, method: &NativeMethod) -> bool {
        self.methods
            .iter()
            .filter(|candidate| {
                candidate.owner() == method.owner() && candidate.name() == method.name()
            })
            .take(2)
            .count()
            > 1
    }
}

impl<'a> IntoIterator for &'a NativeMethods {
    type Item = &'a NativeMethod;
    type IntoIter = std::slice::Iter<'a, NativeMethod>;

    fn into_iter(self) -> Self::IntoIter {
        self.methods.iter()
    }
}

impl IntoIterator for NativeMethods {
    type Item = NativeMethod;
    type IntoIter = std::vec::IntoIter<NativeMethod>;

    fn into_iter(self) -> Self::IntoIter {
        self.methods.into_iter()
    }
}

#[cfg(test)]
mod tests {
    use super::NativeMethods;
    use crate::Error;
    use crate::method::{InvocationKind, NativeMethod};
    use crate::symbol::SymbolStyle;

    #[test]
    fn selects_long_symbols_only_for_native_overloads() {
        let methods = NativeMethods::from_methods([
            NativeMethod::new("sample/Native", "open", "()V", InvocationKind::Static).unwrap(),
            NativeMethod::new("sample/Native", "read", "()I", InvocationKind::Instance).unwrap(),
            NativeMethod::new("sample/Native", "read", "(I)I", InvocationKind::Instance).unwrap(),
        ])
        .unwrap();
        let bindings = methods.bindings().unwrap();

        assert_eq!(bindings[0].style(), SymbolStyle::Short);
        assert_eq!(bindings[1].style(), SymbolStyle::Long);
        assert_eq!(bindings[2].style(), SymbolStyle::Long);
        assert_eq!(bindings[1].symbol().as_str(), "Java_sample_Native_read__");
        assert_eq!(bindings[2].symbol().as_str(), "Java_sample_Native_read__I");
    }

    #[test]
    fn rejects_exact_duplicate_declarations() {
        let method =
            NativeMethod::new("sample/Native", "open", "()V", InvocationKind::Static).unwrap();
        let error = NativeMethods::from_methods([method.clone(), method]).unwrap_err();

        assert!(matches!(error, Error::DuplicateNativeMethod { .. }));
    }

    #[test]
    fn reports_return_only_overload_symbol_collisions() {
        let methods = NativeMethods::from_methods([
            NativeMethod::new("sample/Native", "read", "()I", InvocationKind::Instance).unwrap(),
            NativeMethod::new(
                "sample/Native",
                "read",
                "()Ljava/lang/String;",
                InvocationKind::Instance,
            )
            .unwrap(),
        ])
        .unwrap();

        assert!(matches!(
            methods.bindings().unwrap_err(),
            Error::NativeSymbolCollision { .. }
        ));
    }
}

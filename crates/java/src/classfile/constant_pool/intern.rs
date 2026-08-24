//! Deduplicating constructors for editable constant pools.

use crate::Result;

use super::{Constant, ConstantPool, MethodHandleKind, Utf8Constant};

impl ConstantPool {
    /// Finds the first constant with the same exact encoded value.
    #[must_use]
    pub fn find(&self, wanted: &Constant) -> Option<u16> {
        self.iter()
            .find_map(|(index, value)| constants_equal(value, wanted).then_some(index))
    }

    /// Returns an existing identical constant or appends it.
    ///
    /// Floating-point values are compared by their encoded IEEE bit patterns,
    /// and modified UTF-8 values are compared by exact UTF-16 code units.
    ///
    /// # Errors
    ///
    /// Returns an error if the constant is unusable or the pool is full.
    pub fn intern(&mut self, constant: Constant) -> Result<u16> {
        if let Some(index) = self.find(&constant) {
            Ok(index)
        } else {
            self.push(constant)
        }
    }

    /// Interns a Rust string as a modified UTF-8 constant.
    ///
    /// # Errors
    ///
    /// Returns an error if the constant pool is full.
    pub fn intern_utf8(&mut self, value: &str) -> Result<u16> {
        self.intern(Constant::Utf8(Utf8Constant::new(value)))
    }

    /// Interns exact Java UTF-16 code units as a modified UTF-8 constant.
    ///
    /// # Errors
    ///
    /// Returns an error if the constant pool is full.
    pub fn intern_utf16(&mut self, units: Vec<u16>) -> Result<u16> {
        self.intern(Constant::Utf8(Utf8Constant::from_utf16(units)))
    }

    /// Interns a class or interface symbolic name and its UTF-8 dependency.
    ///
    /// # Errors
    ///
    /// Returns an error if the constant pool is full.
    pub fn intern_class(&mut self, name: &str) -> Result<u16> {
        let name_index = self.intern_utf8(name)?;
        self.intern(Constant::Class { name_index })
    }

    /// Interns a Java string constant and its exact UTF-8 dependency.
    ///
    /// # Errors
    ///
    /// Returns an error if the constant pool is full.
    pub fn intern_string(&mut self, value: &str) -> Result<u16> {
        let string_index = self.intern_utf8(value)?;
        self.intern(Constant::String { string_index })
    }

    /// Interns a field-or-method name and descriptor pair.
    ///
    /// # Errors
    ///
    /// Returns an error if the constant pool is full.
    pub fn intern_name_and_type(&mut self, name: &str, descriptor: &str) -> Result<u16> {
        let name_index = self.intern_utf8(name)?;
        let descriptor_index = self.intern_utf8(descriptor)?;
        self.intern(Constant::NameAndType {
            name_index,
            descriptor_index,
        })
    }

    /// Interns a symbolic field reference and all dependencies.
    ///
    /// # Errors
    ///
    /// Returns an error if the constant pool is full.
    pub fn intern_field_ref(&mut self, owner: &str, name: &str, descriptor: &str) -> Result<u16> {
        let class_index = self.intern_class(owner)?;
        let name_and_type_index = self.intern_name_and_type(name, descriptor)?;
        self.intern(Constant::FieldRef {
            class_index,
            name_and_type_index,
        })
    }

    /// Interns a symbolic class-method reference and all dependencies.
    ///
    /// # Errors
    ///
    /// Returns an error if the constant pool is full.
    pub fn intern_method_ref(&mut self, owner: &str, name: &str, descriptor: &str) -> Result<u16> {
        let class_index = self.intern_class(owner)?;
        let name_and_type_index = self.intern_name_and_type(name, descriptor)?;
        self.intern(Constant::MethodRef {
            class_index,
            name_and_type_index,
        })
    }

    /// Interns a symbolic interface-method reference and all dependencies.
    ///
    /// # Errors
    ///
    /// Returns an error if the constant pool is full.
    pub fn intern_interface_method_ref(
        &mut self,
        owner: &str,
        name: &str,
        descriptor: &str,
    ) -> Result<u16> {
        let class_index = self.intern_class(owner)?;
        let name_and_type_index = self.intern_name_and_type(name, descriptor)?;
        self.intern(Constant::InterfaceMethodRef {
            class_index,
            name_and_type_index,
        })
    }

    /// Interns a method-handle constant.
    ///
    /// # Errors
    ///
    /// Returns an error if the constant pool is full.
    pub fn intern_method_handle(
        &mut self,
        reference_kind: MethodHandleKind,
        reference_index: u16,
    ) -> Result<u16> {
        self.intern(Constant::MethodHandle {
            reference_kind,
            reference_index,
        })
    }

    /// Interns a method-type constant and its descriptor dependency.
    ///
    /// # Errors
    ///
    /// Returns an error if the constant pool is full.
    pub fn intern_method_type(&mut self, descriptor: &str) -> Result<u16> {
        let descriptor_index = self.intern_utf8(descriptor)?;
        self.intern(Constant::MethodType { descriptor_index })
    }

    /// Interns a dynamic constant and its name-and-type dependency.
    ///
    /// # Errors
    ///
    /// Returns an error if the constant pool is full.
    pub fn intern_dynamic(
        &mut self,
        bootstrap_method_attr_index: u16,
        name: &str,
        descriptor: &str,
    ) -> Result<u16> {
        let name_and_type_index = self.intern_name_and_type(name, descriptor)?;
        self.intern(Constant::Dynamic {
            bootstrap_method_attr_index,
            name_and_type_index,
        })
    }

    /// Interns an invokedynamic call-site constant and its dependencies.
    ///
    /// # Errors
    ///
    /// Returns an error if the constant pool is full.
    pub fn intern_invoke_dynamic(
        &mut self,
        bootstrap_method_attr_index: u16,
        name: &str,
        descriptor: &str,
    ) -> Result<u16> {
        let name_and_type_index = self.intern_name_and_type(name, descriptor)?;
        self.intern(Constant::InvokeDynamic {
            bootstrap_method_attr_index,
            name_and_type_index,
        })
    }

    /// Interns a Java module name and its UTF-8 dependency.
    ///
    /// # Errors
    ///
    /// Returns an error if the constant pool is full.
    pub fn intern_module(&mut self, name: &str) -> Result<u16> {
        let name_index = self.intern_utf8(name)?;
        self.intern(Constant::Module { name_index })
    }

    /// Interns a Java package name and its UTF-8 dependency.
    ///
    /// # Errors
    ///
    /// Returns an error if the constant pool is full.
    pub fn intern_package(&mut self, name: &str) -> Result<u16> {
        let name_index = self.intern_utf8(name)?;
        self.intern(Constant::Package { name_index })
    }
}

#[allow(clippy::float_cmp)]
fn constants_equal(left: &Constant, right: &Constant) -> bool {
    match (left, right) {
        (Constant::Float(left), Constant::Float(right)) => left.to_bits() == right.to_bits(),
        (Constant::Double(left), Constant::Double(right)) => left.to_bits() == right.to_bits(),
        _ => left == right,
    }
}

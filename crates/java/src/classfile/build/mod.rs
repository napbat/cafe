//! Ergonomic constructors and indexed mutation for JVM class models.

use crate::bytecode::{self, Instruction};
use crate::{Result, descriptor};

use super::{
    Attribute, ClassAccessFlags, ClassFile, CodeAttribute, ConstantPool, FieldAccessFlags,
    FieldInfo, MethodAccessFlags, MethodInfo, NO_SUPER_CLASS_INDEX,
};

impl ClassFile {
    /// Creates an editable class with an initialized constant pool.
    ///
    /// Pass `None` for the superclass only for `java/lang/Object` or a
    /// module-info class. No declaration members or attributes are added.
    ///
    /// # Errors
    ///
    /// Returns an error if the required constants do not fit in the pool.
    pub fn new(
        major_version: u16,
        name: &str,
        super_name: Option<&str>,
        access_flags: ClassAccessFlags,
    ) -> Result<Self> {
        let mut constant_pool = ConstantPool::new();
        let this_class = constant_pool.intern_class(name)?;
        let super_class = super_name
            .map(|name| constant_pool.intern_class(name))
            .transpose()?
            .unwrap_or(NO_SUPER_CLASS_INDEX);
        Ok(Self {
            minor_version: 0,
            major_version,
            constant_pool,
            access_flags,
            this_class,
            super_class,
            interfaces: Vec::new(),
            fields: Vec::new(),
            methods: Vec::new(),
            attributes: Vec::new(),
        })
    }

    /// Changes the declared class name using an interned class constant.
    ///
    /// # Errors
    ///
    /// Returns an error if the constant pool is full.
    pub fn set_class_name(&mut self, name: &str) -> Result<()> {
        self.this_class = self.constant_pool.intern_class(name)?;
        Ok(())
    }

    /// Changes or clears the declared superclass.
    ///
    /// # Errors
    ///
    /// Returns an error if a new class constant cannot be added.
    pub fn set_super_name(&mut self, name: Option<&str>) -> Result<()> {
        self.super_class = name
            .map(|name| self.constant_pool.intern_class(name))
            .transpose()?
            .unwrap_or(NO_SUPER_CLASS_INDEX);
        Ok(())
    }

    /// Adds an interface unless it is already listed and returns its class index.
    ///
    /// # Errors
    ///
    /// Returns an error if the constant pool is full.
    pub fn add_interface(&mut self, name: &str) -> Result<u16> {
        let index = self.constant_pool.intern_class(name)?;
        if !self.interfaces.contains(&index) {
            self.interfaces.push(index);
        }
        Ok(index)
    }

    /// Creates and appends a field, returning its position.
    ///
    /// # Errors
    ///
    /// Returns an error for an invalid descriptor or a full constant pool.
    pub fn add_field(
        &mut self,
        access_flags: FieldAccessFlags,
        name: &str,
        descriptor: &str,
    ) -> Result<usize> {
        let field = FieldInfo::new(&mut self.constant_pool, access_flags, name, descriptor)?;
        self.fields.push(field);
        Ok(self.fields.len() - 1)
    }

    /// Creates and appends a method, returning its position.
    ///
    /// # Errors
    ///
    /// Returns an error for an invalid descriptor or a full constant pool.
    pub fn add_method(
        &mut self,
        access_flags: MethodAccessFlags,
        name: &str,
        descriptor: &str,
    ) -> Result<usize> {
        let method = MethodInfo::new(&mut self.constant_pool, access_flags, name, descriptor)?;
        self.methods.push(method);
        Ok(self.methods.len() - 1)
    }

    /// Looks up a field by its overload-qualified name and descriptor.
    ///
    /// # Errors
    ///
    /// Returns an error if a declaration has invalid constant-pool indices.
    pub fn field(&self, name: &str, descriptor: &str) -> Result<Option<&FieldInfo>> {
        let position = find_field(self, name, descriptor)?;
        Ok(position.map(|position| &self.fields[position]))
    }

    /// Looks up a mutable field by its overload-qualified name and descriptor.
    ///
    /// # Errors
    ///
    /// Returns an error if a declaration has invalid constant-pool indices.
    pub fn field_mut(&mut self, name: &str, descriptor: &str) -> Result<Option<&mut FieldInfo>> {
        let position = find_field(self, name, descriptor)?;
        Ok(position.map(|position| &mut self.fields[position]))
    }

    /// Looks up a method by its overload-qualified name and descriptor.
    ///
    /// # Errors
    ///
    /// Returns an error if a declaration has invalid constant-pool indices.
    pub fn method(&self, name: &str, descriptor: &str) -> Result<Option<&MethodInfo>> {
        let position = find_method(self, name, descriptor)?;
        Ok(position.map(|position| &self.methods[position]))
    }

    /// Looks up a mutable method by its overload-qualified name and descriptor.
    ///
    /// # Errors
    ///
    /// Returns an error if a declaration has invalid constant-pool indices.
    pub fn method_mut(&mut self, name: &str, descriptor: &str) -> Result<Option<&mut MethodInfo>> {
        let position = find_method(self, name, descriptor)?;
        Ok(position.map(|position| &mut self.methods[position]))
    }
}

impl FieldInfo {
    /// Creates a field and interns its name and descriptor.
    ///
    /// # Errors
    ///
    /// Returns an error for an invalid field descriptor or a full constant pool.
    pub fn new(
        pool: &mut ConstantPool,
        access_flags: FieldAccessFlags,
        name: &str,
        descriptor: &str,
    ) -> Result<Self> {
        descriptor::parse_field(descriptor)?;
        Ok(Self {
            access_flags,
            name_index: pool.intern_utf8(name)?,
            descriptor_index: pool.intern_utf8(descriptor)?,
            attributes: Vec::new(),
        })
    }
}

impl MethodInfo {
    /// Creates a method and interns its name and descriptor.
    ///
    /// # Errors
    ///
    /// Returns an error for an invalid method descriptor or a full constant pool.
    pub fn new(
        pool: &mut ConstantPool,
        access_flags: MethodAccessFlags,
        name: &str,
        descriptor: &str,
    ) -> Result<Self> {
        descriptor::parse_method(descriptor)?;
        Ok(Self {
            access_flags,
            name_index: pool.intern_utf8(name)?,
            descriptor_index: pool.intern_utf8(descriptor)?,
            attributes: Vec::new(),
        })
    }

    /// Installs or replaces this method's sole `Code` attribute.
    ///
    /// Returns every previous code attribute so malformed duplicate inputs can
    /// be normalized without losing caller visibility.
    pub fn set_code(&mut self, code: CodeAttribute) -> Vec<CodeAttribute> {
        let mut removed = Vec::new();
        let mut retained = Vec::with_capacity(self.attributes.len() + 1);
        let insertion = self
            .attributes
            .iter()
            .position(|attribute| matches!(attribute, Attribute::Code(_)))
            .unwrap_or(self.attributes.len());
        for attribute in std::mem::take(&mut self.attributes) {
            if let Attribute::Code(code) = attribute {
                removed.push(code);
            } else {
                retained.push(attribute);
            }
        }
        retained.insert(insertion, Attribute::Code(code));
        self.attributes = retained;
        removed
    }

    /// Removes and returns every `Code` attribute.
    pub fn remove_code(&mut self) -> Vec<CodeAttribute> {
        let mut removed = Vec::new();
        let mut retained = Vec::with_capacity(self.attributes.len());
        for attribute in std::mem::take(&mut self.attributes) {
            if let Attribute::Code(code) = attribute {
                removed.push(code);
            } else {
                retained.push(attribute);
            }
        }
        self.attributes = retained;
        removed
    }
}

impl CodeAttribute {
    /// Creates a code attribute from typed instructions and interns its name.
    ///
    /// # Errors
    ///
    /// Returns an error if the instructions cannot be encoded or the constant
    /// pool is full.
    pub fn new(
        pool: &mut ConstantPool,
        max_stack: u16,
        max_locals: u16,
        instructions: &[Instruction],
    ) -> Result<Self> {
        Ok(Self {
            name_index: pool.intern_utf8(super::CODE_ATTRIBUTE_NAME)?,
            max_stack,
            max_locals,
            code: bytecode::encode(instructions)?,
            exception_table: Vec::new(),
            attributes: Vec::new(),
        })
    }
}

fn find_field(class: &ClassFile, name: &str, descriptor: &str) -> Result<Option<usize>> {
    for (position, field) in class.fields.iter().enumerate() {
        if field.name(&class.constant_pool)? == name
            && field.descriptor(&class.constant_pool)? == descriptor
        {
            return Ok(Some(position));
        }
    }
    Ok(None)
}

fn find_method(class: &ClassFile, name: &str, descriptor: &str) -> Result<Option<usize>> {
    for (position, method) in class.methods.iter().enumerate() {
        if method.name(&class.constant_pool)? == name
            && method.descriptor(&class.constant_pool)? == descriptor
        {
            return Ok(Some(position));
        }
    }
    Ok(None)
}

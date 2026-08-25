//! Verified Program-to-class-file emission.

mod instruction;
mod reference;

use disassembler::{
    AddressUnit, BinaryFormat, CodeAddress, FunctionBody, RawAccessFlags, Reference,
};
use program::{MethodDefinition, Module, ModuleEmitter, TypeDefinition};

use crate::analysis;
use crate::classfile::{
    ClassAccessFlags, ClassFile, CodeAttribute, ConstantPool, ExceptionHandler, FieldAccessFlags,
    JAVA_8_MAJOR_VERSION, MethodAccessFlags,
};

use self::instruction::lower_instructions;
pub use self::reference::DisplayJavaReferenceResolver;

/// Failure to resolve a shared symbolic reference into a new constant pool.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
pub struct JavaReferenceResolutionError {
    message: String,
}

impl JavaReferenceResolutionError {
    /// Creates a resolver failure with a consumer-facing explanation.
    #[must_use]
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

/// Strategy used to intern a shared reference into an emitted class's pool.
pub trait JavaReferenceResolver {
    /// Resolves one reference without trusting its stale source index.
    ///
    /// # Errors
    ///
    /// Returns an error when the shared reference lacks enough symbolic data
    /// for this strategy.
    fn resolve(
        &mut self,
        reference: &Reference,
        pool: &mut ConstantPool,
    ) -> std::result::Result<u16, JavaReferenceResolutionError>;
}

/// Class-file version policy for canonical Program emission.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct JavaEmissionOptions {
    /// Major class-file version assigned to every emitted class.
    pub major_version: u16,
}

impl Default for JavaEmissionOptions {
    fn default() -> Self {
        Self {
            major_version: JAVA_8_MAJOR_VERSION,
        }
    }
}

/// Failure to represent or validate an owned JVM module.
#[derive(Debug, thiserror::Error)]
pub enum JavaEmissionError {
    /// The selected module is not JVM class-file based.
    #[error("cannot emit {actual} module `{module}` as JVM class files")]
    WrongFormat {
        /// Module name.
        module: String,
        /// Actual native format.
        actual: BinaryFormat,
    },
    /// Raw Program access bits exceed the class-file field width.
    #[error("{kind} `{name}` has JVM access flags outside 16 bits: 0x{bits:x}")]
    AccessFlags {
        /// Definition category.
        kind: &'static str,
        /// Native definition name.
        name: String,
        /// Rejected raw bits.
        bits: u32,
    },
    /// A shared body uses the wrong address unit.
    #[error("JVM body `{class}.{method}{descriptor}` uses {actual:?} addresses")]
    AddressUnit {
        /// Owning JVM internal class name.
        class: String,
        /// Method name.
        method: String,
        /// JVM method descriptor.
        descriptor: String,
        /// Rejected shared address unit.
        actual: AddressUnit,
    },
    /// Shared body data cannot be mapped to the selected JVM opcode.
    #[error("cannot emit `{class}.{method}{descriptor}` at {address}: {message}")]
    Instruction {
        /// Owning JVM internal class name.
        class: String,
        /// Method name.
        method: String,
        /// JVM method descriptor.
        descriptor: String,
        /// Shared bytecode address.
        address: CodeAddress,
        /// Violated native operand or layout contract.
        message: String,
    },
    /// A shared reference lacks enough symbolic data for the configured resolver.
    #[error(
        "cannot resolve reference #{index} in `{class}.{method}{descriptor}` at {address}: {source}"
    )]
    Reference {
        /// Owning JVM internal class name.
        class: String,
        /// Method name.
        method: String,
        /// JVM method descriptor.
        descriptor: String,
        /// Shared bytecode address.
        address: CodeAddress,
        /// Original source-table index, retained only for diagnostics.
        index: u32,
        /// Resolver explanation.
        #[source]
        source: JavaReferenceResolutionError,
    },
    /// Native JVM construction, analysis, or validation failed.
    #[error(transparent)]
    Java(#[from] crate::Error),
}

/// Stateful canonical JVM backend using a caller-selected reference resolver.
#[derive(Debug, Clone)]
pub struct JavaEmitter<R = DisplayJavaReferenceResolver> {
    options: JavaEmissionOptions,
    resolver: R,
}

impl<R> JavaEmitter<R> {
    /// Creates an emitter with explicit version and reference policies.
    #[must_use]
    pub const fn new(options: JavaEmissionOptions, resolver: R) -> Self {
        Self { options, resolver }
    }

    /// Returns the configured class-file version policy.
    #[must_use]
    pub const fn options(&self) -> JavaEmissionOptions {
        self.options
    }

    /// Returns mutable access to the reference strategy.
    pub const fn resolver_mut(&mut self) -> &mut R {
        &mut self.resolver
    }
}

impl Default for JavaEmitter<DisplayJavaReferenceResolver> {
    fn default() -> Self {
        Self::new(JavaEmissionOptions::default(), DisplayJavaReferenceResolver)
    }
}

impl<R: JavaReferenceResolver> ModuleEmitter for JavaEmitter<R> {
    type Output = Vec<ClassFile>;
    type Error = JavaEmissionError;

    fn emit_module(&mut self, module: &Module) -> Result<Self::Output, Self::Error> {
        if module.id().format != BinaryFormat::JavaClass {
            return Err(JavaEmissionError::WrongFormat {
                module: module.id().name.clone(),
                actual: module.id().format,
            });
        }
        module
            .types()
            .map(|definition| self.emit_type(definition))
            .collect()
    }
}

impl<R: JavaReferenceResolver> JavaEmitter<R> {
    fn emit_type(&mut self, definition: &TypeDefinition) -> Result<ClassFile, JavaEmissionError> {
        let name = &definition.id().name;
        let superclass = definition.superclass().map(|id| id.name.as_str());
        let mut class = ClassFile::new(
            self.options.major_version,
            name,
            superclass,
            ClassAccessFlags::from_bits_retain(access_bits(
                "class",
                name,
                definition.access_flags(),
            )?),
        )?;
        for interface in definition.interfaces() {
            class.add_interface(&interface.name)?;
        }
        for field in definition.fields() {
            class.add_field(
                FieldAccessFlags::from_bits_retain(access_bits(
                    "field",
                    &field.id().name,
                    field.access_flags(),
                )?),
                &field.id().name,
                &field.id().signature,
            )?;
        }
        for method in definition.methods() {
            self.emit_method(&mut class, name, method)?;
        }
        class.validate()?;
        Ok(class)
    }

    fn emit_method(
        &mut self,
        class: &mut ClassFile,
        owner: &str,
        definition: &MethodDefinition,
    ) -> Result<(), JavaEmissionError> {
        let id = definition.id();
        let flags = MethodAccessFlags::from_bits_retain(access_bits(
            "method",
            &id.name,
            definition.access_flags(),
        )?);
        let position = class.add_method(flags, &id.name, &id.signature)?;
        if let Some(body) = definition.body() {
            let mut code = emit_body(
                body,
                owner,
                &id.name,
                &id.signature,
                flags,
                &mut class.constant_pool,
                &mut self.resolver,
            )?;
            let analysis = analysis::analyze_code(
                &class.constant_pool,
                owner,
                &id.name,
                &id.signature,
                flags,
                &code,
            )?;
            analysis.apply_to_code(&mut class.constant_pool, &mut code)?;
            class.methods[position].set_code(code);
        }
        Ok(())
    }
}

/// Emits and validates every class in one JVM Program module.
///
/// This convenience path uses [`DisplayJavaReferenceResolver`]. Use
/// [`JavaEmitter::new`] when another symbolic reference policy is required.
///
/// # Errors
///
/// Returns an error for a non-JVM module, unrepresentable shared operand, or
/// invalid emitted class.
pub fn emit_module(module: &Module) -> Result<Vec<ClassFile>, JavaEmissionError> {
    let mut emitter = JavaEmitter::default();
    emitter.emit_module(module)
}

fn emit_body<R: JavaReferenceResolver>(
    body: &FunctionBody,
    owner: &str,
    method: &str,
    descriptor: &str,
    _flags: MethodAccessFlags,
    pool: &mut ConstantPool,
    resolver: &mut R,
) -> Result<CodeAttribute, JavaEmissionError> {
    if body.address_unit != AddressUnit::Byte {
        return Err(JavaEmissionError::AddressUnit {
            class: owner.to_owned(),
            method: method.to_owned(),
            descriptor: descriptor.to_owned(),
            actual: body.address_unit,
        });
    }
    let instructions = lower_instructions(body, owner, method, descriptor, pool, resolver)?;
    let mut code = CodeAttribute::new(pool, 0, 0, &instructions)?;
    code.exception_table = body
        .exception_handlers
        .iter()
        .map(|handler| {
            Ok(ExceptionHandler {
                start_pc: u16_address(owner, method, descriptor, handler.protected.start)?,
                end_pc: u16_address(owner, method, descriptor, handler.protected.end)?,
                handler_pc: u16_address(owner, method, descriptor, handler.handler)?,
                catch_type: match &handler.catch {
                    disassembler::CatchType::Any => 0,
                    disassembler::CatchType::Type(name) => pool.intern_class(name)?,
                },
            })
        })
        .collect::<Result<Vec<_>, JavaEmissionError>>()?;
    Ok(code)
}

fn access_bits(
    kind: &'static str,
    name: &str,
    flags: RawAccessFlags,
) -> Result<u16, JavaEmissionError> {
    u16::try_from(flags.bits()).map_err(|_| JavaEmissionError::AccessFlags {
        kind,
        name: name.to_owned(),
        bits: flags.bits(),
    })
}

fn u16_address(
    class: &str,
    method: &str,
    descriptor: &str,
    address: CodeAddress,
) -> Result<u16, JavaEmissionError> {
    u16::try_from(address.get()).map_err(|_| JavaEmissionError::Instruction {
        class: class.to_owned(),
        method: method.to_owned(),
        descriptor: descriptor.to_owned(),
        address,
        message: "bytecode address exceeds the class-file u16 metadata range".to_owned(),
    })
}

#[cfg(test)]
mod tests;

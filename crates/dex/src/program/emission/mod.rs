//! Verified Program-to-DEX emission.

mod descriptor;
mod instruction;
mod reference;

use std::collections::HashMap;

use disassembler::{
    AddressRange, AddressUnit, BinaryFormat, CatchType, CodeAddress, FunctionBody, Operand,
    Reference,
};
use program::{MethodDefinition, Module, ModuleEmitter, TypeDefinition};

use self::descriptor::{field_type_valid, method_parts, register_words};
use self::instruction::lower_instructions;
pub use self::reference::SymbolicDexReferenceResolver;
use crate::analysis::instruction_semantics;
use crate::file::{
    AccessFlags, AnnotationDirectory, CatchHandler, ClassData, ClassDefinition, CodeItem,
    DexBuilder, DexContainer, DexFile, DexIndices, DexVersion, EncodedField, EncodedMethod,
    FieldHandle, MethodIdHandle, PrototypeHandle, StringHandle, TryBlock, TypeHandle,
};
use crate::instruction::{IndexKind, Instruction, InstructionData, Opcode, Operands};

/// Failure to resolve a shared symbolic reference into new DEX tables.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
pub struct DexReferenceResolutionError {
    message: String,
}

impl DexReferenceResolutionError {
    /// Creates a resolver failure with a consumer-facing explanation.
    #[must_use]
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

/// Stable symbolic identifier produced while constructing DEX tables.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DexReferenceHandle {
    /// String identifier.
    String(StringHandle),
    /// Type identifier.
    Type(TypeHandle),
    /// Field identifier.
    Field(FieldHandle),
    /// Method identifier.
    Method(MethodIdHandle),
    /// Method-prototype identifier.
    Prototype(PrototypeHandle),
}

impl DexReferenceHandle {
    fn index_for(self, expected: Option<IndexKind>, indices: &DexIndices) -> Result<u32, String> {
        let value = match (expected, self) {
            (Some(IndexKind::String), Self::String(handle)) => {
                indices.string(handle).map(crate::file::StringIndex::get)
            }
            (Some(IndexKind::Type), Self::Type(handle)) => {
                indices.type_index(handle).map(crate::file::TypeIndex::get)
            }
            (Some(IndexKind::Field), Self::Field(handle)) => {
                indices.field(handle).map(crate::file::FieldIndex::get)
            }
            (Some(IndexKind::Method), Self::Method(handle)) => {
                indices.method(handle).map(crate::file::MethodIndex::get)
            }
            (Some(IndexKind::Prototype), Self::Prototype(handle)) => indices
                .prototype(handle)
                .map(crate::file::PrototypeIndex::get),
            (Some(IndexKind::CallSite | IndexKind::MethodHandle) | None, _) => None,
            _ => {
                return Err(
                    "symbolic reference kind does not match the opcode index table".to_owned(),
                );
            }
        };
        value.ok_or_else(|| {
            "symbolic reference handle is unavailable in the built tables".to_owned()
        })
    }
}

/// Strategy used to intern shared references into an emitted DEX file.
pub trait DexReferenceResolver {
    /// Interns one reference without trusting its stale source index.
    ///
    /// # Errors
    ///
    /// Returns an error when the shared reference lacks enough symbolic data
    /// for this strategy.
    fn intern(
        &mut self,
        reference: &Reference,
        builder: &mut DexBuilder,
    ) -> Result<DexReferenceHandle, DexReferenceResolutionError>;
}

/// Version policy for canonical Program emission.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DexEmissionOptions {
    /// Standard DEX version assigned to the emitted artifact.
    pub version: DexVersion,
}

impl Default for DexEmissionOptions {
    fn default() -> Self {
        Self {
            version: DexVersion::V040,
        }
    }
}

/// Failure to represent or validate an owned DEX module.
#[derive(Debug, thiserror::Error)]
pub enum DexEmissionError {
    /// The selected module is not DEX based.
    #[error("cannot emit {actual} module `{module}` as DEX")]
    WrongFormat {
        /// Module name.
        module: String,
        /// Actual native format.
        actual: BinaryFormat,
    },
    /// A related Program type uses another native format.
    #[error("DEX type `{owner}` has {relationship} `{name}` qualified as {actual}")]
    RelatedTypeFormat {
        /// Owning DEX descriptor.
        owner: String,
        /// Relationship being encoded.
        relationship: &'static str,
        /// Referenced type name.
        name: String,
        /// Rejected native format.
        actual: BinaryFormat,
    },
    /// A declaration has an invalid DEX/JVM descriptor.
    #[error("invalid {kind} descriptor `{descriptor}` on `{owner}.{name}`: {message}")]
    Descriptor {
        /// Declaration category.
        kind: &'static str,
        /// Owning type descriptor.
        owner: String,
        /// Member name.
        name: String,
        /// Rejected descriptor.
        descriptor: String,
        /// Validation explanation.
        message: String,
    },
    /// A shared body uses the wrong address unit.
    #[error("DEX body `{class}->{method}{descriptor}` uses {actual:?} addresses")]
    AddressUnit {
        /// Owning type descriptor.
        class: String,
        /// Method name.
        method: String,
        /// Method descriptor.
        descriptor: String,
        /// Rejected shared address unit.
        actual: AddressUnit,
    },
    /// Retained or inferred frame resources are inconsistent.
    #[error("invalid register resources for `{class}->{method}{descriptor}`: {message}")]
    RegisterResources {
        /// Owning type descriptor.
        class: String,
        /// Method name.
        method: String,
        /// Method descriptor.
        descriptor: String,
        /// Validation explanation.
        message: String,
    },
    /// Shared body data cannot be mapped to the selected DEX opcode.
    #[error("cannot emit `{class}->{method}{descriptor}` at {address}: {message}")]
    Instruction {
        /// Owning type descriptor.
        class: String,
        /// Method name.
        method: String,
        /// Method descriptor.
        descriptor: String,
        /// Shared code-unit address.
        address: CodeAddress,
        /// Violated native operand or layout contract.
        message: String,
    },
    /// A shared instruction reference lacks reconstructable symbolic data.
    #[error(
        "cannot resolve reference #{index} in `{class}->{method}{descriptor}` at {address}: {source}"
    )]
    Reference {
        /// Owning type descriptor.
        class: String,
        /// Method name.
        method: String,
        /// Method descriptor.
        descriptor: String,
        /// Shared code-unit address.
        address: CodeAddress,
        /// Original table index, retained only for diagnostics.
        index: u32,
        /// Resolver explanation.
        #[source]
        source: DexReferenceResolutionError,
    },
    /// Program-owned class dependencies contain a cycle.
    #[error("DEX class hierarchy contains a cycle through `{class}`")]
    HierarchyCycle {
        /// Descriptor participating in the cycle.
        class: String,
    },
    /// Native DEX construction, analysis, assembly, or validation failed.
    #[error(transparent)]
    Dex(#[from] crate::Error),
}

/// Stateful canonical DEX backend using a caller-selected reference resolver.
#[derive(Debug, Clone)]
pub struct DexEmitter<R = SymbolicDexReferenceResolver> {
    options: DexEmissionOptions,
    resolver: R,
}

impl<R> DexEmitter<R> {
    /// Creates an emitter with explicit version and reference policies.
    #[must_use]
    pub const fn new(options: DexEmissionOptions, resolver: R) -> Self {
        Self { options, resolver }
    }

    /// Returns the configured DEX version policy.
    #[must_use]
    pub const fn options(&self) -> DexEmissionOptions {
        self.options
    }

    /// Returns mutable access to the symbolic-reference strategy.
    pub const fn resolver_mut(&mut self) -> &mut R {
        &mut self.resolver
    }
}

impl Default for DexEmitter<SymbolicDexReferenceResolver> {
    fn default() -> Self {
        Self::new(DexEmissionOptions::default(), SymbolicDexReferenceResolver)
    }
}

impl<R: DexReferenceResolver> ModuleEmitter for DexEmitter<R> {
    type Output = DexFile;
    type Error = DexEmissionError;

    fn emit_module(&mut self, module: &Module) -> Result<Self::Output, Self::Error> {
        if module.id().format != BinaryFormat::Dex {
            return Err(DexEmissionError::WrongFormat {
                module: module.id().name.clone(),
                actual: module.id().format,
            });
        }
        let mut builder = DexBuilder::new(self.options.version);
        let mut references = HashMap::new();
        let mut exception_types = HashMap::new();
        let plans = module
            .types()
            .map(|definition| {
                plan_type(
                    definition,
                    &mut builder,
                    &mut self.resolver,
                    &mut references,
                    &mut exception_types,
                )
            })
            .collect::<Result<Vec<_>, _>>()?;
        let order = dependency_order(&plans)?;
        let built = builder.build()?;
        let mut file = built.file;
        for position in order {
            file.push_class(emit_type(
                &plans[position],
                &built.indices,
                &references,
                &exception_types,
            )?)?;
        }
        verify_file(&file)?;
        Ok(file)
    }
}

/// Emits and validates one canonical DEX file from a Program module.
///
/// This convenience path uses [`SymbolicDexReferenceResolver`]. Use
/// [`DexEmitter::new`] for another symbolic-reference policy.
///
/// # Errors
///
/// Returns an error for a non-DEX module, unrepresentable shared operand, or
/// invalid emitted DEX model.
pub fn emit_module(module: &Module) -> Result<DexFile, DexEmissionError> {
    let mut emitter = DexEmitter::default();
    emitter.emit_module(module)
}

struct TypePlan<'a> {
    definition: &'a TypeDefinition,
    class: TypeHandle,
    superclass: Option<TypeHandle>,
    interfaces: Vec<TypeHandle>,
    fields: Vec<FieldPlan<'a>>,
    methods: Vec<MethodPlan<'a>>,
}

struct FieldPlan<'a> {
    definition: &'a program::FieldDefinition,
    handle: FieldHandle,
}

struct MethodPlan<'a> {
    definition: &'a MethodDefinition,
    handle: MethodIdHandle,
}

fn plan_type<'a, R: DexReferenceResolver>(
    definition: &'a TypeDefinition,
    builder: &mut DexBuilder,
    resolver: &mut R,
    references: &mut HashMap<Reference, DexReferenceHandle>,
    exception_types: &mut HashMap<String, TypeHandle>,
) -> Result<TypePlan<'a>, DexEmissionError> {
    let owner = &definition.id().name;
    let class = builder.intern_type(owner)?;
    let superclass = definition
        .superclass()
        .map(|dependency| related_type(owner, "superclass", dependency, builder))
        .transpose()?;
    let interfaces = definition
        .interfaces()
        .iter()
        .map(|dependency| related_type(owner, "interface", dependency, builder))
        .collect::<Result<Vec<_>, _>>()?;

    let mut fields = Vec::new();
    for field in definition.fields() {
        if !field_type_valid(&field.id().signature) {
            return Err(descriptor_error(
                "field",
                owner,
                &field.id().name,
                &field.id().signature,
                "expected one non-void type",
            ));
        }
        fields.push(FieldPlan {
            definition: field,
            handle: builder.intern_field_named(owner, &field.id().name, &field.id().signature)?,
        });
    }

    let mut methods = Vec::new();
    for method in definition.methods() {
        let (parameters, return_type) =
            method_parts(&method.id().signature).map_err(|message| {
                descriptor_error(
                    "method",
                    owner,
                    &method.id().name,
                    &method.id().signature,
                    message,
                )
            })?;
        let return_type = builder.intern_type(&return_type)?;
        let parameters = parameters
            .iter()
            .map(|parameter| builder.intern_type(parameter))
            .collect::<Result<Vec<_>, _>>()?;
        let prototype = builder.intern_prototype(return_type, parameters)?;
        let name = builder.intern_string(&method.id().name)?;
        let handle = builder.intern_method(class, name, prototype)?;
        if let Some(body) = method.body() {
            intern_body_references(owner, method, body, builder, resolver, references)?;
            for handler in &body.exception_handlers {
                if let CatchType::Type(descriptor) = &handler.catch
                    && !exception_types.contains_key(descriptor)
                {
                    let handle = builder.intern_type(descriptor)?;
                    exception_types.insert(descriptor.clone(), handle);
                }
            }
        }
        methods.push(MethodPlan {
            definition: method,
            handle,
        });
    }
    Ok(TypePlan {
        definition,
        class,
        superclass,
        interfaces,
        fields,
        methods,
    })
}

fn related_type(
    owner: &str,
    relationship: &'static str,
    dependency: &program::TypeId,
    builder: &mut DexBuilder,
) -> Result<TypeHandle, DexEmissionError> {
    if dependency.format != BinaryFormat::Dex {
        return Err(DexEmissionError::RelatedTypeFormat {
            owner: owner.to_owned(),
            relationship,
            name: dependency.name.clone(),
            actual: dependency.format,
        });
    }
    Ok(builder.intern_type(&dependency.name)?)
}

fn intern_body_references<R: DexReferenceResolver>(
    owner: &str,
    method: &MethodDefinition,
    body: &FunctionBody,
    builder: &mut DexBuilder,
    resolver: &mut R,
    references: &mut HashMap<Reference, DexReferenceHandle>,
) -> Result<(), DexEmissionError> {
    for instruction in &body.instructions {
        for operand in &instruction.operands {
            let Operand::Reference(reference) = operand else {
                continue;
            };
            if references.contains_key(reference) {
                continue;
            }
            let handle = resolver.intern(reference, builder).map_err(|source| {
                DexEmissionError::Reference {
                    class: owner.to_owned(),
                    method: method.id().name.clone(),
                    descriptor: method.id().signature.clone(),
                    address: instruction.address,
                    index: reference.index,
                    source,
                }
            })?;
            references.insert(reference.clone(), handle);
        }
    }
    Ok(())
}

fn dependency_order(plans: &[TypePlan<'_>]) -> Result<Vec<usize>, DexEmissionError> {
    let positions = plans
        .iter()
        .enumerate()
        .map(|(position, plan)| (plan.definition.id().name.as_str(), position))
        .collect::<HashMap<_, _>>();
    let mut states = vec![0_u8; plans.len()];
    let mut output = Vec::with_capacity(plans.len());
    for position in 0..plans.len() {
        visit_type(position, plans, &positions, &mut states, &mut output)?;
    }
    Ok(output)
}

fn visit_type(
    position: usize,
    plans: &[TypePlan<'_>],
    positions: &HashMap<&str, usize>,
    states: &mut [u8],
    output: &mut Vec<usize>,
) -> Result<(), DexEmissionError> {
    match states[position] {
        2 => return Ok(()),
        1 => {
            return Err(DexEmissionError::HierarchyCycle {
                class: plans[position].definition.id().name.clone(),
            });
        }
        _ => states[position] = 1,
    }
    let definition = plans[position].definition;
    for dependency in definition
        .superclass()
        .into_iter()
        .chain(definition.interfaces().iter())
    {
        if let Some(&dependency_position) = positions.get(dependency.name.as_str()) {
            visit_type(dependency_position, plans, positions, states, output)?;
        }
    }
    states[position] = 2;
    output.push(position);
    Ok(())
}

fn emit_type(
    plan: &TypePlan<'_>,
    indices: &DexIndices,
    references: &HashMap<Reference, DexReferenceHandle>,
    exception_types: &HashMap<String, TypeHandle>,
) -> Result<ClassDefinition, DexEmissionError> {
    let owner = &plan.definition.id().name;
    let class = native_type(plan.class, indices)?;
    let mut static_fields = Vec::new();
    let mut instance_fields = Vec::new();
    for field in &plan.fields {
        let encoded = EncodedField {
            field: indices.field(field.handle).ok_or_else(missing_handle)?,
            access_flags: AccessFlags::from_bits_retain(field.definition.access_flags().bits()),
        };
        if encoded.access_flags.contains(AccessFlags::STATIC) {
            static_fields.push(encoded);
        } else {
            instance_fields.push(encoded);
        }
    }
    static_fields.sort_by_key(|field| field.field);
    instance_fields.sort_by_key(|field| field.field);

    let mut direct_methods = Vec::new();
    let mut virtual_methods = Vec::new();
    for method in &plan.methods {
        let flags = AccessFlags::from_bits_retain(method.definition.access_flags().bits());
        let encoded = EncodedMethod {
            method: indices.method(method.handle).ok_or_else(missing_handle)?,
            access_flags: flags,
            code: method
                .definition
                .body()
                .map(|body| {
                    emit_body(
                        body,
                        owner,
                        &method.definition.id().name,
                        &method.definition.id().signature,
                        flags,
                        references,
                        exception_types,
                        indices,
                    )
                })
                .transpose()?,
        };
        if is_direct(method.definition, flags) {
            direct_methods.push(encoded);
        } else {
            virtual_methods.push(encoded);
        }
    }
    direct_methods.sort_by_key(|method| method.method);
    virtual_methods.sort_by_key(|method| method.method);
    let class_data = (!plan.fields.is_empty() || !plan.methods.is_empty()).then_some(ClassData {
        static_fields,
        instance_fields,
        direct_methods,
        virtual_methods,
        data_offset: 0,
    });
    Ok(ClassDefinition {
        class,
        access_flags: AccessFlags::from_bits_retain(plan.definition.access_flags().bits()),
        superclass: plan
            .superclass
            .map(|handle| native_type(handle, indices))
            .transpose()?,
        interfaces: plan
            .interfaces
            .iter()
            .map(|&handle| native_type(handle, indices))
            .collect::<Result<Vec<_>, _>>()?,
        source_file: None,
        annotations: AnnotationDirectory::default(),
        class_data,
        static_values: Vec::new(),
        definition_index: 0,
    })
}

#[allow(clippy::too_many_arguments)]
fn emit_body(
    body: &FunctionBody,
    owner: &str,
    method: &str,
    descriptor: &str,
    flags: AccessFlags,
    references: &HashMap<Reference, DexReferenceHandle>,
    exception_types: &HashMap<String, TypeHandle>,
    indices: &DexIndices,
) -> Result<CodeItem, DexEmissionError> {
    if body.address_unit != AddressUnit::CodeUnit16 {
        return Err(DexEmissionError::AddressUnit {
            class: owner.to_owned(),
            method: method.to_owned(),
            descriptor: descriptor.to_owned(),
            actual: body.address_unit,
        });
    }
    let instructions = lower_instructions(
        &body.instructions,
        references,
        indices,
        owner,
        method,
        descriptor,
    )?;
    let expected_incoming = incoming_words(descriptor, !flags.contains(AccessFlags::STATIC))
        .map_err(|message| descriptor_error("method", owner, method, descriptor, message))?;
    let inferred_registers = inferred_registers(&instructions)?;
    let inferred_outgoing = inferred_outgoing(&instructions);
    let (registers_size, ins_size, outs_size) = match body.register_resources {
        Some(resources) => {
            if resources.incoming != expected_incoming {
                return Err(resource_error(
                    owner,
                    method,
                    descriptor,
                    format!(
                        "retained incoming width {} differs from descriptor width {expected_incoming}",
                        resources.incoming
                    ),
                ));
            }
            if resources.registers < inferred_registers.max(resources.incoming) {
                return Err(resource_error(
                    owner,
                    method,
                    descriptor,
                    "retained register count does not cover every operand",
                ));
            }
            if resources.outgoing < inferred_outgoing {
                return Err(resource_error(
                    owner,
                    method,
                    descriptor,
                    "retained outgoing width does not cover every invocation",
                ));
            }
            (resources.registers, resources.incoming, resources.outgoing)
        }
        None => (
            inferred_registers.max(expected_incoming),
            expected_incoming,
            inferred_outgoing,
        ),
    };
    Ok(CodeItem {
        registers_size,
        ins_size,
        outs_size,
        instructions,
        tries: emit_tries(body, exception_types, indices, owner, method, descriptor)?,
        debug_info: None,
        data_offset: 0,
    })
}

fn emit_tries(
    body: &FunctionBody,
    exception_types: &HashMap<String, TypeHandle>,
    indices: &DexIndices,
    owner: &str,
    method: &str,
    descriptor: &str,
) -> Result<Vec<TryBlock>, DexEmissionError> {
    let mut groups: Vec<(AddressRange, Vec<CatchHandler>)> = Vec::new();
    for handler in &body.exception_handlers {
        let target = u32_address(owner, method, descriptor, handler.handler)?;
        let exception_type = match &handler.catch {
            CatchType::Any => None,
            CatchType::Type(name) => Some(
                exception_types
                    .get(name)
                    .and_then(|&handle| indices.type_index(handle))
                    .ok_or_else(missing_handle)?,
            ),
        };
        let catch = CatchHandler {
            exception_type,
            address: target,
        };
        if let Some((_, handlers)) = groups
            .iter_mut()
            .find(|(range, _)| *range == handler.protected)
        {
            handlers.push(catch);
        } else {
            groups.push((handler.protected, vec![catch]));
        }
    }
    groups.sort_by_key(|(range, _)| range.start);
    groups
        .into_iter()
        .map(|(range, handlers)| {
            let start = u32_address(owner, method, descriptor, range.start)?;
            let end = u32_address(owner, method, descriptor, range.end)?;
            let count = end
                .checked_sub(start)
                .and_then(|value| u16::try_from(value).ok())
                .ok_or_else(|| {
                    instruction_error(
                        owner,
                        method,
                        descriptor,
                        range.start,
                        "exception range is empty, reversed, or exceeds u16 code units",
                    )
                })?;
            Ok(TryBlock {
                start_address: start,
                instruction_count: count,
                handlers,
            })
        })
        .collect()
}

fn inferred_registers(instructions: &[Instruction]) -> Result<u16, DexEmissionError> {
    let mut count = 0_u16;
    for instruction in instructions {
        let semantics = instruction_semantics(instruction)?;
        for operand in semantics.reads.iter().chain(&semantics.writes) {
            let end = operand
                .register
                .checked_add(u16::from(operand.register_words()))
                .ok_or_else(|| {
                    crate::Error::invalid_instruction(
                        instruction.offset(),
                        "register span overflows u16",
                    )
                })?;
            count = count.max(end);
        }
    }
    Ok(count)
}

fn inferred_outgoing(instructions: &[Instruction]) -> u16 {
    instructions
        .iter()
        .filter_map(|instruction| {
            let InstructionData::Operation { opcode, operands } = instruction.data() else {
                return None;
            };
            is_invoke(*opcode).then(|| match operands {
                Operands::RegisterListIndex { registers, .. } => {
                    u16::try_from(registers.len()).unwrap_or(u16::MAX)
                }
                Operands::RegisterRangeIndex { count, .. } => u16::from(*count),
                _ => 0,
            })
        })
        .max()
        .unwrap_or(0)
}

fn is_invoke(opcode: Opcode) -> bool {
    matches!(
        opcode,
        Opcode::InvokeVirtual
            | Opcode::InvokeSuper
            | Opcode::InvokeDirect
            | Opcode::InvokeStatic
            | Opcode::InvokeInterface
            | Opcode::InvokeVirtualRange
            | Opcode::InvokeSuperRange
            | Opcode::InvokeDirectRange
            | Opcode::InvokeStaticRange
            | Opcode::InvokeInterfaceRange
            | Opcode::InvokePolymorphic
            | Opcode::InvokePolymorphicRange
            | Opcode::InvokeCustom
            | Opcode::InvokeCustomRange
    )
}

fn is_direct(method: &MethodDefinition, flags: AccessFlags) -> bool {
    flags.contains(AccessFlags::STATIC)
        || flags.contains(AccessFlags::PRIVATE)
        || flags.contains(AccessFlags::CONSTRUCTOR)
        || matches!(method.id().name.as_str(), "<init>" | "<clinit>")
}

fn incoming_words(descriptor: &str, receiver: bool) -> Result<u16, String> {
    let (parameters, _) = method_parts(descriptor)?;
    parameters
        .into_iter()
        .try_fold(u16::from(receiver), |sum, parameter| {
            sum.checked_add(register_words(&parameter))
                .ok_or_else(|| "incoming register width exceeds u16".to_owned())
        })
}

fn native_type(
    handle: TypeHandle,
    indices: &DexIndices,
) -> Result<crate::file::TypeIndex, DexEmissionError> {
    indices.type_index(handle).ok_or_else(missing_handle)
}

fn verify_file(file: &DexFile) -> Result<(), DexEmissionError> {
    if file.version() == DexVersion::V041 {
        let mut container = DexContainer::new();
        container.push(file.clone())?;
        let bytes = container.to_bytes()?;
        DexContainer::parse(&bytes)?;
    } else {
        let bytes = file.to_bytes()?;
        DexFile::parse(&bytes)?;
    }
    Ok(())
}

fn descriptor_error(
    kind: &'static str,
    owner: &str,
    name: &str,
    descriptor: &str,
    message: impl Into<String>,
) -> DexEmissionError {
    DexEmissionError::Descriptor {
        kind,
        owner: owner.to_owned(),
        name: name.to_owned(),
        descriptor: descriptor.to_owned(),
        message: message.into(),
    }
}

fn resource_error(
    class: &str,
    method: &str,
    descriptor: &str,
    message: impl Into<String>,
) -> DexEmissionError {
    DexEmissionError::RegisterResources {
        class: class.to_owned(),
        method: method.to_owned(),
        descriptor: descriptor.to_owned(),
        message: message.into(),
    }
}

fn instruction_error(
    class: &str,
    method: &str,
    descriptor: &str,
    address: CodeAddress,
    message: impl Into<String>,
) -> DexEmissionError {
    DexEmissionError::Instruction {
        class: class.to_owned(),
        method: method.to_owned(),
        descriptor: descriptor.to_owned(),
        address,
        message: message.into(),
    }
}

fn u32_address(
    class: &str,
    method: &str,
    descriptor: &str,
    address: CodeAddress,
) -> Result<u32, DexEmissionError> {
    u32::try_from(address.get()).map_err(|_| {
        instruction_error(
            class,
            method,
            descriptor,
            address,
            "code-unit address exceeds u32",
        )
    })
}

fn missing_handle() -> DexEmissionError {
    DexEmissionError::Dex(crate::Error::invalid_assembly(
        "emission plan contains a foreign symbolic handle",
    ))
}

#[cfg(test)]
mod tests;

//! Checked canonical lowering from shared MLIL to Dalvik LLIL.

mod arrays;
mod instruction;
mod intrinsic;
mod layout;
mod opcodes;
mod registers;
mod target;

use std::collections::{BTreeMap, BTreeSet};

use ::mlil::{EdgeRole, EntityId, Function, InstructionId, Operation};
use disassembler::cfglib::BlockId;
use disassembler::{
    AddressRange, AddressUnit, BinaryFormat, CodeAddress, FunctionCoordinate, ReferenceKind,
    SourceMap,
};

use crate::DexReferenceResolutionError;
use crate::file::{
    CallSiteIndex, CatchHandler, CodeItem, DexFile, FieldIndex, MethodHandleIndex, MethodIndex,
    PrototypeIndex, StringIndex, TryBlock, TypeIndex,
};
use crate::instruction::IndexKind;
use crate::llil;

use self::instruction::{Emission, emit_instruction};
pub use self::intrinsic::{
    DexIntrinsicInstruction, DexIntrinsicLoweringError, DexIntrinsicRequest,
    DexMlilIntrinsicLowerer, RejectDexIntrinsics,
};
use self::layout::Planner;
use self::registers::RegisterAllocation;
use super::{Error, Result};

pub use self::target::TargetDexReferenceResolver;

/// Resolves MLIL symbols into final indices of the target DEX identifier tables.
pub trait DexMlilReferenceResolver {
    /// Resolves one indexed instruction operand for its required DEX table.
    ///
    /// # Errors
    ///
    /// Returns an error when the reference is absent or belongs to another table.
    fn resolve(
        &mut self,
        file: &DexFile,
        reference: &disassembler::Reference,
        expected: IndexKind,
    ) -> std::result::Result<u32, DexReferenceResolutionError>;

    /// Resolves one exception descriptor into a target type index.
    ///
    /// # Errors
    ///
    /// Returns an error when the target file has no matching type identifier.
    fn resolve_type(
        &mut self,
        file: &DexFile,
        descriptor: &str,
    ) -> std::result::Result<TypeIndex, DexReferenceResolutionError>;

    /// Resolves one effective call descriptor into a prototype index.
    ///
    /// # Errors
    ///
    /// Returns an error when no target prototype has the descriptor.
    fn resolve_prototype(
        &mut self,
        file: &DexFile,
        descriptor: &str,
    ) -> std::result::Result<PrototypeIndex, DexReferenceResolutionError>;
}

/// Resolver that retains checked indices into the MLIL function's source DEX.
#[derive(Debug, Clone, Copy, Default)]
pub struct SourceDexReferenceResolver;

impl DexMlilReferenceResolver for SourceDexReferenceResolver {
    fn resolve(
        &mut self,
        file: &DexFile,
        reference: &disassembler::Reference,
        expected: IndexKind,
    ) -> std::result::Result<u32, DexReferenceResolutionError> {
        if !source_reference_matches(reference.kind, expected) {
            return Err(DexReferenceResolutionError::new(format!(
                "source reference kind {:?} is incompatible with {expected:?}",
                reference.kind
            )));
        }
        let index = reference.index;
        let result = match expected {
            IndexKind::String => file.resolve_string(StringIndex::new(index)).map(drop),
            IndexKind::Type => file.resolve_type(TypeIndex::new(index)).map(drop),
            IndexKind::Field => file.resolve_field_id(FieldIndex::new(index)).map(drop),
            IndexKind::Method => file.resolve_method_id(MethodIndex::new(index)).map(drop),
            IndexKind::Prototype => file.resolve_prototype(PrototypeIndex::new(index)).map(drop),
            IndexKind::CallSite => file.resolve_call_site(CallSiteIndex::new(index)).map(drop),
            IndexKind::MethodHandle => file
                .resolve_method_handle_id(MethodHandleIndex::new(index))
                .map(drop),
        };
        result.map_err(|error| {
            DexReferenceResolutionError::new(format!(
                "source {expected:?} index #{index} is invalid: {error}"
            ))
        })?;
        Ok(index)
    }

    fn resolve_type(
        &mut self,
        file: &DexFile,
        descriptor: &str,
    ) -> std::result::Result<TypeIndex, DexReferenceResolutionError> {
        for (position, _) in file.types().iter().enumerate() {
            let index = TypeIndex::new(u32::try_from(position).map_err(|_| {
                DexReferenceResolutionError::new("DEX type table position exceeds u32")
            })?);
            if file.type_descriptor(index).map_err(|error| {
                DexReferenceResolutionError::new(format!("invalid target type table: {error}"))
            })? == descriptor
            {
                return Ok(index);
            }
        }
        Err(DexReferenceResolutionError::new(format!(
            "target DEX has no type identifier for `{descriptor}`"
        )))
    }

    fn resolve_prototype(
        &mut self,
        file: &DexFile,
        descriptor: &str,
    ) -> std::result::Result<PrototypeIndex, DexReferenceResolutionError> {
        for (position, _) in file.prototypes().iter().enumerate() {
            let index = PrototypeIndex::new(u32::try_from(position).map_err(|_| {
                DexReferenceResolutionError::new("DEX prototype table position exceeds u32")
            })?);
            if file.prototype_descriptor(index).map_err(|error| {
                DexReferenceResolutionError::new(format!("invalid target prototype table: {error}"))
            })? == descriptor
            {
                return Ok(index);
            }
        }
        Err(DexReferenceResolutionError::new(format!(
            "target DEX has no prototype identifier for `{descriptor}`"
        )))
    }
}

const fn source_reference_matches(kind: ReferenceKind, expected: IndexKind) -> bool {
    match expected {
        IndexKind::String => matches!(kind, ReferenceKind::String),
        IndexKind::Type => matches!(kind, ReferenceKind::Type),
        IndexKind::Field => matches!(kind, ReferenceKind::Field),
        IndexKind::Method => {
            matches!(kind, ReferenceKind::Method | ReferenceKind::InterfaceMethod)
        }
        IndexKind::Prototype => matches!(kind, ReferenceKind::MethodPrototype),
        IndexKind::CallSite => matches!(kind, ReferenceKind::DynamicCallSite),
        IndexKind::MethodHandle => matches!(kind, ReferenceKind::MethodHandle),
    }
}

/// Canonically generated Dalvik LLIL plus original-native to generated-native provenance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoweredBody {
    /// Verified Dalvik LLIL body.
    pub body: llil::Body,
    /// Many-to-many correspondence retained through MLIL instruction identities.
    pub source_map: SourceMap,
}

/// Lowers verified MLIL into Dalvik LLIL against target identifier tables.
///
/// The generated body has fresh register allocation, branch and payload layout,
/// exception ranges, and no stale debug offsets.
///
/// # Errors
///
/// Returns an error for invalid MLIL, missing target identifiers, or semantic
/// constructs without a canonical Dalvik encoding.
pub fn lower_body(file: &DexFile, function: &Function) -> Result<LoweredBody> {
    lower_body_with_resolver(file, function, &mut TargetDexReferenceResolver)
}

/// Lowers DEX-origin MLIL while retaining checked source identifier indices.
///
/// This is the escape hatch for recursive call-site and method-handle metadata
/// that one MLIL reference cannot reconstruct. Prefer [`lower_body`] for
/// retargetable symbolic lookup.
///
/// # Errors
///
/// Returns an error for non-DEX provenance, invalid MLIL, stale source indices,
/// or semantic constructs without a canonical Dalvik encoding.
pub fn lower_body_from_source(file: &DexFile, function: &Function) -> Result<LoweredBody> {
    if function.source().format != BinaryFormat::Dex {
        return Err(Error::WrongFormat {
            actual: function.source().format,
        });
    }
    lower_body_with_resolver(file, function, &mut SourceDexReferenceResolver)
}

/// Lowers verified MLIL using an explicit target-table resolver.
///
/// # Errors
///
/// Returns the same target failures as [`lower_body`] plus resolver failures.
pub fn lower_body_with_resolver<R: DexMlilReferenceResolver>(
    file: &DexFile,
    function: &Function,
    resolver: &mut R,
) -> Result<LoweredBody> {
    lower_body_with_resolver_and_intrinsics(file, function, resolver, &mut RejectDexIntrinsics)
}

/// Lowers verified MLIL with explicit target reference and intrinsic policies.
///
/// # Errors
///
/// Returns the same failures as [`lower_body_with_resolver`] plus a contextual
/// failure from `intrinsics` when it declines or mis-encodes an operation.
pub fn lower_body_with_resolver_and_intrinsics<
    R: DexMlilReferenceResolver,
    I: DexMlilIntrinsicLowerer,
>(
    file: &DexFile,
    function: &Function,
    resolver: &mut R,
    intrinsics: &mut I,
) -> Result<LoweredBody> {
    verify_function(function)?;
    let allocation = RegisterAllocation::compute(function)?;
    let mut planner = Planner::new();
    let entry = entry_target(function);
    planner.goto(entry)?;
    let mut emissions = BTreeMap::new();
    for block in function.cfg().blocks() {
        if block.id() == function.cfg().entry() {
            continue;
        }
        planner.bind(block.id());
        for instruction in block.instructions() {
            let emission = emit_instruction(
                &mut planner,
                instruction,
                &allocation,
                file,
                resolver,
                intrinsics,
                function,
                block.id(),
            )?;
            emissions.insert(instruction.id(), emission);
        }
        emit_implicit_transfer(function, block.id(), &mut planner)?;
    }
    let tries = exception_tries(function, file, resolver, &planner, &emissions)?;
    let instructions = planner.finish()?;
    let code = CodeItem {
        registers_size: allocation.registers_size(),
        ins_size: allocation.ins_size(),
        outs_size: allocation.outs_size(),
        instructions,
        tries,
        debug_info: None,
        data_offset: 0,
    };
    let body = llil::Body::from_code(&code)?;
    let source_map = source_map(function, &emissions)?;
    Ok(LoweredBody { body, source_map })
}

fn verify_function(function: &Function) -> Result<()> {
    let report = function.verify();
    if report.is_ok() {
        Ok(())
    } else {
        Err(::mlil::Error::from(report).into())
    }
}

fn entry_target(function: &Function) -> BlockId {
    let edges = function.cfg().successor_edges(function.cfg().entry());
    function.cfg().edge(edges[0]).target()
}

fn emit_implicit_transfer(
    function: &Function,
    block: BlockId,
    planner: &mut Planner,
) -> Result<()> {
    let terminal = function.cfg().block(block).instructions().last();
    if terminal.is_some_and(|instruction| {
        matches!(
            instruction.operation(),
            Operation::Branch(_)
                | Operation::Jump
                | Operation::Switch(_)
                | Operation::Return
                | Operation::Throw
        )
    }) {
        return Ok(());
    }
    let ordinary = function
        .cfg()
        .successor_edges(block)
        .iter()
        .map(|edge| function.cfg().edge(*edge))
        .filter(|edge| !edge.payload().role.is_exception())
        .collect::<Vec<_>>();
    match ordinary.as_slice() {
        [] => Ok(()),
        [edge] => planner.goto(edge.target()).map(drop),
        _ => Err(Error::lowering(
            terminal.map_or(InstructionId::from_raw(0), ::mlil::Instruction::id),
            "non-control block has more than one ordinary successor",
        )),
    }
}

fn exception_tries<R: DexMlilReferenceResolver>(
    function: &Function,
    file: &DexFile,
    resolver: &mut R,
    planner: &Planner,
    emissions: &BTreeMap<InstructionId, Emission>,
) -> Result<Vec<TryBlock>> {
    let mut grouped =
        BTreeMap::<InstructionId, Vec<(u32, disassembler::CatchType, BlockId)>>::new();
    let mut seen = BTreeSet::new();
    for edge in function.cfg().edges() {
        let EdgeRole::Exception {
            catch,
            handler_order,
            ..
        } = &edge.payload().role
        else {
            continue;
        };
        let throw_site = edge
            .payload()
            .throw_site
            .expect("verified exception edge has a throw site");
        if seen.insert((throw_site, *handler_order, edge.target())) {
            grouped.entry(throw_site).or_default().push((
                *handler_order,
                catch.clone(),
                edge.target(),
            ));
        }
    }
    let mut tries = Vec::with_capacity(grouped.len());
    for (throw_site, mut catches) in grouped {
        catches.sort_by_key(|(order, _, _)| *order);
        if catches.iter().enumerate().any(|(position, (_, catch, _))| {
            matches!(catch, disassembler::CatchType::Any) && position + 1 != catches.len()
        }) {
            return Err(Error::lowering(
                throw_site,
                "Dalvik catch-all handler must be last",
            ));
        }
        let range = emissions[&throw_site].throw_range.ok_or_else(|| {
            Error::lowering(
                throw_site,
                "exception edge names an instruction without emitted throw range",
            )
        })?;
        let instruction_count =
            u16::try_from(range.end.get() - range.start.get()).map_err(|_| {
                Error::lowering(
                    throw_site,
                    "generated Dalvik protected instruction exceeds u16",
                )
            })?;
        let handlers = catches
            .into_iter()
            .map(|(_, catch, target)| {
                let exception_type = match catch {
                    disassembler::CatchType::Any => None,
                    disassembler::CatchType::Type(descriptor) => {
                        Some(resolver.resolve_type(file, &descriptor).map_err(|source| {
                            Error::Reference {
                                instruction: throw_site,
                                source,
                            }
                        })?)
                    }
                };
                let address = planner.block_offset(target).ok_or_else(|| {
                    Error::lowering(throw_site, format!("handler block {target} has no offset"))
                })?;
                Ok(CatchHandler {
                    exception_type,
                    address,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        tries.push(TryBlock {
            start_address: u32::try_from(range.start.get())
                .map_err(|_| Error::lowering(throw_site, "generated try start exceeds u32"))?,
            instruction_count,
            handlers,
        });
    }
    tries.sort_by_key(|protected| protected.start_address);
    Ok(tries)
}

fn source_map(
    function: &Function,
    emissions: &BTreeMap<InstructionId, Emission>,
) -> Result<SourceMap> {
    let generated = FunctionCoordinate::new(
        BinaryFormat::Dex,
        function.source().symbol.clone(),
        AddressUnit::CodeUnit16,
    );
    let mut map = SourceMap::new(function.source().clone(), generated);
    for (instruction, emission) in emissions {
        let generated_range = AddressRange::new(
            CodeAddress::from(emission.start),
            CodeAddress::from(emission.end),
        );
        for origin in function
            .provenance()
            .mappings_to(EntityId::Instruction(*instruction))
        {
            map.insert(origin.source, generated_range)
                .map_err(|error| {
                    Error::lowering(
                        *instruction,
                        format!("cannot record generated source map: {error}"),
                    )
                })?;
        }
    }
    Ok(map)
}

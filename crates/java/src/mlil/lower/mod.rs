//! Checked canonical lowering from shared MLIL to JVM LLIL.

mod arrays;
mod instruction;
mod intrinsic;
mod locals;
mod typing;

use std::collections::{BTreeMap, BTreeSet};

use ::mlil::{EdgeRole, EntityId, Function, InstructionId, Operation};
use disassembler::cfglib::BlockId;
use disassembler::{AddressUnit, BinaryFormat, FunctionCoordinate, ReferenceKind, SourceMap};

use crate::bytecode::{CatchTarget, CodeBuilder, Label};
use crate::classfile::{CodeAttribute, Constant, ConstantPool};
use crate::llil;
use crate::{DisplayJavaReferenceResolver, JavaReferenceResolutionError, JavaReferenceResolver};

use self::instruction::{Emission, emit_instruction};
pub use self::intrinsic::{
    JavaIntrinsicInstruction, JavaIntrinsicLoweringError, JavaIntrinsicRequest,
    JavaMlilIntrinsicLowerer, RejectJavaIntrinsics,
};
use self::locals::LocalAllocation;
use super::{Error, Result};

/// Canonically generated JVM LLIL plus original-native to generated-native provenance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoweredBody {
    /// Verified JVM LLIL body.
    pub body: llil::Body,
    /// Many-to-many correspondence retained through MLIL instruction identities.
    pub source_map: SourceMap,
}

/// Resolver that retains checked indices into the MLIL function's source pool.
///
/// Use [`lower_body_with_resolver`] with a symbolic resolver when targeting a
/// rebuilt or different constant pool.
#[derive(Debug, Clone, Copy, Default)]
pub struct SourceJavaReferenceResolver;

impl JavaReferenceResolver for SourceJavaReferenceResolver {
    fn resolve(
        &mut self,
        reference: &disassembler::Reference,
        pool: &mut ConstantPool,
    ) -> std::result::Result<u16, JavaReferenceResolutionError> {
        let index = u16::try_from(reference.index).map_err(|_| {
            JavaReferenceResolutionError::new("source constant-pool index exceeds u16")
        })?;
        let constant = pool.get(index).map_err(|error| {
            JavaReferenceResolutionError::new(format!(
                "source constant-pool index #{index} is invalid: {error}"
            ))
        })?;
        if !source_constant_matches(reference.kind, constant) {
            return Err(JavaReferenceResolutionError::new(format!(
                "source constant-pool index #{index} is incompatible with {:?}",
                reference.kind
            )));
        }
        Ok(index)
    }
}

fn source_constant_matches(kind: ReferenceKind, constant: &Constant) -> bool {
    match kind {
        ReferenceKind::Constant => matches!(
            constant,
            Constant::Integer(_)
                | Constant::Float(_)
                | Constant::Long(_)
                | Constant::Double(_)
                | Constant::Dynamic { .. }
        ),
        ReferenceKind::String => matches!(constant, Constant::String { .. }),
        ReferenceKind::Type => matches!(constant, Constant::Class { .. }),
        ReferenceKind::Field => matches!(constant, Constant::FieldRef { .. }),
        ReferenceKind::Method => matches!(constant, Constant::MethodRef { .. }),
        ReferenceKind::InterfaceMethod => {
            matches!(constant, Constant::InterfaceMethodRef { .. })
        }
        ReferenceKind::MethodPrototype => matches!(constant, Constant::MethodType { .. }),
        ReferenceKind::MethodHandle => matches!(constant, Constant::MethodHandle { .. }),
        ReferenceKind::DynamicCallSite => matches!(constant, Constant::InvokeDynamic { .. }),
    }
}

/// Lowers verified MLIL into JVM LLIL using reconstructable symbolic references.
///
/// The generated body uses fresh layout, resource counts, exception ranges, and
/// an empty nested-attribute list. Stale debug and stack-map offsets are never
/// copied from the source body.
///
/// # Errors
///
/// Returns an error when MLIL is invalid, contains a target construct without a
/// JVM encoding, or a symbolic reference cannot be interned.
pub fn lower_body(function: &Function, pool: &mut ConstantPool) -> Result<LoweredBody> {
    lower_body_with_resolver(function, pool, &mut DisplayJavaReferenceResolver)
}

/// Lowers JVM-origin MLIL while retaining checked indices in its source pool.
///
/// This is the escape hatch for source constants whose recursive bootstrap
/// metadata is not reconstructable from one MLIL reference. Prefer
/// [`lower_body`] for retargetable symbolic generation.
///
/// # Errors
///
/// Returns an error for non-JVM provenance, invalid MLIL, stale source indices,
/// or target constructs without a JVM encoding.
pub fn lower_body_from_source(function: &Function, pool: &mut ConstantPool) -> Result<LoweredBody> {
    if function.source().format != BinaryFormat::JavaClass {
        return Err(Error::WrongFormat {
            actual: function.source().format,
        });
    }
    lower_body_with_resolver(function, pool, &mut SourceJavaReferenceResolver)
}

/// Lowers verified MLIL using an explicit target constant-pool resolver.
///
/// # Errors
///
/// Returns the same structural and target failures as [`lower_body`], plus
/// resolver-specific failures for symbolic constants and members.
pub fn lower_body_with_resolver<R: JavaReferenceResolver>(
    function: &Function,
    pool: &mut ConstantPool,
    resolver: &mut R,
) -> Result<LoweredBody> {
    lower_body_with_resolver_and_intrinsics(function, pool, resolver, &mut RejectJavaIntrinsics)
}

/// Lowers verified MLIL with explicit target reference and intrinsic policies.
///
/// # Errors
///
/// Returns the same failures as [`lower_body_with_resolver`] plus a contextual
/// failure from `intrinsics` when it declines or mis-encodes an operation.
pub fn lower_body_with_resolver_and_intrinsics<
    R: JavaReferenceResolver,
    I: JavaMlilIntrinsicLowerer,
>(
    function: &Function,
    pool: &mut ConstantPool,
    resolver: &mut R,
    intrinsics: &mut I,
) -> Result<LoweredBody> {
    verify_function(function)?;
    let allocation = LocalAllocation::compute(function)?;
    let mut builder = CodeBuilder::new();
    let labels = function
        .cfg()
        .blocks()
        .iter()
        .filter(|block| block.id() != function.cfg().entry())
        .map(|block| (block.id(), builder.new_label()))
        .collect::<BTreeMap<_, _>>();
    let entry = entry_target(function);
    builder.emit_branch(crate::bytecode::Opcode::Goto, labels[&entry])?;

    let mut emissions = BTreeMap::new();
    let mut maximum_stack = 0u16;
    for block in function.cfg().blocks() {
        if block.id() == function.cfg().entry() {
            continue;
        }
        builder.bind(labels[&block.id()])?;
        for instruction in block.instructions() {
            let emission = emit_instruction(
                &mut builder,
                instruction,
                &allocation,
                pool,
                resolver,
                intrinsics,
                function,
                &labels,
                block.id(),
            )?;
            maximum_stack = maximum_stack.max(emission.maximum_stack);
            emissions.insert(instruction.id(), emission);
        }
        emit_implicit_transfer(function, block.id(), &mut builder, &labels)?;
    }
    add_exception_handlers(function, pool, &mut builder, &labels, &emissions)?;

    let built = builder.finish()?;
    let code =
        CodeAttribute::from_built(pool, maximum_stack.max(1), allocation.max_locals(), &built)?;
    let body = llil::Body::from_code(&code)?;
    let source_map = source_map(function, &built, &emissions)?;
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
    let edge = function.cfg().edge(edges[0]);
    edge.target()
}

fn emit_implicit_transfer(
    function: &Function,
    block: BlockId,
    builder: &mut CodeBuilder,
    labels: &BTreeMap<BlockId, Label>,
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
        [edge] => {
            builder.emit_branch(crate::bytecode::Opcode::Goto, labels[&edge.target()])?;
            Ok(())
        }
        _ => Err(Error::lowering(
            terminal.map_or(InstructionId::from_raw(0), ::mlil::Instruction::id),
            "non-control block has more than one ordinary successor",
        )),
    }
}

fn add_exception_handlers(
    function: &Function,
    pool: &mut ConstantPool,
    builder: &mut CodeBuilder,
    labels: &BTreeMap<BlockId, Label>,
    emissions: &BTreeMap<InstructionId, Emission>,
) -> Result<()> {
    let mut seen = BTreeSet::new();
    let mut handlers = Vec::new();
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
        let key = (throw_site, *handler_order, edge.target());
        if !seen.insert(key) {
            continue;
        }
        handlers.push((throw_site, *handler_order, edge.target(), catch.clone()));
    }
    handlers.sort_by_key(|(throw_site, handler_order, target, _)| {
        (*throw_site, *handler_order, *target)
    });
    for (throw_site, _, target, catch) in handlers {
        let emission = &emissions[&throw_site];
        let (start, end) = emission.throw_range.ok_or_else(|| {
            Error::lowering(
                throw_site,
                "exception edge names an instruction without emitted throw range",
            )
        })?;
        let catch = match catch {
            disassembler::CatchType::Any => CatchTarget::Any,
            disassembler::CatchType::Type(descriptor) => {
                let name = descriptor
                    .strip_prefix('L')
                    .and_then(|value| value.strip_suffix(';'))
                    .unwrap_or(&descriptor);
                CatchTarget::Class(pool.intern_class(name)?)
            }
        };
        builder.add_exception_handler(start, end, labels[&target], catch)?;
    }
    Ok(())
}

fn source_map(
    function: &Function,
    built: &crate::bytecode::BuiltCode,
    emissions: &BTreeMap<InstructionId, Emission>,
) -> Result<SourceMap> {
    let generated = FunctionCoordinate::new(
        BinaryFormat::JavaClass,
        function.source().symbol.clone(),
        AddressUnit::Byte,
    );
    let mut map = SourceMap::new(function.source().clone(), generated);
    for (instruction, emission) in emissions {
        let generated = built.label_range(emission.start, emission.end)?;
        for origin in function
            .provenance()
            .mappings_to(EntityId::Instruction(*instruction))
        {
            map.insert(origin.source, generated).map_err(|error| {
                Error::lowering(
                    *instruction,
                    format!("cannot record generated source map: {error}"),
                )
            })?;
        }
    }
    Ok(map)
}

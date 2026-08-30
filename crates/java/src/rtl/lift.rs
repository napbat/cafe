//! Direct verified JVM LLIL to JVM RTL graph construction.

use std::collections::{BTreeMap, BTreeSet};

use cfglib::BlockId;
use cfglib::ir::rtl::{Expr, FunctionBuilder, Place, Signature, StatementId};
use disassembler::{
    AddressRange, AddressUnit, BinaryFormat, CatchType, CodeAddress, FunctionCoordinate,
    FunctionSymbol,
};
use mlil::{
    EdgeRole, JavaDialect, NativeVariable, Operation, SourceStorage, TypedVariable, VariableId,
    VariableRole,
};

use crate::analysis::{
    ClassHierarchy, MethodAnalysis, ReferenceHierarchy, analyze_decoded_code_with_hierarchy,
};
use crate::bytecode::{Instruction as NativeInstruction, Opcode, decode, decode_code};
use crate::classfile::{
    CATCH_ALL_EXCEPTION_INDEX, CodeAttribute, ConstantPool, ExceptionHandler, MethodAccessFlags,
};
use crate::descriptor::parse_method;
use crate::llil;
use crate::mlil::instruction::{LiftedInstruction, Step, lift_instruction};
use crate::mlil::state::{StateVariables, VariableAllocator, reference_descriptor};
use crate::mlil::{Error, Result};

use super::{EdgeMetadata, Function, JvmConstraint, JvmRtlDialect, JvmStorage};

struct NativeBlocks {
    blocks: BTreeMap<usize, BlockId>,
    normal_sources: BTreeMap<usize, BlockId>,
    throw_sites: BTreeMap<usize, StatementId>,
    ranges: BTreeMap<usize, AddressRange>,
}

#[derive(Default)]
struct Variables {
    storage: Vec<JvmStorage>,
    temporary: u32,
}

impl VariableAllocator for Variables {
    fn declare_variable(
        &mut self,
        _role: VariableRole,
        native: Option<NativeVariable>,
    ) -> mlil::Result<VariableId> {
        let storage = match native {
            Some(NativeVariable {
                format: BinaryFormat::JavaClass,
                storage: SourceStorage::JvmLocal(index),
            }) => JvmStorage::SourceLocal(index),
            Some(NativeVariable {
                format: BinaryFormat::JavaClass,
                storage: SourceStorage::JvmStack(index),
            }) => JvmStorage::SourceStack(index),
            Some(_) => {
                return Err(mlil::Error::InvalidConstruction(
                    "JVM semantic state names non-JVM native storage".into(),
                ));
            }
            None => {
                let temporary = self.temporary;
                self.temporary = self.temporary.checked_add(1).ok_or_else(|| {
                    mlil::Error::InvalidConstruction("JVM RTL temporary space exceeds u32".into())
                })?;
                JvmStorage::Temporary(temporary)
            }
        };
        let raw = u32::try_from(self.storage.len()).map_err(|_| {
            mlil::Error::InvalidConstruction("JVM semantic variable space exceeds u32".into())
        })?;
        self.storage.push(storage);
        Ok(VariableId::from_raw(raw))
    }
}

impl Variables {
    fn storage(&self, variable: VariableId) -> mlil::Result<JvmStorage> {
        self.storage.get(variable.index()).copied().ok_or_else(|| {
            mlil::Error::InvalidConstruction(format!(
                "JVM semantic variable {variable} has no RTL storage"
            ))
        })
    }

    fn place(&self, variable: VariableId) -> mlil::Result<Place<JvmRtlDialect>> {
        Ok(Place {
            storage: self.storage(variable)?,
            lanes: vec![0],
        })
    }

    fn read(&self, variable: &TypedVariable) -> mlil::Result<Expr<JvmRtlDialect>> {
        Ok(Expr::Read {
            storage: self.storage(variable.variable)?,
            lanes: vec![0],
            scalar: <JvmConstraint as mlil::rtl::ValueConstraint>::from_value_type(
                variable.value_type.clone(),
            ),
        })
    }
}

pub(super) fn lift_body(
    pool: &ConstantPool,
    owner: &str,
    name: &str,
    descriptor: &str,
    access_flags: MethodAccessFlags,
    body: &llil::Body,
) -> Result<Function> {
    let hierarchy = ClassHierarchy::new();
    lift_body_with_hierarchy(
        pool,
        owner,
        name,
        descriptor,
        access_flags,
        body,
        &hierarchy,
    )
}

pub(super) fn lift_body_with_hierarchy(
    pool: &ConstantPool,
    owner: &str,
    name: &str,
    descriptor: &str,
    access_flags: MethodAccessFlags,
    body: &llil::Body,
    hierarchy: &dyn ReferenceHierarchy,
) -> Result<Function> {
    let code = body.to_code()?;
    let native = decode(&code.code)?;
    let analysis = analyze_decoded_code_with_hierarchy(
        pool,
        owner,
        name,
        descriptor,
        access_flags,
        &code,
        &native,
        hierarchy,
    )?;
    lift_analyzed_body(
        pool,
        owner,
        name,
        descriptor,
        access_flags,
        body,
        &native,
        &analysis,
    )
}

#[allow(clippy::too_many_arguments)]
pub(super) fn lift_code_with_hierarchy(
    pool: &ConstantPool,
    owner: &str,
    name: &str,
    descriptor: &str,
    access_flags: MethodAccessFlags,
    code: &CodeAttribute,
    hierarchy: &dyn ReferenceHierarchy,
) -> Result<Function> {
    let native = decode_code(code)?;
    let body = llil::Body::from_decoded_code(code, &native)?;
    let analysis = analyze_decoded_code_with_hierarchy(
        pool,
        owner,
        name,
        descriptor,
        access_flags,
        code,
        &native,
        hierarchy,
    )?;
    lift_analyzed_body(
        pool,
        owner,
        name,
        descriptor,
        access_flags,
        &body,
        &native,
        &analysis,
    )
}

#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
fn lift_analyzed_body(
    pool: &ConstantPool,
    owner: &str,
    name: &str,
    descriptor: &str,
    access_flags: MethodAccessFlags,
    body: &llil::Body,
    native: &[NativeInstruction],
    analysis: &MethodAnalysis,
) -> Result<Function> {
    reject_unsupported(body)?;
    if native.len() != body.instructions.len() {
        return Err(Error::Unsupported {
            offset: 0,
            feature: "JVM LLIL/native instruction counts disagree after verification".into(),
        });
    }
    let coordinate = FunctionCoordinate::new(
        BinaryFormat::JavaClass,
        FunctionSymbol {
            owner: owner.to_owned(),
            name: name.to_owned(),
            signature: descriptor.to_owned(),
        },
        AddressUnit::Byte,
    );
    let synchronized = access_flags.contains(MethodAccessFlags::SYNCHRONIZED);
    let mut builder = FunctionBuilder::<JvmRtlDialect>::new(coordinate);
    let mut storage = Variables::default();
    let parsed_descriptor = parse_method(descriptor)?;
    let variables = StateVariables::declare(
        &mut storage,
        analysis.max_locals(),
        body.max_stack,
        &parsed_descriptor,
        access_flags,
    )?;
    let parameters = variables
        .parameters()
        .iter()
        .map(|&variable| storage.place(variable))
        .collect::<mlil::Result<Vec<_>>>()?;
    builder.set_signature(Signature::new(parameters, variables.returns().to_vec()))?;
    let mut blocks = allocate_native_blocks(&mut builder, native)?;
    let first = native.first().ok_or_else(|| Error::Unsupported {
        offset: 0,
        feature: "empty JVM code body".into(),
    })?;
    builder.add_edge(
        builder.entry(),
        blocks.blocks[&first.offset],
        EdgeMetadata::ordinary(EdgeRole::Entry),
    )?;

    for (instruction_index, (native_instruction, instruction)) in
        native.iter().zip(&body.instructions).enumerate()
    {
        let offset = native_instruction.offset;
        let (entry, exit) =
            analysis
                .frames_at(instruction_index)
                .ok_or_else(|| Error::Unsupported {
                    offset,
                    feature: "frame analysis omitted a JVM instruction boundary".into(),
                })?;
        let lifted = lift_instruction(
            &mut storage,
            &variables,
            pool,
            native_instruction,
            instruction,
            entry,
            exit,
            owner,
        )?;
        append_lifted(
            &mut builder,
            &storage,
            &mut blocks,
            native_instruction,
            lifted,
            opcode_may_throw(native_instruction.opcode, synchronized),
        )?;
    }

    add_normal_edges(&mut builder, body, native, &blocks)?;
    let landings = add_exception_edges(
        &mut builder,
        &storage,
        pool,
        body,
        analysis,
        &variables,
        &blocks,
        owner,
        synchronized,
    )?;
    add_exception_regions(&mut builder, pool, body, &blocks, &landings)?;
    Ok(builder.finish()?)
}

fn allocate_native_blocks(
    builder: &mut FunctionBuilder<JvmRtlDialect>,
    native: &[NativeInstruction],
) -> Result<NativeBlocks> {
    let mut blocks = BTreeMap::new();
    let mut ranges = BTreeMap::new();
    for instruction in native {
        let range = instruction_range(instruction)?;
        let block = builder.new_block(format!("jvm_{:04x}", instruction.offset));
        blocks.insert(instruction.offset, block);
        ranges.insert(instruction.offset, range);
    }
    Ok(NativeBlocks {
        normal_sources: blocks.clone(),
        blocks,
        throw_sites: BTreeMap::new(),
        ranges,
    })
}

fn append_lifted(
    builder: &mut FunctionBuilder<JvmRtlDialect>,
    storage: &Variables,
    blocks: &mut NativeBlocks,
    native: &NativeInstruction,
    mut lifted: LiftedInstruction,
    may_throw: bool,
) -> Result<()> {
    let range = blocks.ranges[&native.offset];
    let block = blocks.blocks[&native.offset];
    let normal_steps = lifted.steps.split_off(lifted.throw_step + 1);
    let mut ids = Vec::with_capacity(lifted.steps.len());
    for (index, step) in lifted.steps.into_iter().enumerate() {
        ids.push(append_step(
            builder,
            storage,
            block,
            step,
            index == lifted.throw_step && may_throw,
            range,
        )?);
    }
    blocks
        .throw_sites
        .insert(native.offset, ids[lifted.throw_step]);

    if !normal_steps.is_empty() {
        let continuation = builder.new_block(format!("jvm_{:04x}_continue", native.offset));
        builder.add_edge(
            block,
            continuation,
            EdgeMetadata::ordinary(EdgeRole::Commit),
        )?;
        for step in normal_steps {
            append_step(builder, storage, continuation, step, false, range)?;
        }
        blocks.normal_sources.insert(native.offset, continuation);
    }
    Ok(())
}

fn append_step(
    builder: &mut FunctionBuilder<JvmRtlDialect>,
    storage: &Variables,
    block: BlockId,
    step: Step,
    may_throw: bool,
    range: AddressRange,
) -> Result<StatementId> {
    let metadata = <JavaDialect as cfglib::ir::mlil::Dialect>::instruction_metadata(
        &step.operation,
        may_throw,
    );
    let operands = step
        .uses
        .iter()
        .map(|variable| storage.read(variable))
        .collect::<mlil::Result<Vec<_>>>()?;
    let definitions = step
        .defs
        .into_iter()
        .map(|variable| Ok((storage.place(variable.variable)?, variable.value_type)))
        .collect::<mlil::Result<Vec<_>>>()?;
    let statement = mlil::rtl::lower_operation::<JvmRtlDialect>(
        step.operation,
        operands,
        definitions,
        metadata.effects,
        metadata.may_throw,
    )?;
    Ok(builder.append(block, statement, Some(range))?)
}

fn add_normal_edges(
    builder: &mut FunctionBuilder<JvmRtlDialect>,
    body: &llil::Body,
    native: &[NativeInstruction],
    blocks: &NativeBlocks,
) -> Result<()> {
    for (position, (instruction, native_instruction)) in
        body.instructions.iter().zip(native).enumerate()
    {
        let source = blocks.normal_sources[&native_instruction.offset];
        match &instruction.operation {
            llil::Operation::Branch { target, .. } => {
                add_target_edge(
                    builder,
                    blocks,
                    source,
                    native_instruction.offset,
                    *target,
                    EdgeRole::BranchTrue,
                )?;
                let next = next_offset(native, position, native_instruction.offset)?;
                add_target_edge_usize(
                    builder,
                    blocks,
                    source,
                    native_instruction.offset,
                    next,
                    EdgeRole::BranchFalse,
                )?;
            }
            llil::Operation::Jump { target } => add_target_edge(
                builder,
                blocks,
                source,
                native_instruction.offset,
                *target,
                EdgeRole::Jump,
            )?,
            llil::Operation::Switch(table) => {
                add_target_edge(
                    builder,
                    blocks,
                    source,
                    native_instruction.offset,
                    table.default,
                    EdgeRole::SwitchDefault,
                )?;
                for case in &table.cases {
                    add_target_edge(
                        builder,
                        blocks,
                        source,
                        native_instruction.offset,
                        case.target,
                        EdgeRole::SwitchCase(i64::from(case.key)),
                    )?;
                }
            }
            llil::Operation::Return(_) | llil::Operation::Throw => {}
            llil::Operation::SubroutineCall { .. } | llil::Operation::SubroutineReturn { .. } => {
                unreachable!("legacy subroutines were rejected before RTL lifting")
            }
            _ => {
                let next = next_offset(native, position, native_instruction.offset)?;
                add_target_edge_usize(
                    builder,
                    blocks,
                    source,
                    native_instruction.offset,
                    next,
                    EdgeRole::FallThrough,
                )?;
            }
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn add_exception_edges(
    builder: &mut FunctionBuilder<JvmRtlDialect>,
    storage: &Variables,
    pool: &ConstantPool,
    body: &llil::Body,
    analysis: &MethodAnalysis,
    variables: &StateVariables,
    blocks: &NativeBlocks,
    owner: &str,
    synchronized: bool,
) -> Result<Vec<BlockId>> {
    let mut landings = Vec::with_capacity(body.exception_table.len());
    let mut shared: BTreeMap<(usize, CatchType), BlockId> = BTreeMap::new();
    for (order, handler) in body.exception_table.iter().enumerate() {
        let handler_offset = usize::from(handler.handler_pc);
        let catch = catch_type(pool, *handler)?;
        let landing = if let Some(&landing) = shared.get(&(handler_offset, catch.clone())) {
            landing
        } else {
            let landing = builder.new_block(format!("jvm_{handler_offset:04x}_handler_{order}"));
            shared.insert((handler_offset, catch.clone()), landing);
            let handler_range =
                blocks
                    .ranges
                    .get(&handler_offset)
                    .copied()
                    .ok_or(Error::MissingTarget {
                        source_offset: handler_offset,
                        target: handler_offset,
                    })?;
            let handler_frame =
                analysis
                    .entry_frame(handler_offset)
                    .ok_or_else(|| Error::Unsupported {
                        offset: handler_offset,
                        feature: "handler has no analyzed entry frame".into(),
                    })?;
            let caught = variables.stack(handler_frame, 0, owner);
            append_step(
                builder,
                storage,
                landing,
                Step {
                    operation: Operation::CaughtException(catch.clone()),
                    uses: vec![],
                    defs: vec![caught],
                },
                false,
                handler_range,
            )?;
            builder.add_edge(
                landing,
                blocks.blocks[&handler_offset],
                EdgeMetadata::ordinary(EdgeRole::FallThrough),
            )?;
            landing
        };
        landings.push(landing);

        let protected = AddressRange::new(handler.start_pc.into(), handler.end_pc.into());
        let handler_order = u32::try_from(order).map_err(|_| Error::Unsupported {
            offset: handler_offset,
            feature: "exception-table order exceeds u32".into(),
        })?;
        for instruction in &body.instructions {
            if protected_handlers(std::slice::from_ref(handler), instruction.offset).is_empty()
                || !opcode_may_throw(instruction.encoding.opcode, synchronized)
            {
                continue;
            }
            let source = blocks.blocks[&instruction.offset];
            builder.add_edge(
                source,
                landing,
                EdgeMetadata {
                    role: EdgeRole::Exception {
                        catch: catch.clone(),
                        handler_order,
                        protected,
                    },
                    throw_site: Some(blocks.throw_sites[&instruction.offset]),
                },
            )?;
        }
    }
    Ok(landings)
}

fn add_exception_regions(
    builder: &mut FunctionBuilder<JvmRtlDialect>,
    pool: &ConstantPool,
    body: &llil::Body,
    blocks: &NativeBlocks,
    landings: &[BlockId],
) -> Result<()> {
    use cfglib::{Handler, HandlerBody, HandlerKind, Region, RegionId};

    let mut order = Vec::new();
    let mut groups: BTreeMap<BlockId, (HandlerKind, BTreeSet<BlockId>)> = BTreeMap::new();
    for (entry, &landing) in body.exception_table.iter().zip(landings) {
        let kind = match catch_type(pool, *entry)? {
            CatchType::Any => HandlerKind::CatchAll,
            CatchType::Type(_) => HandlerKind::Catch,
        };
        let group = groups.entry(landing).or_insert_with(|| {
            order.push(landing);
            (kind, BTreeSet::new())
        });
        for (&offset, &block) in &blocks.blocks {
            if offset >= usize::from(entry.start_pc) && offset < usize::from(entry.end_pc) {
                group.1.insert(block);
                group.1.insert(blocks.normal_sources[&offset]);
            }
        }
    }
    let mut regions: Vec<(BTreeSet<BlockId>, Vec<Handler>)> = Vec::new();
    for landing in order {
        let Some((kind, protected)) = groups.remove(&landing) else {
            continue;
        };
        if protected.is_empty() {
            continue;
        }
        let handler = Handler {
            entry: landing,
            body: HandlerBody::Unknown,
            kind,
        };
        if let Some(existing) = regions.iter_mut().find(|(set, _)| *set == protected) {
            existing.1.push(handler);
        } else {
            regions.push((protected, vec![handler]));
        }
    }
    for (protected_blocks, handlers) in regions {
        builder.add_region(Region {
            id: RegionId::from_raw(0),
            protected_blocks,
            handlers,
            parent: None,
        })?;
    }
    Ok(())
}

fn add_target_edge(
    builder: &mut FunctionBuilder<JvmRtlDialect>,
    blocks: &NativeBlocks,
    source: BlockId,
    source_offset: usize,
    target: i32,
    role: EdgeRole,
) -> Result<()> {
    let target = usize::try_from(target).map_err(|_| Error::MissingTarget {
        source_offset,
        target: usize::MAX,
    })?;
    add_target_edge_usize(builder, blocks, source, source_offset, target, role)
}

fn add_target_edge_usize(
    builder: &mut FunctionBuilder<JvmRtlDialect>,
    blocks: &NativeBlocks,
    source: BlockId,
    source_offset: usize,
    target: usize,
    role: EdgeRole,
) -> Result<()> {
    let target = blocks
        .blocks
        .get(&target)
        .copied()
        .ok_or(Error::MissingTarget {
            source_offset,
            target,
        })?;
    builder.add_edge(source, target, EdgeMetadata::ordinary(role))?;
    Ok(())
}

fn next_offset(native: &[NativeInstruction], position: usize, source: usize) -> Result<usize> {
    native
        .get(position + 1)
        .map(|instruction| instruction.offset)
        .ok_or(Error::MissingTarget {
            source_offset: source,
            target: source.saturating_add(1),
        })
}

fn protected_handlers(handlers: &[ExceptionHandler], offset: usize) -> Vec<&ExceptionHandler> {
    handlers
        .iter()
        .filter(|handler| {
            offset >= usize::from(handler.start_pc) && offset < usize::from(handler.end_pc)
        })
        .collect()
}

fn catch_type(pool: &ConstantPool, handler: ExceptionHandler) -> Result<CatchType> {
    if handler.catch_type == CATCH_ALL_EXCEPTION_INDEX {
        Ok(CatchType::Any)
    } else {
        Ok(CatchType::Type(reference_descriptor(
            pool.class_name(handler.catch_type)?,
        )))
    }
}

fn instruction_range(instruction: &NativeInstruction) -> Result<AddressRange> {
    let start = u64::try_from(instruction.offset).map_err(|_| Error::Unsupported {
        offset: instruction.offset,
        feature: "JVM offset exceeds u64".into(),
    })?;
    let size = u64::try_from(instruction.size).map_err(|_| Error::Unsupported {
        offset: instruction.offset,
        feature: "JVM size exceeds u64".into(),
    })?;
    let end = start.checked_add(size).ok_or_else(|| Error::Unsupported {
        offset: instruction.offset,
        feature: "JVM range overflow".into(),
    })?;
    Ok(AddressRange::new(
        CodeAddress::new(start),
        CodeAddress::new(end),
    ))
}

fn reject_unsupported(body: &llil::Body) -> Result<()> {
    for instruction in &body.instructions {
        match &instruction.operation {
            llil::Operation::SubroutineCall { .. } | llil::Operation::SubroutineReturn { .. } => {
                return Err(Error::Unsupported {
                    offset: instruction.offset,
                    feature: "legacy jsr/ret subroutines".into(),
                });
            }
            llil::Operation::Intrinsic(intrinsic) => {
                return Err(Error::Unsupported {
                    offset: instruction.offset,
                    feature: format!("reserved JVM intrinsic {intrinsic:?}"),
                });
            }
            _ => {}
        }
    }
    Ok(())
}

fn opcode_may_throw(opcode: Opcode, synchronized: bool) -> bool {
    if matches!(
        opcode,
        Opcode::IReturn
            | Opcode::LReturn
            | Opcode::FReturn
            | Opcode::DReturn
            | Opcode::AReturn
            | Opcode::Return
    ) {
        return synchronized;
    }
    matches!(
        opcode,
        Opcode::Ldc
            | Opcode::LdcW
            | Opcode::Ldc2W
            | Opcode::IALoad
            | Opcode::LALoad
            | Opcode::FALoad
            | Opcode::DALoad
            | Opcode::AALoad
            | Opcode::BALoad
            | Opcode::CALoad
            | Opcode::SALoad
            | Opcode::IAStore
            | Opcode::LAStore
            | Opcode::FAStore
            | Opcode::DAStore
            | Opcode::AAStore
            | Opcode::BAStore
            | Opcode::CAStore
            | Opcode::SAStore
            | Opcode::IDiv
            | Opcode::LDiv
            | Opcode::IRem
            | Opcode::LRem
            | Opcode::GetStatic
            | Opcode::PutStatic
            | Opcode::GetField
            | Opcode::PutField
            | Opcode::InvokeVirtual
            | Opcode::InvokeSpecial
            | Opcode::InvokeStatic
            | Opcode::InvokeInterface
            | Opcode::InvokeDynamic
            | Opcode::New
            | Opcode::NewArray
            | Opcode::ANewArray
            | Opcode::ArrayLength
            | Opcode::AThrow
            | Opcode::CheckCast
            | Opcode::InstanceOf
            | Opcode::MonitorEnter
            | Opcode::MonitorExit
            | Opcode::MultiANewArray
    )
}

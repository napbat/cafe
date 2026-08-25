//! Whole-method JVM LLIL to MLIL graph construction.

use std::collections::BTreeMap;

use ::mlil::{
    EdgeMetadata, EdgeRole, EntityId, Function, FunctionBuilder, Operation, TypedVariable,
    VariableRole,
};
use disassembler::cfglib::BlockId;
use disassembler::{
    AddressRange, AddressUnit, BinaryFormat, CatchType, CodeAddress, FunctionCoordinate,
    FunctionSymbol,
};

use crate::analysis::{
    ClassHierarchy, MethodAnalysis, ReferenceHierarchy, analyze_code, analyze_code_with_hierarchy,
};
use crate::bytecode::{Instruction as NativeInstruction, Opcode, decode};
use crate::classfile::{
    CATCH_ALL_EXCEPTION_INDEX, ClassFile, ConstantPool, ExceptionHandler, MethodAccessFlags,
    MethodInfo,
};
use crate::descriptor::parse_method;
use crate::llil;

use super::instruction::{LiftedInstruction, lift_instruction};
use super::state::{StateVariables, reference_descriptor};
use super::{Error, Result};

struct NativeBlocks {
    blocks: BTreeMap<usize, BlockId>,
    normal_sources: BTreeMap<usize, BlockId>,
    throw_sites: BTreeMap<usize, ::mlil::InstructionId>,
    ranges: BTreeMap<usize, AddressRange>,
}

/// Lifts one class method into shared MLIL when it has executable code.
///
/// # Errors
///
/// Returns an error for malformed metadata or bytecode, unsupported legacy
/// subroutines, failed frame analysis, or invalid generated MLIL.
pub fn lift_method(class: &ClassFile, method: &MethodInfo) -> Result<Option<Function>> {
    let hierarchy = ClassHierarchy::from_classes([class])?;
    lift_method_with_hierarchy(class, method, &hierarchy)
}

/// Lifts one class method using caller-supplied class relationships.
///
/// # Errors
///
/// Returns an error for malformed metadata or bytecode, unsupported legacy
/// subroutines, failed frame analysis, or invalid generated MLIL.
pub fn lift_method_with_hierarchy(
    class: &ClassFile,
    method: &MethodInfo,
    hierarchy: &dyn ReferenceHierarchy,
) -> Result<Option<Function>> {
    let Some(code) = method.code() else {
        return Ok(None);
    };
    let owner = class.class_name()?;
    let name = method.name(&class.constant_pool)?;
    let descriptor = method.descriptor(&class.constant_pool)?;
    let body = llil::Body::from_code(code)?;
    lift_body_with_hierarchy(
        &class.constant_pool,
        owner,
        name,
        descriptor,
        method.access_flags,
        &body,
        hierarchy,
    )
    .map(Some)
}

/// Lifts a standalone verified JVM LLIL body into shared MLIL.
///
/// # Errors
///
/// Returns an error for invalid descriptors, constant references, LLIL/native
/// disagreement, frame analysis failures, or invalid generated MLIL.
pub fn lift_body(
    pool: &ConstantPool,
    owner: &str,
    name: &str,
    descriptor: &str,
    access_flags: MethodAccessFlags,
    body: &llil::Body,
) -> Result<Function> {
    let code = body.to_code()?;
    let analysis = analyze_code(pool, owner, name, descriptor, access_flags, &code)?;
    lift_analyzed_body(
        pool,
        owner,
        name,
        descriptor,
        access_flags,
        body,
        &code.code,
        &analysis,
    )
}

/// Lifts a standalone JVM LLIL body using caller-supplied class relationships.
///
/// # Errors
///
/// Returns an error for invalid descriptors, constant references, LLIL/native
/// disagreement, frame analysis failures, or invalid generated MLIL.
#[allow(clippy::too_many_arguments)]
pub fn lift_body_with_hierarchy(
    pool: &ConstantPool,
    owner: &str,
    name: &str,
    descriptor: &str,
    access_flags: MethodAccessFlags,
    body: &llil::Body,
    hierarchy: &dyn ReferenceHierarchy,
) -> Result<Function> {
    let code = body.to_code()?;
    let analysis = analyze_code_with_hierarchy(
        pool,
        owner,
        name,
        descriptor,
        access_flags,
        &code,
        hierarchy,
    )?;
    lift_analyzed_body(
        pool,
        owner,
        name,
        descriptor,
        access_flags,
        body,
        &code.code,
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
    native_code: &[u8],
    analysis: &MethodAnalysis,
) -> Result<Function> {
    reject_unsupported(body)?;
    let native = decode(native_code)?;
    if native.len() != body.instructions.len() {
        return Err(Error::unsupported(
            0,
            "JVM LLIL/native instruction counts disagree after verification",
        ));
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
    let mut builder = FunctionBuilder::new(coordinate);
    let parsed_descriptor = parse_method(descriptor)?;
    let variables = StateVariables::declare(
        &mut builder,
        analysis.max_locals(),
        body.max_stack,
        &parsed_descriptor,
        access_flags,
    )?;
    let mut blocks = allocate_native_blocks(&mut builder, &native)?;
    let first = native
        .first()
        .ok_or_else(|| Error::unsupported(0, "empty JVM code body"))?;
    builder.add_edge(
        builder.entry(),
        blocks.blocks[&first.offset],
        EdgeMetadata::ordinary(EdgeRole::Entry),
        None,
    )?;

    for (native_instruction, instruction) in native.iter().zip(&body.instructions) {
        let offset = native_instruction.offset;
        let entry = analysis.entry_frame(offset).ok_or_else(|| {
            Error::unsupported(offset, "frame analysis omitted a JVM instruction entry")
        })?;
        let exit = analysis.exit_frame(offset).ok_or_else(|| {
            Error::unsupported(offset, "frame analysis omitted a JVM instruction exit")
        })?;
        let protected = if opcode_may_throw(native_instruction.opcode) {
            protected_handlers(&body.exception_table, offset)
        } else {
            Vec::new()
        };
        let lifted = lift_instruction(
            &mut builder,
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
            &variables,
            &mut blocks,
            native_instruction,
            lifted,
            !protected.is_empty(),
        )?;
    }

    add_normal_edges(&mut builder, body, &native, &blocks)?;
    add_exception_edges(
        &mut builder,
        pool,
        body,
        analysis,
        &variables,
        &blocks,
        owner,
    )?;
    Ok(builder.finish()?)
}

fn allocate_native_blocks(
    builder: &mut FunctionBuilder,
    native: &[NativeInstruction],
) -> Result<NativeBlocks> {
    let mut blocks = BTreeMap::new();
    let mut ranges = BTreeMap::new();
    for instruction in native {
        let range = instruction_range(instruction)?;
        let block = builder.new_block(format!("jvm_{:04x}", instruction.offset));
        builder.map_entity(range, EntityId::Block(block))?;
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
    builder: &mut FunctionBuilder,
    variables: &StateVariables,
    blocks: &mut NativeBlocks,
    native: &NativeInstruction,
    mut lifted: LiftedInstruction,
    has_exception_edges: bool,
) -> Result<()> {
    let range = blocks.ranges[&native.offset];
    let block = blocks.blocks[&native.offset];
    let normal_steps = lifted.steps.split_off(lifted.throw_step + 1);
    let mut commit_uses = Vec::new();
    let mut commit_defs = Vec::new();
    if has_exception_edges {
        for step in &mut lifted.steps {
            for definition in &mut step.defs {
                if variables.is_native_state(definition.variable) {
                    let native_definition = definition.clone();
                    let temporary = builder.declare_variable(VariableRole::Temporary, None)?;
                    *definition =
                        TypedVariable::new(temporary, native_definition.value_type.clone());
                    commit_uses.push(definition.clone());
                    commit_defs.push(native_definition);
                }
            }
        }
    }

    let may_throw = opcode_may_throw(native.opcode) || has_exception_edges;
    let mut instruction_ids = Vec::with_capacity(lifted.steps.len());
    for (index, step) in lifted.steps.into_iter().enumerate() {
        instruction_ids.push(builder.append_instruction(
            block,
            step.operation,
            step.uses,
            step.defs,
            index == lifted.throw_step && may_throw,
            Some(range),
        )?);
    }
    blocks
        .throw_sites
        .insert(native.offset, instruction_ids[lifted.throw_step]);

    if !commit_defs.is_empty() || !normal_steps.is_empty() {
        let commit = builder.new_block(format!("jvm_{:04x}_commit", native.offset));
        builder.map_entity(range, EntityId::Block(commit))?;
        builder.add_edge(
            block,
            commit,
            EdgeMetadata::ordinary(EdgeRole::Commit),
            Some(range),
        )?;
        if !commit_defs.is_empty() {
            let operation = if commit_defs.len() == 1 {
                Operation::Copy
            } else {
                Operation::ParallelCopy
            };
            builder.append_instruction(
                commit,
                operation,
                commit_uses,
                commit_defs,
                false,
                Some(range),
            )?;
        }
        for step in normal_steps {
            builder.append_instruction(
                commit,
                step.operation,
                step.uses,
                step.defs,
                false,
                Some(range),
            )?;
        }
        blocks.normal_sources.insert(native.offset, commit);
    }
    Ok(())
}

fn add_normal_edges(
    builder: &mut FunctionBuilder,
    body: &llil::Body,
    native: &[NativeInstruction],
    blocks: &NativeBlocks,
) -> Result<()> {
    for (position, (instruction, native_instruction)) in
        body.instructions.iter().zip(native).enumerate()
    {
        let source = blocks.normal_sources[&native_instruction.offset];
        let range = blocks.ranges[&native_instruction.offset];
        match &instruction.operation {
            llil::Operation::Branch { target, .. } => {
                add_target_edge(
                    builder,
                    blocks,
                    source,
                    native_instruction.offset,
                    *target,
                    EdgeRole::BranchTrue,
                    range,
                )?;
                let next = next_offset(native, position, native_instruction.offset)?;
                add_target_edge_usize(
                    builder,
                    blocks,
                    source,
                    native_instruction.offset,
                    next,
                    EdgeRole::BranchFalse,
                    range,
                )?;
            }
            llil::Operation::Jump { target } => add_target_edge(
                builder,
                blocks,
                source,
                native_instruction.offset,
                *target,
                EdgeRole::Jump,
                range,
            )?,
            llil::Operation::Switch(table) => {
                add_target_edge(
                    builder,
                    blocks,
                    source,
                    native_instruction.offset,
                    table.default,
                    EdgeRole::SwitchDefault,
                    range,
                )?;
                for case in &table.cases {
                    add_target_edge(
                        builder,
                        blocks,
                        source,
                        native_instruction.offset,
                        case.target,
                        EdgeRole::SwitchCase(i64::from(case.key)),
                        range,
                    )?;
                }
            }
            llil::Operation::Return(_) | llil::Operation::Throw => {}
            llil::Operation::SubroutineCall { .. } | llil::Operation::SubroutineReturn { .. } => {
                unreachable!("rejected before lifting")
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
                    range,
                )?;
            }
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn add_exception_edges(
    builder: &mut FunctionBuilder,
    pool: &ConstantPool,
    body: &llil::Body,
    analysis: &MethodAnalysis,
    variables: &StateVariables,
    blocks: &NativeBlocks,
    owner: &str,
) -> Result<()> {
    for (order, handler) in body.exception_table.iter().enumerate() {
        let handler_offset = usize::from(handler.handler_pc);
        let catch = catch_type(pool, *handler)?;
        let landing = builder.new_block(format!("jvm_{handler_offset:04x}_handler_{order}"));
        let handler_range =
            blocks
                .ranges
                .get(&handler_offset)
                .copied()
                .ok_or(Error::MissingTarget {
                    source_offset: handler_offset,
                    target: handler_offset,
                })?;
        builder.map_entity(handler_range, EntityId::Block(landing))?;
        let handler_frame = analysis.entry_frame(handler_offset).ok_or_else(|| {
            Error::unsupported(handler_offset, "handler has no analyzed entry frame")
        })?;
        let caught = variables.stack(handler_frame, 0, owner);
        builder.append_instruction(
            landing,
            Operation::CaughtException(catch.clone()),
            vec![],
            vec![caught],
            false,
            Some(handler_range),
        )?;
        builder.add_edge(
            landing,
            blocks.blocks[&handler_offset],
            EdgeMetadata::ordinary(EdgeRole::FallThrough),
            Some(handler_range),
        )?;

        let protected = AddressRange::new(
            CodeAddress::from(handler.start_pc),
            CodeAddress::from(handler.end_pc),
        );
        let handler_order = u32::try_from(order)
            .map_err(|_| Error::unsupported(handler_offset, "exception-table order exceeds u32"))?;
        for instruction in &body.instructions {
            if instruction.offset >= usize::from(handler.start_pc)
                && instruction.offset < usize::from(handler.end_pc)
                && opcode_may_throw(instruction.encoding.opcode)
            {
                let source = blocks.blocks[&instruction.offset];
                let throw_site = blocks.throw_sites[&instruction.offset];
                builder.add_edge(
                    source,
                    landing,
                    EdgeMetadata::exceptional(
                        EdgeRole::Exception {
                            catch: catch.clone(),
                            handler_order,
                            protected,
                        },
                        throw_site,
                    ),
                    Some(blocks.ranges[&instruction.offset]),
                )?;
            }
        }
    }
    Ok(())
}

fn add_target_edge(
    builder: &mut FunctionBuilder,
    blocks: &NativeBlocks,
    source: BlockId,
    source_offset: usize,
    target: i32,
    role: EdgeRole,
    range: AddressRange,
) -> Result<()> {
    let target = usize::try_from(target).map_err(|_| Error::MissingTarget {
        source_offset,
        target: usize::MAX,
    })?;
    add_target_edge_usize(builder, blocks, source, source_offset, target, role, range)
}

fn add_target_edge_usize(
    builder: &mut FunctionBuilder,
    blocks: &NativeBlocks,
    source: BlockId,
    source_offset: usize,
    target: usize,
    role: EdgeRole,
    range: AddressRange,
) -> Result<()> {
    let target_block = blocks
        .blocks
        .get(&target)
        .copied()
        .ok_or(Error::MissingTarget {
            source_offset,
            target,
        })?;
    builder.add_edge(
        source,
        target_block,
        EdgeMetadata::ordinary(role),
        Some(range),
    )?;
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
    let start = u64::try_from(instruction.offset)
        .map_err(|_| Error::unsupported(instruction.offset, "JVM offset exceeds u64"))?;
    let size = u64::try_from(instruction.size)
        .map_err(|_| Error::unsupported(instruction.offset, "JVM size exceeds u64"))?;
    let end = start
        .checked_add(size)
        .ok_or_else(|| Error::unsupported(instruction.offset, "JVM range overflow"))?;
    Ok(AddressRange::new(
        CodeAddress::new(start),
        CodeAddress::new(end),
    ))
}

fn reject_unsupported(body: &llil::Body) -> Result<()> {
    for instruction in &body.instructions {
        match &instruction.operation {
            llil::Operation::SubroutineCall { .. } | llil::Operation::SubroutineReturn { .. } => {
                return Err(Error::unsupported(
                    instruction.offset,
                    "legacy jsr/ret subroutines",
                ));
            }
            llil::Operation::Intrinsic(intrinsic) => {
                return Err(Error::unsupported(
                    instruction.offset,
                    format!("reserved JVM intrinsic {intrinsic:?}"),
                ));
            }
            _ => {}
        }
    }
    Ok(())
}

fn opcode_may_throw(opcode: Opcode) -> bool {
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
            | Opcode::IReturn
            | Opcode::LReturn
            | Opcode::FReturn
            | Opcode::DReturn
            | Opcode::AReturn
            | Opcode::Return
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

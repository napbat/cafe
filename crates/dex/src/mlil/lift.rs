//! Whole-method Dalvik LLIL to MLIL graph construction.

use std::collections::BTreeMap;

use ::mlil::{
    EdgeMetadata, EdgeRole, EntityId, Function, FunctionBuilder, Operation, TypedVariable,
    ValueType, VariableRole,
};
use disassembler::cfglib::BlockId;
use disassembler::{
    AddressRange, AddressUnit, BinaryFormat, CatchType, CodeAddress, FunctionCoordinate,
    FunctionSymbol,
};

use crate::analysis::{
    FlowEdgeKind, ReferenceHierarchy, RegisterAnalysis, analyze_body, analyze_method_registers,
    analyze_method_registers_with_hierarchy,
};
use crate::file::{CatchHandler, CodeItem, DexFile, EncodedMethod, TryBlock};
use crate::instruction::{Instruction as NativeInstruction, InstructionData};
use crate::llil::{self, InstructionKind, OperationKind};

use super::instruction::{LiftedInstruction, lift_instruction};
use super::state::StateVariables;
use super::{Error, Result};

const JAVA_LANG_THROWABLE_DESCRIPTOR: &str = "Ljava/lang/Throwable;";

struct NativeBlocks {
    blocks: BTreeMap<u32, BlockId>,
    normal_sources: BTreeMap<u32, BlockId>,
    throw_sites: BTreeMap<u32, ::mlil::InstructionId>,
    ranges: BTreeMap<u32, AddressRange>,
}

/// Lifts one encoded DEX method into shared MLIL when it has code.
///
/// # Errors
///
/// Returns an error for invalid identifiers, body relationships, register
/// states, payloads, exception metadata, or generated MLIL.
pub fn lift_method(file: &DexFile, declaration: &EncodedMethod) -> Result<Option<Function>> {
    let Some(code) = &declaration.code else {
        return Ok(None);
    };
    let analysis = analyze_method_registers(file, declaration)?.ok_or_else(|| {
        Error::unsupported(0, "register analysis omitted an executable DEX method")
    })?;
    let body = llil::Body::from_code(code)?;
    lift_analyzed_body(file, declaration, &body, code, &analysis).map(Some)
}

/// Lifts one encoded method using caller-supplied class relationships.
///
/// # Errors
///
/// Returns an error for invalid identifiers, body relationships, register
/// states, payloads, exception metadata, or generated MLIL.
pub fn lift_method_with_hierarchy(
    file: &DexFile,
    declaration: &EncodedMethod,
    hierarchy: &dyn ReferenceHierarchy,
) -> Result<Option<Function>> {
    let Some(code) = &declaration.code else {
        return Ok(None);
    };
    let analysis = analyze_method_registers_with_hierarchy(file, declaration, hierarchy)?
        .ok_or_else(|| {
            Error::unsupported(0, "register analysis omitted an executable DEX method")
        })?;
    let body = llil::Body::from_code(code)?;
    lift_analyzed_body(file, declaration, &body, code, &analysis).map(Some)
}

/// Lifts an edited Dalvik LLIL body using its method declaration identity.
///
/// # Errors
///
/// Returns an error for LLIL/native disagreement, invalid identifiers or
/// register states, or invalid generated MLIL.
pub fn lift_body(
    file: &DexFile,
    declaration: &EncodedMethod,
    body: &llil::Body,
) -> Result<Function> {
    let code = body.to_code()?;
    let mut analyzed_declaration = declaration.clone();
    analyzed_declaration.code = Some(code.clone());
    let analysis = analyze_method_registers(file, &analyzed_declaration)?.ok_or_else(|| {
        Error::unsupported(0, "register analysis omitted an executable DEX LLIL body")
    })?;
    lift_analyzed_body(file, &analyzed_declaration, body, &code, &analysis)
}

/// Lifts an edited Dalvik LLIL body with caller-supplied class relationships.
///
/// # Errors
///
/// Returns an error for LLIL/native disagreement, invalid identifiers or
/// register states, or invalid generated MLIL.
pub fn lift_body_with_hierarchy(
    file: &DexFile,
    declaration: &EncodedMethod,
    body: &llil::Body,
    hierarchy: &dyn ReferenceHierarchy,
) -> Result<Function> {
    let code = body.to_code()?;
    let mut analyzed_declaration = declaration.clone();
    analyzed_declaration.code = Some(code.clone());
    let analysis = analyze_method_registers_with_hierarchy(file, &analyzed_declaration, hierarchy)?
        .ok_or_else(|| {
            Error::unsupported(0, "register analysis omitted an executable DEX LLIL body")
        })?;
    lift_analyzed_body(file, &analyzed_declaration, body, &code, &analysis)
}

fn lift_analyzed_body(
    file: &DexFile,
    declaration: &EncodedMethod,
    body: &llil::Body,
    code: &CodeItem,
    analysis: &RegisterAnalysis,
) -> Result<Function> {
    let body_analysis = analyze_body(code)?;
    if code.instructions.len() != body.instructions.len() {
        return Err(Error::unsupported(
            0,
            "Dalvik LLIL/native instruction counts disagree after verification",
        ));
    }
    let identity = file.resolve_method(declaration.method)?;
    let coordinate = FunctionCoordinate::new(
        BinaryFormat::Dex,
        FunctionSymbol {
            owner: identity.owner.to_owned(),
            name: identity.name.to_owned(),
            signature: identity.signature,
        },
        AddressUnit::CodeUnit16,
    );
    let mut builder = FunctionBuilder::new(coordinate);
    let variables = StateVariables::declare(&mut builder, file, declaration, code)?;
    let mut blocks = allocate_native_blocks(&mut builder, &code.instructions)?;
    let entry_offset = analysis.flow().entry();
    let entry_block = blocks
        .blocks
        .get(&entry_offset)
        .copied()
        .ok_or(Error::MissingTarget {
            source_offset: entry_offset,
            target: entry_offset,
        })?;
    builder.add_edge(
        builder.entry(),
        entry_block,
        EdgeMetadata::ordinary(EdgeRole::Entry),
        None,
    )?;

    for (native, instruction) in code.instructions.iter().zip(&body.instructions) {
        let InstructionKind::Operation(operation) = &instruction.kind else {
            continue;
        };
        let offset = instruction.offset;
        let lifted = lift_instruction(
            &mut builder,
            &variables,
            file,
            native,
            instruction,
            analysis.entry_frame(offset),
            analysis.exit_frame(offset),
            body,
        )?;
        let has_exception_edges = operation.semantics.may_throw
            && body
                .tries
                .iter()
                .any(|try_block| protects(try_block, offset));
        append_lifted(
            &mut builder,
            &variables,
            &mut blocks,
            instruction,
            lifted,
            has_exception_edges,
        )?;
    }

    add_normal_edges(&mut builder, body, analysis, &blocks)?;
    add_exception_edges(&mut builder, file, body, &variables, &blocks)?;
    add_payload_provenance(&mut builder, body, &body_analysis, &blocks)?;
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
        ranges.insert(instruction.offset(), range);
        if matches!(instruction.data(), InstructionData::Operation { .. }) {
            let block = builder.new_block(format!("dex_{:04x}", instruction.offset()));
            builder.map_entity(range, EntityId::Block(block))?;
            blocks.insert(instruction.offset(), block);
        }
    }
    Ok(NativeBlocks {
        normal_sources: blocks.clone(),
        blocks,
        throw_sites: BTreeMap::new(),
        ranges,
    })
}

#[allow(clippy::too_many_arguments)]
fn append_lifted(
    builder: &mut FunctionBuilder,
    variables: &StateVariables,
    blocks: &mut NativeBlocks,
    instruction: &llil::Instruction,
    mut lifted: LiftedInstruction,
    has_exception_edges: bool,
) -> Result<()> {
    let range = blocks.ranges[&instruction.offset];
    let block = blocks.blocks[&instruction.offset];
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
    let may_throw = match &instruction.kind {
        InstructionKind::Operation(operation) => operation.semantics.may_throw,
        InstructionKind::Payload(_) => false,
    };
    let mut ids = Vec::with_capacity(lifted.steps.len());
    for (index, step) in lifted.steps.into_iter().enumerate() {
        ids.push(builder.append_instruction(
            block,
            step.operation,
            step.uses,
            step.defs,
            index == lifted.throw_step && may_throw,
            Some(range),
        )?);
    }
    let primary = ids[lifted.throw_step];
    blocks.throw_sites.insert(instruction.offset, primary);

    if !commit_defs.is_empty() || !normal_steps.is_empty() {
        let commit = builder.new_block(format!("dex_{:04x}_commit", instruction.offset));
        builder.map_entity(range, EntityId::Block(commit))?;
        builder.add_edge(
            block,
            commit,
            EdgeMetadata::ordinary(EdgeRole::Commit),
            Some(range),
        )?;
        if !commit_defs.is_empty() {
            builder.append_instruction(
                commit,
                if commit_defs.len() == 1 {
                    Operation::Copy
                } else {
                    Operation::ParallelCopy
                },
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
        blocks.normal_sources.insert(instruction.offset, commit);
    }
    Ok(())
}

fn add_normal_edges(
    builder: &mut FunctionBuilder,
    body: &llil::Body,
    analysis: &RegisterAnalysis,
    blocks: &NativeBlocks,
) -> Result<()> {
    let operations = body
        .instructions
        .iter()
        .filter_map(|instruction| match &instruction.kind {
            InstructionKind::Operation(operation) => Some((instruction.offset, &operation.kind)),
            InstructionKind::Payload(_) => None,
        })
        .collect::<BTreeMap<_, _>>();
    for edge in analysis.flow().edges() {
        if matches!(edge.kind, FlowEdgeKind::Exception(_)) {
            continue;
        }
        let source = blocks.normal_sources[&edge.source];
        let target = blocks
            .blocks
            .get(&edge.target)
            .copied()
            .ok_or(Error::MissingTarget {
                source_offset: edge.source,
                target: edge.target,
            })?;
        let role = match (operations[&edge.source], edge.kind) {
            (OperationKind::Switch, FlowEdgeKind::FallThrough) => EdgeRole::SwitchDefault,
            (
                OperationKind::BranchPair(_) | OperationKind::BranchZero(_),
                FlowEdgeKind::FallThrough,
            ) => EdgeRole::BranchFalse,
            (OperationKind::BranchPair(_) | OperationKind::BranchZero(_), FlowEdgeKind::Branch) => {
                EdgeRole::BranchTrue
            }
            (_, FlowEdgeKind::SwitchCase(key)) => EdgeRole::SwitchCase(i64::from(key)),
            (_, FlowEdgeKind::FallThrough) => EdgeRole::FallThrough,
            (_, FlowEdgeKind::Branch) => EdgeRole::Jump,
            (_, FlowEdgeKind::Exception(_)) => unreachable!(),
        };
        builder.add_edge(
            source,
            target,
            EdgeMetadata::ordinary(role),
            Some(blocks.ranges[&edge.source]),
        )?;
    }
    Ok(())
}

fn add_exception_edges(
    builder: &mut FunctionBuilder,
    file: &DexFile,
    body: &llil::Body,
    variables: &StateVariables,
    blocks: &NativeBlocks,
) -> Result<()> {
    let mut handler_order = 0u32;
    for (try_index, try_block) in body.tries.iter().enumerate() {
        let protected = protected_range(try_block)?;
        let mut region_handlers = Vec::with_capacity(try_block.handlers.len());
        for (handler_index, handler) in try_block.handlers.iter().enumerate() {
            let catch = catch_type(file, *handler)?;
            let landing = builder.new_block(format!(
                "dex_{:04x}_handler_{try_index}_{handler_index}",
                handler.address
            ));
            let handler_range =
                blocks
                    .ranges
                    .get(&handler.address)
                    .copied()
                    .ok_or(Error::MissingTarget {
                        source_offset: handler.address,
                        target: handler.address,
                    })?;
            builder.map_entity(handler_range, EntityId::Block(landing))?;
            builder.append_instruction(
                landing,
                Operation::CaughtException(catch.clone()),
                vec![],
                vec![TypedVariable::new(
                    variables.exception,
                    catch_value_type(&catch),
                )],
                false,
                Some(handler_range),
            )?;
            builder.add_edge(
                landing,
                blocks.blocks[&handler.address],
                EdgeMetadata::ordinary(EdgeRole::FallThrough),
                Some(handler_range),
            )?;

            for instruction in &body.instructions {
                let InstructionKind::Operation(operation) = &instruction.kind else {
                    continue;
                };
                if protects(try_block, instruction.offset) && operation.semantics.may_throw {
                    builder.add_edge(
                        blocks.blocks[&instruction.offset],
                        landing,
                        EdgeMetadata::exceptional(
                            EdgeRole::Exception {
                                catch: catch.clone(),
                                handler_order,
                                protected,
                            },
                            blocks.throw_sites[&instruction.offset],
                        ),
                        Some(blocks.ranges[&instruction.offset]),
                    )?;
                }
            }
            region_handlers.push({
                use ::mlil::cfglib::{Handler, HandlerBody, HandlerKind};
                Handler {
                    entry: landing,
                    body: HandlerBody::Unknown,
                    kind: match catch {
                        CatchType::Any => HandlerKind::CatchAll,
                        CatchType::Type(_) => HandlerKind::Catch,
                    },
                }
            });
            handler_order = handler_order
                .checked_add(1)
                .ok_or_else(|| Error::unsupported(handler.address, "handler order exceeds u32"))?;
        }
        add_exception_region(builder, try_block, blocks, region_handlers)?;
    }
    Ok(())
}

/// Registers one exception region per try block, handlers in dispatch
/// order. Handler-body extents stay unknown in the canonical function;
/// presentation derives them on its own graph when it needs structure.
fn add_exception_region(
    builder: &mut FunctionBuilder,
    try_block: &TryBlock,
    blocks: &NativeBlocks,
    handlers: Vec<::mlil::cfglib::Handler>,
) -> Result<()> {
    use ::mlil::cfglib::{Region, RegionId};

    let mut protected = std::collections::BTreeSet::new();
    for (&offset, &block) in &blocks.blocks {
        if protects(try_block, offset) {
            protected.insert(block);
            // The commit continuation of a protected instruction belongs
            // to the same protected extent.
            protected.insert(blocks.normal_sources[&offset]);
        }
    }
    if protected.is_empty() {
        return Ok(());
    }
    builder.add_region(Region {
        id: RegionId::from_raw(0),
        protected_blocks: protected,
        handlers,
        parent: None,
    })?;
    Ok(())
}

fn add_payload_provenance(
    builder: &mut FunctionBuilder,
    body: &llil::Body,
    analysis: &crate::analysis::BodyAnalysis,
    blocks: &NativeBlocks,
) -> Result<()> {
    for instruction in &body.instructions {
        let InstructionKind::Operation(_) = &instruction.kind else {
            continue;
        };
        let Some(link) = analysis
            .instruction(instruction.offset)
            .and_then(|facts| facts.payload)
        else {
            continue;
        };
        let range =
            blocks
                .ranges
                .get(&link.payload_offset)
                .copied()
                .ok_or(Error::MissingTarget {
                    source_offset: instruction.offset,
                    target: link.payload_offset,
                })?;
        let instruction_id = blocks.throw_sites[&instruction.offset];
        builder.map_entity(range, EntityId::Instruction(instruction_id))?;
    }
    Ok(())
}

fn protects(try_block: &TryBlock, offset: u32) -> bool {
    let end = try_block
        .start_address
        .saturating_add(u32::from(try_block.instruction_count));
    offset >= try_block.start_address && offset < end
}

fn protected_range(try_block: &TryBlock) -> Result<AddressRange> {
    let end = try_block
        .start_address
        .checked_add(u32::from(try_block.instruction_count))
        .ok_or_else(|| Error::unsupported(try_block.start_address, "try range overflow"))?;
    Ok(AddressRange::new(
        CodeAddress::from(try_block.start_address),
        CodeAddress::from(end),
    ))
}

fn catch_type(file: &DexFile, handler: CatchHandler) -> Result<CatchType> {
    handler.exception_type.map_or(Ok(CatchType::Any), |index| {
        Ok(CatchType::Type(file.type_descriptor(index)?.to_owned()))
    })
}

fn catch_value_type(catch: &CatchType) -> ValueType {
    match catch {
        CatchType::Any => ValueType::Reference(Some(JAVA_LANG_THROWABLE_DESCRIPTOR.to_owned())),
        CatchType::Type(descriptor) => ValueType::Reference(Some(descriptor.clone())),
    }
}

fn instruction_range(instruction: &NativeInstruction) -> Result<AddressRange> {
    let size = instruction
        .code_units()
        .ok_or_else(|| Error::unsupported(instruction.offset(), "instruction width exceeds u32"))?;
    let end = instruction
        .offset()
        .checked_add(size)
        .ok_or_else(|| Error::unsupported(instruction.offset(), "instruction range overflow"))?;
    Ok(AddressRange::new(
        CodeAddress::from(instruction.offset()),
        CodeAddress::from(end),
    ))
}

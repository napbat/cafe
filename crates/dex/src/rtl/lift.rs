//! Direct verified Dalvik LLIL to Dalvik RTL graph construction.

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
    FlowEdgeKind, ReferenceHierarchy, RegisterAnalysis, analyze_body,
    analyze_method_registers_with_hierarchy,
};
use crate::file::{CatchHandler, CodeItem, DexFile, EncodedMethod, TryBlock};
use crate::instruction::{Instruction as NativeInstruction, InstructionData};
use crate::llil::{self, InstructionKind, OperationKind};
use crate::mlil::instruction::{LiftedInstruction, Step, lift_instruction};
use crate::mlil::state::{StateVariables, VariableAllocator};
use crate::mlil::{Error, Result};

use super::{DexRtlDialect, DexStorage, EdgeMetadata, Function, RegisterConstraint};

const JAVA_LANG_THROWABLE_DESCRIPTOR: &str = "Ljava/lang/Throwable;";

struct NativeBlocks {
    blocks: BTreeMap<u32, BlockId>,
    normal_sources: BTreeMap<u32, BlockId>,
    throw_sites: BTreeMap<u32, StatementId>,
    ranges: BTreeMap<u32, AddressRange>,
}

#[derive(Default)]
struct Variables {
    storage: Vec<DexStorage>,
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
                format: BinaryFormat::Dex,
                storage: SourceStorage::DexRegister(index),
            }) => DexStorage::SourceRegister(index),
            Some(NativeVariable {
                format: BinaryFormat::Dex,
                storage: SourceStorage::DexResult,
            }) => DexStorage::SourceResult,
            Some(NativeVariable {
                format: BinaryFormat::Dex,
                storage: SourceStorage::DexException,
            }) => DexStorage::SourceException,
            Some(_) => {
                return Err(mlil::Error::InvalidConstruction(
                    "Dalvik semantic state names non-DEX native storage".into(),
                ));
            }
            None => {
                let temporary = self.temporary;
                self.temporary = self.temporary.checked_add(1).ok_or_else(|| {
                    mlil::Error::InvalidConstruction(
                        "Dalvik RTL temporary space exceeds u32".into(),
                    )
                })?;
                DexStorage::Temporary(temporary)
            }
        };
        let raw = u32::try_from(self.storage.len()).map_err(|_| {
            mlil::Error::InvalidConstruction("Dalvik semantic variable space exceeds u32".into())
        })?;
        self.storage.push(storage);
        Ok(VariableId::from_raw(raw))
    }
}

impl Variables {
    fn storage(&self, variable: VariableId) -> mlil::Result<DexStorage> {
        self.storage.get(variable.index()).copied().ok_or_else(|| {
            mlil::Error::InvalidConstruction(format!(
                "Dalvik semantic variable {variable} has no RTL storage"
            ))
        })
    }

    fn place(&self, variable: VariableId) -> mlil::Result<Place<DexRtlDialect>> {
        Ok(Place {
            storage: self.storage(variable)?,
            lanes: vec![0],
        })
    }

    fn read(&self, variable: &TypedVariable) -> mlil::Result<Expr<DexRtlDialect>> {
        Ok(Expr::Read {
            storage: self.storage(variable.variable)?,
            lanes: vec![0],
            scalar: <RegisterConstraint as mlil::rtl::ValueConstraint>::from_value_type(
                variable.value_type.clone(),
            ),
        })
    }
}

pub(super) fn lift_method_with_hierarchy(
    file: &DexFile,
    declaration: &EncodedMethod,
    hierarchy: &dyn ReferenceHierarchy,
) -> Result<Option<Function>> {
    let Some(code) = &declaration.code else {
        return Ok(None);
    };
    let analysis = analyze_method_registers_with_hierarchy(file, declaration, hierarchy)?
        .ok_or_else(|| Error::Unsupported {
            offset: 0,
            feature: "register analysis omitted an executable DEX method".into(),
        })?;
    let body = llil::Body::from_code(code)?;
    lift_analyzed_body(file, declaration, &body, code, &analysis).map(Some)
}

pub(super) fn lift_body_with_hierarchy(
    file: &DexFile,
    declaration: &EncodedMethod,
    body: &llil::Body,
    hierarchy: &dyn ReferenceHierarchy,
) -> Result<Function> {
    let code = body.to_code()?;
    let mut analyzed_declaration = declaration.clone();
    analyzed_declaration.code = Some(code.clone());
    let analysis = analyze_method_registers_with_hierarchy(file, &analyzed_declaration, hierarchy)?
        .ok_or_else(|| Error::Unsupported {
            offset: 0,
            feature: "register analysis omitted an executable DEX LLIL body".into(),
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
        return Err(Error::Unsupported {
            offset: 0,
            feature: "Dalvik LLIL/native instruction counts disagree after verification".into(),
        });
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
    let mut builder = FunctionBuilder::<DexRtlDialect>::new(coordinate);
    let mut storage = Variables::default();
    let variables = StateVariables::declare(&mut storage, file, declaration, code)?;
    let parameters = variables
        .parameters()
        .iter()
        .map(|&variable| storage.place(variable))
        .collect::<mlil::Result<Vec<_>>>()?;
    builder.set_signature(Signature::new(parameters, variables.returns().to_vec()))?;
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
    )?;

    for (native, instruction) in code.instructions.iter().zip(&body.instructions) {
        let InstructionKind::Operation(operation) = &instruction.kind else {
            continue;
        };
        let offset = instruction.offset;
        let lifted = lift_instruction(
            &mut storage,
            &variables,
            file,
            native,
            instruction,
            analysis.entry_frame(offset),
            analysis.exit_frame(offset),
            body,
        )?;
        append_lifted(
            &mut builder,
            &storage,
            &mut blocks,
            instruction,
            lifted,
            operation.semantics.may_throw,
        )?;
    }

    add_normal_edges(&mut builder, body, analysis, &blocks)?;
    add_exception_edges(&mut builder, &storage, file, body, &variables, &blocks)?;
    add_payload_provenance(&mut builder, body, &body_analysis, &blocks)?;
    Ok(builder.finish()?)
}

fn allocate_native_blocks(
    builder: &mut FunctionBuilder<DexRtlDialect>,
    native: &[NativeInstruction],
) -> Result<NativeBlocks> {
    let mut blocks = BTreeMap::new();
    let mut ranges = BTreeMap::new();
    for instruction in native {
        let range = instruction_range(instruction)?;
        ranges.insert(instruction.offset(), range);
        if matches!(instruction.data(), InstructionData::Operation { .. }) {
            let block = builder.new_block(format!("dex_{:04x}", instruction.offset()));
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

fn append_lifted(
    builder: &mut FunctionBuilder<DexRtlDialect>,
    storage: &Variables,
    blocks: &mut NativeBlocks,
    instruction: &llil::Instruction,
    mut lifted: LiftedInstruction,
    may_throw: bool,
) -> Result<()> {
    let range = blocks.ranges[&instruction.offset];
    let block = blocks.blocks[&instruction.offset];
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
        .insert(instruction.offset, ids[lifted.throw_step]);

    if !normal_steps.is_empty() {
        let continuation = builder.new_block(format!("dex_{:04x}_continue", instruction.offset));
        builder.add_edge(
            block,
            continuation,
            EdgeMetadata::ordinary(EdgeRole::Commit),
        )?;
        for step in normal_steps {
            append_step(builder, storage, continuation, step, false, range)?;
        }
        blocks
            .normal_sources
            .insert(instruction.offset, continuation);
    }
    Ok(())
}

fn append_step(
    builder: &mut FunctionBuilder<DexRtlDialect>,
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
    let statement = mlil::rtl::lower_operation::<DexRtlDialect>(
        step.operation,
        operands,
        definitions,
        metadata.effects,
        metadata.may_throw,
    )?;
    Ok(builder.append(block, statement, Some(range))?)
}

fn add_normal_edges(
    builder: &mut FunctionBuilder<DexRtlDialect>,
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
        builder.add_edge(source, target, EdgeMetadata::ordinary(role))?;
    }
    Ok(())
}

fn add_exception_edges(
    builder: &mut FunctionBuilder<DexRtlDialect>,
    storage: &Variables,
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
            append_step(
                builder,
                storage,
                landing,
                Step {
                    operation: Operation::CaughtException(catch.clone()),
                    uses: vec![],
                    defs: vec![TypedVariable::new(
                        variables.exception(),
                        catch_value_type(&catch),
                    )],
                },
                false,
                handler_range,
            )?;
            builder.add_edge(
                landing,
                blocks.blocks[&handler.address],
                EdgeMetadata::ordinary(EdgeRole::FallThrough),
            )?;

            for instruction in &body.instructions {
                let InstructionKind::Operation(operation) = &instruction.kind else {
                    continue;
                };
                if protects(try_block, instruction.offset) && operation.semantics.may_throw {
                    builder.add_edge(
                        blocks.blocks[&instruction.offset],
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
            region_handlers.push(cfglib::Handler {
                entry: landing,
                body: cfglib::HandlerBody::Unknown,
                kind: match catch {
                    CatchType::Any => cfglib::HandlerKind::CatchAll,
                    CatchType::Type(_) => cfglib::HandlerKind::Catch,
                },
            });
            handler_order = handler_order
                .checked_add(1)
                .ok_or_else(|| Error::Unsupported {
                    offset: handler.address,
                    feature: "handler order exceeds u32".into(),
                })?;
        }
        add_exception_region(builder, try_block, blocks, region_handlers)?;
    }
    Ok(())
}

fn add_exception_region(
    builder: &mut FunctionBuilder<DexRtlDialect>,
    try_block: &crate::file::TryBlock,
    blocks: &NativeBlocks,
    handlers: Vec<cfglib::Handler>,
) -> Result<()> {
    let mut protected_blocks = BTreeSet::new();
    for (&offset, &block) in &blocks.blocks {
        if protects(try_block, offset) {
            protected_blocks.insert(block);
            protected_blocks.insert(blocks.normal_sources[&offset]);
        }
    }
    if protected_blocks.is_empty() {
        return Ok(());
    }
    builder.add_region(cfglib::Region {
        id: cfglib::RegionId::from_raw(0),
        protected_blocks,
        handlers,
        parent: None,
    })?;
    Ok(())
}

fn add_payload_provenance(
    builder: &mut FunctionBuilder<DexRtlDialect>,
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
        builder.map_statement(range, blocks.throw_sites[&instruction.offset])?;
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
        .ok_or_else(|| Error::Unsupported {
            offset: try_block.start_address,
            feature: "try range overflow".into(),
        })?;
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

fn catch_value_type(catch: &CatchType) -> mlil::ValueType {
    match catch {
        CatchType::Any => {
            mlil::ValueType::Reference(Some(JAVA_LANG_THROWABLE_DESCRIPTOR.to_owned()))
        }
        CatchType::Type(descriptor) => mlil::ValueType::Reference(Some(descriptor.clone())),
    }
}

fn instruction_range(instruction: &NativeInstruction) -> Result<AddressRange> {
    let size = instruction.code_units().ok_or_else(|| Error::Unsupported {
        offset: instruction.offset(),
        feature: "instruction width exceeds u32".into(),
    })?;
    let end = instruction
        .offset()
        .checked_add(size)
        .ok_or_else(|| Error::Unsupported {
            offset: instruction.offset(),
            feature: "instruction range overflow".into(),
        })?;
    Ok(AddressRange::new(
        CodeAddress::from(instruction.offset()),
        CodeAddress::from(end),
    ))
}

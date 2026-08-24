//! Operation-level control flow with typed exceptional edges.

use std::collections::{BTreeMap, BTreeSet};

use crate::file::CodeItem;
use crate::instruction::{InstructionData, Opcode, Operands};
use crate::{Error, Result};

use super::{BodyAnalysis, ControlFlow, FlowEdge, FlowEdgeKind, analyze_body};

/// Builds exception-aware control flow for one structurally valid code item.
///
/// Data payloads are associations rather than executable nodes. Handler edges
/// originate only at protected operations whose semantics may throw.
///
/// # Errors
///
/// Returns an error for malformed body relationships or fallthrough into data.
pub fn control_flow(code: &CodeItem) -> Result<ControlFlow> {
    let body = analyze_body(code)?;
    build_control_flow(code, &body)
}

pub(super) fn build_control_flow(code: &CodeItem, body: &BodyAnalysis) -> Result<ControlFlow> {
    let operations = code
        .instructions
        .iter()
        .filter_map(|instruction| match instruction.data() {
            InstructionData::Operation { .. } => Some(instruction.offset()),
            InstructionData::PackedSwitchPayload(_)
            | InstructionData::SparseSwitchPayload(_)
            | InstructionData::ArrayDataPayload(_) => None,
        })
        .collect::<Vec<_>>();
    let entry = *operations.first().ok_or_else(|| {
        Error::invalid_instruction(0, "method body contains no executable instruction")
    })?;
    let instruction_by_offset = code
        .instructions
        .iter()
        .map(|instruction| (instruction.offset(), instruction))
        .collect::<BTreeMap<_, _>>();
    let operation_set = operations.iter().copied().collect::<BTreeSet<_>>();
    let mut edges = Vec::new();

    for &source in &operations {
        let instruction = instruction_by_offset[&source];
        let InstructionData::Operation { opcode, operands } = instruction.data() else {
            unreachable!("operation offsets were filtered above");
        };
        let fallthrough = source
            .checked_add(opcode.format().code_units())
            .ok_or_else(|| Error::invalid_instruction(source, "fallthrough address overflowed"))?;
        if opcode.is_switch() {
            push_fallthrough(source, fallthrough, &operation_set, body, &mut edges)?;
            push_switch_edges(
                source,
                *opcode,
                operands,
                &instruction_by_offset,
                &mut edges,
            )?;
        } else if opcode.is_conditional_branch() {
            push_fallthrough(source, fallthrough, &operation_set, body, &mut edges)?;
            edges.push(FlowEdge {
                source,
                target: branch_target(operands, source)?,
                kind: FlowEdgeKind::Branch,
            });
        } else if opcode.is_unconditional_branch() {
            edges.push(FlowEdge {
                source,
                target: branch_target(operands, source)?,
                kind: FlowEdgeKind::Branch,
            });
        } else if !opcode.is_return() && *opcode != Opcode::Throw {
            push_fallthrough(source, fallthrough, &operation_set, body, &mut edges)?;
        }
        push_exception_edges(code, body, source, &mut edges);
    }

    reject_normal_entry_to_move_exception(&instruction_by_offset, &edges)?;
    Ok(ControlFlow {
        entry,
        nodes: operations,
        edges,
    })
}

fn push_fallthrough(
    source: u32,
    target: u32,
    operations: &BTreeSet<u32>,
    body: &BodyAnalysis,
    edges: &mut Vec<FlowEdge>,
) -> Result<()> {
    if operations.contains(&target) {
        edges.push(FlowEdge {
            source,
            target,
            kind: FlowEdgeKind::FallThrough,
        });
        Ok(())
    } else {
        let reason = if target == body.stream_end() {
            "execution falls off the end of the method"
        } else {
            "execution falls through into a data payload"
        };
        Err(Error::invalid_instruction(source, reason))
    }
}

fn push_switch_edges(
    source: u32,
    opcode: Opcode,
    operands: &Operands,
    instructions: &BTreeMap<u32, &crate::instruction::Instruction>,
    edges: &mut Vec<FlowEdge>,
) -> Result<()> {
    let payload_offset = branch_target(operands, source)?;
    let payload = instructions
        .get(&payload_offset)
        .ok_or_else(|| Error::invalid_instruction(source, "switch payload target is missing"))?;
    match (opcode, payload.data()) {
        (Opcode::PackedSwitch, InstructionData::PackedSwitchPayload(payload)) => {
            for (position, &target) in payload.targets.iter().enumerate() {
                let key_delta = i32::try_from(position).map_err(|_| {
                    Error::invalid_instruction(source, "packed-switch key overflowed")
                })?;
                let key = payload.first_key.checked_add(key_delta).ok_or_else(|| {
                    Error::invalid_instruction(source, "packed-switch key overflowed")
                })?;
                edges.push(FlowEdge {
                    source,
                    target,
                    kind: FlowEdgeKind::SwitchCase(key),
                });
            }
        }
        (Opcode::SparseSwitch, InstructionData::SparseSwitchPayload(payload)) => {
            for (&key, &target) in payload.keys.iter().zip(&payload.targets) {
                edges.push(FlowEdge {
                    source,
                    target,
                    kind: FlowEdgeKind::SwitchCase(key),
                });
            }
        }
        _ => {
            return Err(Error::invalid_instruction(
                source,
                "switch target selects an incompatible payload",
            ));
        }
    }
    Ok(())
}

fn push_exception_edges(
    code: &CodeItem,
    body: &BodyAnalysis,
    source: u32,
    edges: &mut Vec<FlowEdge>,
) {
    if !body
        .instruction(source)
        .is_some_and(|facts| facts.semantics.may_throw)
    {
        return;
    }
    for protected in &code.tries {
        let end = protected
            .start_address
            .saturating_add(u32::from(protected.instruction_count));
        if source < protected.start_address || source >= end {
            continue;
        }
        for handler in &protected.handlers {
            edges.push(FlowEdge {
                source,
                target: handler.address,
                kind: FlowEdgeKind::Exception(handler.exception_type),
            });
        }
    }
}

fn branch_target(operands: &Operands, source: u32) -> Result<u32> {
    match operands {
        Operands::Branch { target }
        | Operands::RegisterBranch { target, .. }
        | Operands::RegistersBranch { target, .. } => Ok(*target),
        _ => Err(Error::invalid_instruction(
            source,
            "branch target is missing",
        )),
    }
}

fn reject_normal_entry_to_move_exception(
    instructions: &BTreeMap<u32, &crate::instruction::Instruction>,
    edges: &[FlowEdge],
) -> Result<()> {
    for edge in edges {
        if matches!(edge.kind, FlowEdgeKind::Exception(_)) {
            continue;
        }
        let target = instructions[&edge.target];
        if matches!(
            target.data(),
            InstructionData::Operation {
                opcode: Opcode::MoveException,
                ..
            }
        ) {
            return Err(Error::invalid_instruction(
                edge.target,
                "move-exception is reachable through ordinary control flow",
            ));
        }
    }
    Ok(())
}

//! Instruction-level JVM control flow with exception-table edges.

use std::collections::BTreeSet;

use crate::bytecode::{Instruction, Opcode, Operand};
use crate::classfile::ExceptionHandler;
use crate::{Error, Result};

use super::model::{ControlFlow, FlowEdge, FlowEdgeKind};

pub(super) fn build_control_flow(
    instructions: &[Instruction],
    handlers: &[ExceptionHandler],
) -> Result<ControlFlow> {
    let entry = instructions
        .first()
        .map(|instruction| instruction.offset)
        .ok_or_else(|| Error::invalid_bytecode(0, "method code is empty"))?;
    let nodes = instructions
        .iter()
        .map(|instruction| instruction.offset)
        .collect::<Vec<_>>();
    let node_set = nodes.iter().copied().collect::<BTreeSet<_>>();
    let mut edges = Vec::new();

    for instruction in instructions {
        let next = instruction
            .offset
            .checked_add(instruction.size)
            .ok_or_else(|| {
                Error::invalid_bytecode(instruction.offset, "fallthrough offset overflowed")
            })?;
        match instruction.opcode {
            opcode if opcode.is_conditional_branch() => {
                push_fallthrough(instruction.offset, next, &node_set, &mut edges)?;
                edges.push(FlowEdge {
                    source: instruction.offset,
                    target: branch_target(instruction)?,
                    kind: FlowEdgeKind::Branch,
                });
            }
            Opcode::Goto | Opcode::GotoW | Opcode::Jsr | Opcode::JsrW => {
                edges.push(FlowEdge {
                    source: instruction.offset,
                    target: branch_target(instruction)?,
                    kind: FlowEdgeKind::Branch,
                });
            }
            Opcode::TableSwitch | Opcode::LookupSwitch => {
                push_switch_edges(instruction, &mut edges)?;
            }
            opcode if opcode.is_return() || opcode == Opcode::AThrow || opcode == Opcode::Ret => {}
            _ => push_fallthrough(instruction.offset, next, &node_set, &mut edges)?,
        }
        for handler in handlers {
            if instruction.offset >= usize::from(handler.start_pc)
                && instruction.offset < usize::from(handler.end_pc)
            {
                edges.push(FlowEdge {
                    source: instruction.offset,
                    target: usize::from(handler.handler_pc),
                    kind: FlowEdgeKind::Exception {
                        catch_type: handler.catch_type,
                    },
                });
            }
        }
    }

    ControlFlow::build(entry, &nodes, edges)
}

fn push_fallthrough(
    source: usize,
    target: usize,
    nodes: &BTreeSet<usize>,
    edges: &mut Vec<FlowEdge>,
) -> Result<()> {
    if !nodes.contains(&target) {
        return Err(Error::invalid_bytecode(
            source,
            "execution falls off the end of the method",
        ));
    }
    edges.push(FlowEdge {
        source,
        target,
        kind: FlowEdgeKind::FallThrough,
    });
    Ok(())
}

fn push_switch_edges(instruction: &Instruction, edges: &mut Vec<FlowEdge>) -> Result<()> {
    match &instruction.operand {
        Operand::TableSwitch {
            default, targets, ..
        } => {
            edges.push(FlowEdge {
                source: instruction.offset,
                target: absolute_target(*default, instruction.offset)?,
                kind: FlowEdgeKind::Branch,
            });
            for &target in targets {
                edges.push(FlowEdge {
                    source: instruction.offset,
                    target: absolute_target(target, instruction.offset)?,
                    kind: FlowEdgeKind::Branch,
                });
            }
        }
        Operand::LookupSwitch { default, pairs } => {
            edges.push(FlowEdge {
                source: instruction.offset,
                target: absolute_target(*default, instruction.offset)?,
                kind: FlowEdgeKind::Branch,
            });
            for &(_, target) in pairs {
                edges.push(FlowEdge {
                    source: instruction.offset,
                    target: absolute_target(target, instruction.offset)?,
                    kind: FlowEdgeKind::Branch,
                });
            }
        }
        _ => {
            return Err(Error::invalid_bytecode(
                instruction.offset,
                "switch instruction lacks its target table",
            ));
        }
    }
    Ok(())
}

fn branch_target(instruction: &Instruction) -> Result<usize> {
    let Operand::Branch(target) = instruction.operand else {
        return Err(Error::invalid_bytecode(
            instruction.offset,
            "branch instruction lacks its target",
        ));
    };
    absolute_target(target, instruction.offset)
}

fn absolute_target(target: i32, source: usize) -> Result<usize> {
    usize::try_from(target)
        .map_err(|_| Error::invalid_bytecode(source, "branch target is negative"))
}

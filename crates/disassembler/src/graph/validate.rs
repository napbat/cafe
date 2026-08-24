//! Consumer-semantic validation for shared Java-ecosystem control flow.

use std::fmt;

use cfglib::{BlockId, Cfg, EdgeId, EdgeKind, SemanticValidator};

use super::{ControlFlowEdge, ControlFlowEdgeRole, ExceptionHandlerIndex};
use crate::{CodeAddress, FunctionBody, Instruction, InstructionFlow};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ControlFlowViolation {
    NormalEdges {
        block: BlockId,
        expected: Vec<ControlFlowEdgeRole>,
        actual: Vec<ControlFlowEdgeRole>,
    },
    ExceptionEdges {
        block: BlockId,
        expected: Vec<ControlFlowEdgeRole>,
        actual: Vec<ControlFlowEdgeRole>,
    },
    ExceptionSourceNotIsolated {
        block: BlockId,
        instruction_count: usize,
    },
    SourceAddress {
        edge: EdgeId,
        expected: CodeAddress,
        actual: CodeAddress,
    },
    TargetAddress {
        edge: EdgeId,
        expected: CodeAddress,
        actual: CodeAddress,
    },
    EdgeKind {
        edge: EdgeId,
        expected: EdgeKind,
        actual: EdgeKind,
    },
}

impl fmt::Display for ControlFlowViolation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NormalEdges {
                block,
                expected,
                actual,
            } => write!(
                formatter,
                "block {block} normal edges differ: expected {expected:?}, got {actual:?}"
            ),
            Self::ExceptionEdges {
                block,
                expected,
                actual,
            } => write!(
                formatter,
                "block {block} exception edges differ: expected {expected:?}, got {actual:?}"
            ),
            Self::ExceptionSourceNotIsolated {
                block,
                instruction_count,
            } => write!(
                formatter,
                "exception source block {block} contains {instruction_count} instructions"
            ),
            Self::SourceAddress {
                edge,
                expected,
                actual,
            } => write!(
                formatter,
                "edge {edge} source address differs: expected {expected}, got {actual}"
            ),
            Self::TargetAddress {
                edge,
                expected,
                actual,
            } => write!(
                formatter,
                "edge {edge} target address differs: expected {expected}, got {actual}"
            ),
            Self::EdgeKind {
                edge,
                expected,
                actual,
            } => write!(
                formatter,
                "edge {edge} kind differs: expected {expected}, got {actual}"
            ),
        }
    }
}

pub(crate) struct ControlFlowValidator<'body> {
    body: &'body FunctionBody,
}

impl<'body> ControlFlowValidator<'body> {
    pub(crate) const fn new(body: &'body FunctionBody) -> Self {
        Self { body }
    }
}

impl SemanticValidator<Instruction, ControlFlowEdge> for ControlFlowValidator<'_> {
    type Error = ControlFlowViolation;

    fn validate_block(
        &self,
        cfg: &Cfg<Instruction, ControlFlowEdge>,
        block: BlockId,
        errors: &mut Vec<Self::Error>,
    ) {
        let instructions = cfg.block(block).instructions();
        let Some(last) = instructions.last() else {
            return;
        };
        let has_next = cfg
            .blocks()
            .iter()
            .skip(block.index() + 1)
            .any(|candidate| !candidate.is_empty());
        let expected_normal = expected_normal_roles(last, has_next);
        let actual_normal = cfg
            .successor_edges(block)
            .iter()
            .map(|&edge| cfg.edge(edge).payload().role())
            .filter(|role| !role.is_exceptional())
            .cloned()
            .collect::<Vec<_>>();
        if actual_normal != expected_normal {
            errors.push(ControlFlowViolation::NormalEdges {
                block,
                expected: expected_normal,
                actual: actual_normal,
            });
        }

        let source = instructions[0].address;
        let expected_exception = self
            .body
            .exception_handlers
            .iter()
            .enumerate()
            .filter(|(_, handler)| handler.protected.contains(source))
            .map(|(index, handler)| ControlFlowEdgeRole::Exception {
                handler: ExceptionHandlerIndex::from_index(index),
                protected: handler.protected,
                catch: handler.catch.clone(),
            })
            .collect::<Vec<_>>();
        let actual_exception = cfg
            .successor_edges(block)
            .iter()
            .map(|&edge| cfg.edge(edge).payload().role())
            .filter(|role| role.is_exceptional())
            .cloned()
            .collect::<Vec<_>>();
        if !actual_exception.is_empty() && instructions.len() != 1 {
            errors.push(ControlFlowViolation::ExceptionSourceNotIsolated {
                block,
                instruction_count: instructions.len(),
            });
        }
        if actual_exception != expected_exception {
            errors.push(ControlFlowViolation::ExceptionEdges {
                block,
                expected: expected_exception,
                actual: actual_exception,
            });
        }
    }

    fn validate_edge(
        &self,
        cfg: &Cfg<Instruction, ControlFlowEdge>,
        edge_id: EdgeId,
        errors: &mut Vec<Self::Error>,
    ) {
        let edge = cfg.edge(edge_id);
        let source_block = cfg.block(edge.source());
        let target_block = cfg.block(edge.target());
        let expected_source = if edge.payload().is_exceptional() {
            source_block.instructions().first()
        } else {
            source_block.instructions().last()
        }
        .expect("constructed edges connect non-empty source blocks")
        .address;
        let expected_target = target_block
            .instructions()
            .first()
            .expect("constructed edges connect non-empty target blocks")
            .address;
        if edge.payload().source() != expected_source {
            errors.push(ControlFlowViolation::SourceAddress {
                edge: edge_id,
                expected: expected_source,
                actual: edge.payload().source(),
            });
        }
        if edge.payload().target() != expected_target {
            errors.push(ControlFlowViolation::TargetAddress {
                edge: edge_id,
                expected: expected_target,
                actual: edge.payload().target(),
            });
        }
        let expected_kind = expected_edge_kind(edge.payload().role());
        if edge.kind() != expected_kind {
            errors.push(ControlFlowViolation::EdgeKind {
                edge: edge_id,
                expected: expected_kind,
                actual: edge.kind(),
            });
        }
    }
}

fn expected_normal_roles(instruction: &Instruction, has_next: bool) -> Vec<ControlFlowEdgeRole> {
    let optional_fallthrough = |role| has_next.then_some(role).into_iter();
    match &instruction.flow {
        InstructionFlow::FallThrough => {
            optional_fallthrough(ControlFlowEdgeRole::Sequential).collect()
        }
        InstructionFlow::ConditionalBranch { .. } => {
            std::iter::once(ControlFlowEdgeRole::ConditionalTaken)
                .chain(optional_fallthrough(
                    ControlFlowEdgeRole::ConditionalFallThrough,
                ))
                .collect()
        }
        InstructionFlow::UnconditionalBranch { .. } => {
            vec![ControlFlowEdgeRole::DirectBranch]
        }
        InstructionFlow::Switch { cases, .. } => {
            std::iter::once(ControlFlowEdgeRole::SwitchDefault)
                .chain(
                    cases
                        .iter()
                        .map(|case| ControlFlowEdgeRole::SwitchCase { key: case.key }),
                )
                .collect()
        }
        InstructionFlow::SubroutineCall { .. } => {
            std::iter::once(ControlFlowEdgeRole::SubroutineCall)
                .chain(optional_fallthrough(
                    ControlFlowEdgeRole::SubroutineContinuation {
                        call_site: instruction.address,
                    },
                ))
                .collect()
        }
        InstructionFlow::Return | InstructionFlow::Throw | InstructionFlow::IndirectBranch => {
            Vec::new()
        }
    }
}

const fn expected_edge_kind(role: &ControlFlowEdgeRole) -> EdgeKind {
    match role {
        ControlFlowEdgeRole::Sequential => EdgeKind::Fallthrough,
        ControlFlowEdgeRole::ConditionalTaken => EdgeKind::ConditionalTrue,
        ControlFlowEdgeRole::ConditionalFallThrough => EdgeKind::ConditionalFalse,
        ControlFlowEdgeRole::DirectBranch => EdgeKind::Jump,
        ControlFlowEdgeRole::SwitchDefault | ControlFlowEdgeRole::SwitchCase { .. } => {
            EdgeKind::SwitchCase
        }
        ControlFlowEdgeRole::SubroutineCall => EdgeKind::Call,
        ControlFlowEdgeRole::SubroutineContinuation { .. } => EdgeKind::CallReturn,
        ControlFlowEdgeRole::Exception { .. } => EdgeKind::ExceptionUnwind,
    }
}

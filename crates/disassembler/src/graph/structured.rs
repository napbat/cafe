//! Conservative bridge from recovered native exception flow to cfglib lifting.

use cfglib::{
    AstNode, Cfg, ExtentPromotionStatus, HandlerRef, RegionId, promote_exclusive_extents,
};

use super::{ControlFlowEdge, ControlFlowGraph, HandlerExtentStatus, RecoveredExceptionModel};
use crate::Instruction;

/// Result of deciding whether one exception region can be structured safely.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StructuredRegionStatus {
    /// Every handler has an unambiguous, non-overlapping recovered extent.
    Promoted,
    /// One handler's extent has unresolved structural evidence.
    AmbiguousExtent {
        /// Handler that prevents promotion of the complete region.
        handler: HandlerRef,
    },
    /// Two handlers claim at least one common body block.
    ///
    /// Shared native handler code is valid, but cfglib's structured AST owns a
    /// distinct body per catch arm. The bridge therefore keeps the region
    /// unstructured instead of duplicating or dropping code.
    OverlappingHandlerExtents {
        /// First overlapping handler in stable identity order.
        first: HandlerRef,
        /// Second overlapping handler in stable identity order.
        second: HandlerRef,
    },
}

/// Promotion decision for one cfglib exception region.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct StructuredRegionDecision {
    /// Stable region identity in the derived graph.
    pub region: RegionId,
    /// Whether and why the region was promoted.
    pub status: StructuredRegionStatus,
}

/// Derived CFG whose safe recovered handler extents are available to cfglib's AST lifter.
///
/// The canonical [`ControlFlowGraph`] is never mutated. Promotion is atomic per
/// exception region because cfglib structures a try only when every handler
/// body is complete. Ambiguous extents and shared handler bodies remain
/// [`cfglib::HandlerBody::Unknown`]. Catch-all entries remain catch-all
/// entries; this bridge never infers source-level `finally` semantics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecoveredStructuredControlFlow {
    cfg: Cfg<Instruction, ControlFlowEdge>,
    exception_model: RecoveredExceptionModel,
    decisions: Vec<StructuredRegionDecision>,
}

impl RecoveredStructuredControlFlow {
    pub(crate) fn compute(graph: &ControlFlowGraph) -> Self {
        let exception_model = graph.recovered_exception_model();
        let mut cfg = graph.cfg().clone();
        let decisions = promote_exclusive_extents(&mut cfg, |handler| {
            let recovered = exception_model
                .handler_by_ref(handler)
                .expect("every cfglib handler retains a native identity");
            (recovered.extent.status() != HandlerExtentStatus::Ambiguous)
                .then(|| recovered.extent.blocks.clone())
        })
        .into_iter()
        .map(|decision| StructuredRegionDecision {
            region: decision.region,
            status: match decision.status {
                ExtentPromotionStatus::Promoted => StructuredRegionStatus::Promoted,
                ExtentPromotionStatus::AmbiguousExtent { handler } => {
                    StructuredRegionStatus::AmbiguousExtent { handler }
                }
                ExtentPromotionStatus::OverlappingExtents { first, second } => {
                    StructuredRegionStatus::OverlappingHandlerExtents { first, second }
                }
            },
        })
        .collect();
        Self {
            cfg,
            exception_model,
            decisions,
        }
    }

    /// Returns the derived CFG used for structured lifting.
    #[must_use]
    pub const fn cfg(&self) -> &Cfg<Instruction, ControlFlowEdge> {
        &self.cfg
    }

    /// Returns the exact and recovered EH evidence used for promotion.
    #[must_use]
    pub const fn exception_model(&self) -> &RecoveredExceptionModel {
        &self.exception_model
    }

    /// Returns promotion decisions in cfglib region order.
    #[must_use]
    pub fn decisions(&self) -> &[StructuredRegionDecision] {
        &self.decisions
    }

    /// Whether a region received complete handler extents in the derived CFG.
    #[must_use]
    pub fn is_promoted(&self, region: RegionId) -> bool {
        self.decisions.iter().any(|decision| {
            decision.region == region && decision.status == StructuredRegionStatus::Promoted
        })
    }

    /// Lifts the derived graph with cfglib's structured AST reconstruction.
    #[must_use]
    pub fn lift(&self) -> AstNode<Instruction> {
        cfglib::lift(&self.cfg)
    }
}

#[cfg(test)]
mod tests {
    use cfglib::HandlerBody;

    use super::*;
    use crate::{
        AddressRange, AddressUnit, CatchType, CodeAddress, CodeSize, ExceptionHandler,
        FunctionBody, InstructionFlow,
    };

    const ONE_UNIT: CodeSize = CodeSize::new(1);

    fn instruction(address: u32, flow: InstructionFlow) -> Instruction {
        Instruction::new(
            CodeAddress::from(address),
            ONE_UNIT,
            0,
            "test",
            Vec::new(),
            flow,
        )
    }

    fn handler(start: u32, end: u32, entry: u32, catch: CatchType) -> ExceptionHandler {
        ExceptionHandler {
            protected: AddressRange::new(CodeAddress::from(start), CodeAddress::from(end)),
            handler: CodeAddress::from(entry),
            catch,
        }
    }

    fn contains_try(node: &AstNode<Instruction>) -> bool {
        match node {
            AstNode::TryCatch { .. } => true,
            AstNode::Sequence { body }
            | AstNode::Label { body, .. }
            | AstNode::Loop { body, .. }
            | AstNode::Guarded { body, .. } => body.iter().any(contains_try),
            AstNode::IfThenElse {
                then_body,
                else_body,
                ..
            } => then_body.iter().chain(else_body).any(contains_try),
            AstNode::Switch {
                cases,
                default_body,
                ..
            } => cases
                .iter()
                .flat_map(|case| &case.body)
                .chain(default_body)
                .any(contains_try),
            AstNode::Block { .. }
            | AstNode::Return { .. }
            | AstNode::Break { .. }
            | AstNode::Continue { .. }
            | AstNode::Goto { .. } => false,
        }
    }

    #[test]
    fn promotes_only_a_derived_graph_and_lifts_an_isolated_handler() {
        let body = FunctionBody::new(
            AddressUnit::Byte,
            vec![
                instruction(0, InstructionFlow::Return),
                instruction(1, InstructionFlow::Throw),
            ],
            vec![handler(0, 1, 1, CatchType::Type("example/Error".into()))],
        );
        let graph = body.control_flow_graph().unwrap();
        let derived = graph.recovered_structured_control_flow();
        let handler_ref = graph
            .exception_handler_ref(super::super::ExceptionHandlerIndex::from_index(0))
            .unwrap();

        assert_eq!(
            graph.cfg().handler(handler_ref).unwrap().body,
            HandlerBody::Unknown
        );
        assert!(derived.cfg().handler(handler_ref).unwrap().body.is_known());
        assert!(derived.is_promoted(handler_ref.region()));
        assert!(contains_try(&derived.lift()));
    }

    #[test]
    fn keeps_ambiguous_and_shared_handler_bodies_unstructured() {
        let ambiguous = FunctionBody::new(
            AddressUnit::Byte,
            vec![
                instruction(0, InstructionFlow::FallThrough),
                instruction(1, InstructionFlow::Throw),
            ],
            vec![handler(0, 1, 1, CatchType::Any)],
        );
        let graph = ambiguous.control_flow_graph().unwrap();
        let derived = graph.recovered_structured_control_flow();
        assert!(matches!(
            derived.decisions()[0].status,
            StructuredRegionStatus::AmbiguousExtent { .. }
        ));
        assert!(!contains_try(&derived.lift()));

        let shared = FunctionBody::new(
            AddressUnit::Byte,
            vec![
                instruction(0, InstructionFlow::Return),
                instruction(1, InstructionFlow::Throw),
            ],
            vec![
                handler(0, 1, 1, CatchType::Type("example/First".into())),
                handler(0, 1, 1, CatchType::Any),
            ],
        );
        let graph = shared.control_flow_graph().unwrap();
        let derived = graph.recovered_structured_control_flow();
        assert!(matches!(
            derived.decisions()[0].status,
            StructuredRegionStatus::OverlappingHandlerExtents { .. }
        ));
        assert!(!contains_try(&derived.lift()));
    }
}

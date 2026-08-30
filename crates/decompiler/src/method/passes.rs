//! Configured MLIL presentation-pass schedule.

use std::cell::RefCell;
use std::convert::Infallible;
use std::rc::Rc;

use mlil::cfglib::{AstNode, Cfg, LiftReport, Pass, PassChange, PassId, PassPipeline};
use mlil::{EdgeMetadata, Function, Instruction};

use crate::options::{DecompilerPass, DecompilerPasses};

use super::regions::detach_monitor_cleanup_coverage;

type Graph = Cfg<Instruction, EdgeMetadata>;
type Structure = (AstNode<Instruction>, LiftReport);

struct PresentationPass {
    pass: DecompilerPass,
    structure: Rc<RefCell<Option<Structure>>>,
}

pub(super) struct Presentation {
    pub(super) function: Function,
    pub(super) structure: Option<Structure>,
}

impl Pass<Graph> for PresentationPass {
    type Error = Infallible;

    fn id(&self) -> PassId {
        PassId::new(self.pass.name())
    }

    fn run(&mut self, cfg: &mut Graph) -> Result<PassChange, Self::Error> {
        // A structure result describes only the exact graph state produced by
        // the pass that captured it. Any following transformation invalidates it.
        self.structure.borrow_mut().take();
        let changed = match self.pass {
            DecompilerPass::RemoveUnreachableExceptionRegions => {
                cfg.remove_unreachable_regions() > 0
            }
            DecompilerPass::PropagateValueAliases => {
                let stats = mlil::cfglib::alias_propagation(cfg);
                stats.uses_rewritten > 0 || stats.aliases_removed > 0
            }
            DecompilerPass::DetachMonitorCleanupCoverage => {
                detach_monitor_cleanup_coverage(cfg) > 0
            }
            DecompilerPass::ExtendEquivalentExceptionCoverage => {
                mlil::cfglib::ir::mlil::extend_equivalent_coverage(cfg) > 0
            }
            DecompilerPass::PromoteHandlerExtents => mlil::cfglib::promote_handler_extents(cfg) > 0,
            DecompilerPass::DuplicateStructuringTails => {
                let duplication = mlil::cfglib::duplicate_structuring_tails_with_structure(cfg);
                let changed = duplication.blocks_materialized > 0;
                *self.structure.borrow_mut() = Some((duplication.ast, duplication.report));
                changed
            }
        };
        Ok(PassChange::from_changed(changed))
    }
}

/// Builds one derived function and applies selected passes in canonical order.
pub(super) fn apply(function: &Function, passes: &DecompilerPasses) -> Presentation {
    let mut pipeline = PassPipeline::<Graph, Infallible>::new();
    let structure = Rc::new(RefCell::new(None));
    for pass in passes {
        pipeline.push(PresentationPass {
            pass,
            structure: Rc::clone(&structure),
        });
    }
    let function = function.with_derived_cfg(|cfg| {
        let _report = pipeline.run_infallible(cfg);
    });
    let structure = structure.borrow_mut().take();
    Presentation {
        function,
        structure,
    }
}

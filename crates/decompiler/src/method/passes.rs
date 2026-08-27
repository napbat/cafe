//! Configured MLIL presentation-pass schedule.

use std::convert::Infallible;

use mlil::cfglib::{Cfg, Pass, PassChange, PassId, PassPipeline};
use mlil::{EdgeMetadata, Function, Instruction};

use crate::options::{DecompilerPass, DecompilerPasses};

use super::regions::detach_monitor_cleanup_coverage;

type Graph = Cfg<Instruction, EdgeMetadata>;

struct PresentationPass(DecompilerPass);

impl Pass<Graph> for PresentationPass {
    type Error = Infallible;

    fn id(&self) -> PassId {
        PassId::new(self.0.name())
    }

    fn run(&mut self, cfg: &mut Graph) -> Result<PassChange, Self::Error> {
        let changed = match self.0 {
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
                mlil::cfglib::duplicate_structuring_tails(cfg) > 0
            }
        };
        Ok(PassChange::from_changed(changed))
    }
}

/// Builds one derived function and applies selected passes in canonical order.
pub(super) fn apply(function: &Function, passes: &DecompilerPasses) -> Function {
    let mut pipeline = PassPipeline::<Graph, Infallible>::new();
    for pass in passes {
        pipeline.push(PresentationPass(pass));
    }
    function.with_derived_cfg(|cfg| {
        let _report = pipeline.run_infallible(cfg);
    })
}

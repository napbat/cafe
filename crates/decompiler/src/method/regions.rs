//! JVM-shape exception-region presentation surgery.

use std::collections::BTreeSet;

use mlil::cfglib::{BlockId, Cfg, HandlerKind, RegionId};
use mlil::{EdgeMetadata, Instruction, MonitorAction, Operation};

type Graph = Cfg<Instruction, EdgeMetadata>;

/// Strips the self-coverage of monitor cleanups on a derived graph.
///
/// javac protects each `synchronized` cleanup (`monitorexit; athrow`)
/// with its own catch-any handler, so an exception during the release
/// re-enters the same cleanup — a cycle that defeats tree structuring.
/// The `synchronized` statement the region recovery regenerates carries
/// exactly that self-coverage, so dropping it from the presentation view
/// is a round-trip identity. Only chains that are exactly a monitor
/// cleanup strip, and any fallback rendering of a monitor method lands in
/// the state machine over the canonical function, so semantics never
/// change silently.
pub(super) fn detach_monitor_cleanup_coverage(cfg: &mut Graph) -> usize {
    let mut strips: Vec<(RegionId, BTreeSet<BlockId>, BlockId)> = Vec::new();
    for region in cfg.regions() {
        for handler in &region.handlers {
            if !matches!(handler.kind, HandlerKind::CatchAll) {
                continue;
            }
            let destination = cleanup_destination(cfg, handler.entry);
            if !region.protected_blocks.contains(&destination) {
                continue;
            }
            let Some(chain) = cleanup_chain(cfg, &region.protected_blocks, destination) else {
                continue;
            };
            strips.push((region.id, chain, handler.entry));
        }
    }
    let detached = strips.len();
    for (id, chain, entry) in strips {
        let mut doomed = Vec::new();
        for &block in &chain {
            for &edge in cfg.successor_edges(block) {
                let reference = cfg.edge(edge);
                if reference.payload().role.is_exception() && reference.target() == entry {
                    doomed.push(edge);
                }
            }
        }
        for edge in doomed {
            cfg.remove_edge(edge);
        }
        if let Some(region) = cfg.region_mut(id) {
            for block in &chain {
                region.protected_blocks.remove(block);
            }
        }
    }
    detached
}

/// The first block of the cleanup a landing pad leads to: the pad's
/// normal successor, or the pad itself when it carries the cleanup.
fn cleanup_destination(cfg: &Graph, entry: BlockId) -> BlockId {
    cfg.successor_edges(entry)
        .iter()
        .map(|&edge| cfg.edge(edge))
        .find(|edge| !edge.payload().role.is_exception())
        .map_or(entry, mlil::cfglib::Edge::target)
}

/// The protected chain from `start` when it is exactly one monitor
/// cleanup: the delivered exception materialized and moved, one monitor
/// released, and the exception rethrown — nothing else.
fn cleanup_chain(
    cfg: &Graph,
    protected: &BTreeSet<BlockId>,
    start: BlockId,
) -> Option<BTreeSet<BlockId>> {
    let mut chain = BTreeSet::new();
    let mut releases = 0usize;
    let mut current = Some(start);
    while let Some(block) = current {
        if !protected.contains(&block) || !chain.insert(block) {
            break;
        }
        for instruction in cfg.block(block).instructions() {
            match instruction.operation() {
                Operation::Monitor(MonitorAction::Exit) => releases += 1,
                Operation::CaughtException(_)
                | Operation::Copy
                | Operation::TypeRefine
                | Operation::Throw
                | Operation::Jump
                | Operation::Nop => {}
                _ => return None,
            }
        }
        current = cfg
            .successor_edges(block)
            .iter()
            .map(|&edge| cfg.edge(edge))
            .find(|edge| !edge.payload().role.is_exception())
            .map(mlil::cfglib::Edge::target);
    }
    (releases > 0).then_some(chain)
}

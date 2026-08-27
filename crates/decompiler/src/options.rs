//! Source-recovery policies.

use std::collections::{BTreeSet, btree_set};

/// One opt-in transformation in the MLIL presentation pipeline.
///
/// Variants are declared in their required execution order. A
/// [`DecompilerPasses`] selection always follows that canonical order,
/// independently of the order in which callers enable passes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DecompilerPass {
    /// Drops exception regions whose protected blocks are unreachable.
    RemoveUnreachableExceptionRegions,
    /// Replaces dominated value-preserving aliases and removes dead aliases.
    PropagateValueAliases,
    /// Detaches javac monitor-cleanup self-coverage in the presentation graph.
    DetachMonitorCleanupCoverage,
    /// Extends protected regions across equivalent non-throwing joins.
    ExtendEquivalentExceptionCoverage,
    /// Recovers conservative derived handler-body extents for structuring.
    PromoteHandlerExtents,
    /// Duplicates small shared tails that prevent tree structuring.
    DuplicateStructuringTails,
}

impl DecompilerPass {
    /// Every built-in pass in canonical execution order.
    pub const ALL: [Self; 6] = [
        Self::RemoveUnreachableExceptionRegions,
        Self::PropagateValueAliases,
        Self::DetachMonitorCleanupCoverage,
        Self::ExtendEquivalentExceptionCoverage,
        Self::PromoteHandlerExtents,
        Self::DuplicateStructuringTails,
    ];

    /// Returns the stable configuration- and report-facing pass name.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::RemoveUnreachableExceptionRegions => "remove-unreachable-exception-regions",
            Self::PropagateValueAliases => "propagate-value-aliases",
            Self::DetachMonitorCleanupCoverage => "detach-monitor-cleanup-coverage",
            Self::ExtendEquivalentExceptionCoverage => "extend-equivalent-exception-coverage",
            Self::PromoteHandlerExtents => "promote-handler-extents",
            Self::DuplicateStructuringTails => "duplicate-structuring-tails",
        }
    }
}

/// Selected built-in decompiler passes.
///
/// This is a set rather than a caller-ordered list: pass dependencies retain
/// one deterministic schedule while callers opt individual transformations in
/// or out. [`Self::default`] enables the recommended presentation pipeline and
/// [`Self::none`] starts with an exact, unnormalized MLIL graph.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecompilerPasses {
    enabled: BTreeSet<DecompilerPass>,
}

impl DecompilerPasses {
    /// Selects no presentation transformations.
    #[must_use]
    pub const fn none() -> Self {
        Self {
            enabled: BTreeSet::new(),
        }
    }

    /// Selects Cafe's recommended source-recovery pipeline.
    #[must_use]
    pub fn recommended() -> Self {
        Self::only(DecompilerPass::ALL)
    }

    /// Selects exactly the supplied passes.
    #[must_use]
    pub fn only(passes: impl IntoIterator<Item = DecompilerPass>) -> Self {
        Self {
            enabled: passes.into_iter().collect(),
        }
    }

    /// Enables a pass, returning whether the selection changed.
    pub fn enable(&mut self, pass: DecompilerPass) -> bool {
        self.enabled.insert(pass)
    }

    /// Disables a pass, returning whether the selection changed.
    pub fn disable(&mut self, pass: DecompilerPass) -> bool {
        self.enabled.remove(&pass)
    }

    /// Enables a pass and returns the updated selection.
    #[must_use]
    pub fn with_enabled(mut self, pass: DecompilerPass) -> Self {
        self.enable(pass);
        self
    }

    /// Disables a pass and returns the updated selection.
    #[must_use]
    pub fn without(mut self, pass: DecompilerPass) -> Self {
        self.disable(pass);
        self
    }

    /// Returns whether a pass is selected.
    #[must_use]
    pub fn contains(&self, pass: DecompilerPass) -> bool {
        self.enabled.contains(&pass)
    }

    /// Iterates selected passes in canonical execution order.
    pub fn iter(&self) -> std::iter::Copied<btree_set::Iter<'_, DecompilerPass>> {
        self.enabled.iter().copied()
    }

    /// Returns the number of selected passes.
    #[must_use]
    pub fn len(&self) -> usize {
        self.enabled.len()
    }

    /// Returns whether no pass is selected.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.enabled.is_empty()
    }
}

impl Default for DecompilerPasses {
    fn default() -> Self {
        Self::recommended()
    }
}

impl FromIterator<DecompilerPass> for DecompilerPasses {
    fn from_iter<T: IntoIterator<Item = DecompilerPass>>(iter: T) -> Self {
        Self::only(iter)
    }
}

impl<'selection> IntoIterator for &'selection DecompilerPasses {
    type Item = DecompilerPass;
    type IntoIter = std::iter::Copied<btree_set::Iter<'selection, DecompilerPass>>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

/// Preferred Java representation of method control flow.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ControlFlowPreference {
    /// Use structured Java when cfglib recovers it without labels or gotos,
    /// otherwise use an exact state machine.
    #[default]
    StructuredWhenReducible,
    /// Always render normal control flow as a state machine.
    StateMachine,
}

/// Configurable Java decompiler policies.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecompilerOptions {
    /// Preferred normal-control-flow representation.
    pub control_flow: ControlFlowPreference,
    /// Include members marked synthetic by the class file.
    pub include_synthetic_members: bool,
    /// MLIL presentation transformations used before HLIL lifting.
    pub passes: DecompilerPasses,
}

impl DecompilerOptions {
    /// Replaces the selected presentation passes.
    #[must_use]
    pub fn with_passes(mut self, passes: DecompilerPasses) -> Self {
        self.passes = passes;
        self
    }
}

impl Default for DecompilerOptions {
    fn default() -> Self {
        Self {
            control_flow: ControlFlowPreference::StructuredWhenReducible,
            include_synthetic_members: true,
            passes: DecompilerPasses::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recommended_profile_contains_every_current_pass_in_canonical_order() {
        let passes = DecompilerPasses::recommended();
        assert_eq!(passes.iter().collect::<Vec<_>>(), DecompilerPass::ALL);
    }

    #[test]
    fn exact_selection_deduplicates_and_uses_canonical_order() {
        let passes = DecompilerPasses::only([
            DecompilerPass::DuplicateStructuringTails,
            DecompilerPass::PropagateValueAliases,
            DecompilerPass::DuplicateStructuringTails,
        ]);
        assert_eq!(
            passes.iter().collect::<Vec<_>>(),
            [
                DecompilerPass::PropagateValueAliases,
                DecompilerPass::DuplicateStructuringTails,
            ]
        );
    }

    #[test]
    fn passes_can_be_enabled_and_disabled_fluently() {
        let passes = DecompilerPasses::none()
            .with_enabled(DecompilerPass::PromoteHandlerExtents)
            .without(DecompilerPass::PropagateValueAliases);
        assert!(passes.contains(DecompilerPass::PromoteHandlerExtents));
        assert_eq!(passes.len(), 1);
        assert!(!passes.is_empty());
    }
}

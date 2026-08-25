//! Source-recovery policies.

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
}

impl Default for DecompilerOptions {
    fn default() -> Self {
        Self {
            control_flow: ControlFlowPreference::StructuredWhenReducible,
            include_synthetic_members: true,
        }
    }
}

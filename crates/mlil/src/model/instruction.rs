//! Java-managed MLIL instruction metadata.

/// Observable semantic effect beyond variable definitions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Effect {
    /// Read managed heap or array state.
    ReadMemory,
    /// Mutate managed heap or array state.
    WriteMemory,
    /// Allocate managed storage.
    Allocate,
    /// Invoke another method or dynamic call site.
    Call,
    /// Acquire or release synchronization state.
    Synchronize,
    /// May transfer through an exception edge or terminate exceptionally.
    Throw,
    /// Changes intraprocedural control flow or exits the function.
    Control,
}

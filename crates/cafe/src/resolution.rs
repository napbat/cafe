//! Explicit definition-resolution outcomes.

/// Result of resolving an identity across a multi-module [`Program`](crate::Program).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Resolution<T> {
    /// No definition has the requested identity.
    Missing,
    /// Exactly one definition has the requested identity.
    Unique(T),
    /// More than one module defines the requested identity.
    Ambiguous {
        /// Number of matching definitions.
        matches: usize,
    },
}

impl<T> Resolution<T> {
    /// Returns the uniquely resolved value, if resolution was unambiguous.
    #[must_use]
    pub fn unique(self) -> Option<T> {
        match self {
            Self::Unique(value) => Some(value),
            Self::Missing | Self::Ambiguous { .. } => None,
        }
    }

    /// Returns whether no definition matched.
    #[must_use]
    pub const fn is_missing(&self) -> bool {
        matches!(self, Self::Missing)
    }

    /// Returns whether more than one definition matched.
    #[must_use]
    pub const fn is_ambiguous(&self) -> bool {
        matches!(self, Self::Ambiguous { .. })
    }
}

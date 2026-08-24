//! Format-qualified mappings between bytecode regions.

use std::fmt;

use crate::{AddressRange, AddressUnit, BinaryFormat, CodeAddress, FunctionSymbol};

/// Coordinate system for addresses within one executable function.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct FunctionCoordinate {
    /// Bytecode format that defines the addresses.
    pub format: BinaryFormat,
    /// Overload-qualified function identity.
    pub symbol: FunctionSymbol,
    /// Unit used by addresses and ranges.
    pub address_unit: AddressUnit,
}

impl FunctionCoordinate {
    /// Creates a function coordinate system.
    #[must_use]
    pub const fn new(
        format: BinaryFormat,
        symbol: FunctionSymbol,
        address_unit: AddressUnit,
    ) -> Self {
        Self {
            format,
            symbol,
            address_unit,
        }
    }
}

impl fmt::Display for FunctionCoordinate {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{} {}.{}{}",
            self.format, self.symbol.owner, self.symbol.name, self.symbol.signature
        )
    }
}

/// One source region and the generated region that represents it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SourceMapEntry {
    /// Half-open range in the source coordinate system.
    pub source: AddressRange,
    /// Half-open range in the generated coordinate system.
    pub generated: AddressRange,
}

/// Invalid region supplied to a [`SourceMap`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum SourceMapError {
    /// Source range is empty or reversed.
    #[error("source-map source range {start}..{end} is empty or reversed")]
    InvalidSourceRange {
        /// Inclusive source start.
        start: CodeAddress,
        /// Exclusive source end.
        end: CodeAddress,
    },
    /// Generated range is empty or reversed.
    #[error("source-map generated range {start}..{end} is empty or reversed")]
    InvalidGeneratedRange {
        /// Inclusive generated start.
        start: CodeAddress,
        /// Exclusive generated end.
        end: CodeAddress,
    },
}

/// Deterministic many-to-many provenance between two bytecode functions.
///
/// Entries may overlap on either side so instruction expansion and fusion can
/// both be represented. Exact duplicate entries are discarded. Regions that
/// were elided or synthesized simply have no correspondence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceMap {
    source: FunctionCoordinate,
    generated: FunctionCoordinate,
    entries: Vec<SourceMapEntry>,
}

impl SourceMap {
    /// Creates an empty mapping between two function coordinate systems.
    #[must_use]
    pub const fn new(source: FunctionCoordinate, generated: FunctionCoordinate) -> Self {
        Self {
            source,
            generated,
            entries: Vec::new(),
        }
    }

    /// Returns the source function coordinate system.
    #[must_use]
    pub const fn source(&self) -> &FunctionCoordinate {
        &self.source
    }

    /// Returns the generated function coordinate system.
    #[must_use]
    pub const fn generated(&self) -> &FunctionCoordinate {
        &self.generated
    }

    /// Adds one mapping and retains stable source-then-generated ordering.
    ///
    /// Returns `true` when a new entry was inserted and `false` for an exact
    /// duplicate.
    ///
    /// # Errors
    ///
    /// Returns an error when either half-open range is empty or reversed.
    pub fn insert(
        &mut self,
        source: AddressRange,
        generated: AddressRange,
    ) -> Result<bool, SourceMapError> {
        if source.is_empty() {
            return Err(SourceMapError::InvalidSourceRange {
                start: source.start,
                end: source.end,
            });
        }
        if generated.is_empty() {
            return Err(SourceMapError::InvalidGeneratedRange {
                start: generated.start,
                end: generated.end,
            });
        }
        let entry = SourceMapEntry { source, generated };
        match self.entries.binary_search(&entry) {
            Ok(_) => Ok(false),
            Err(position) => {
                self.entries.insert(position, entry);
                Ok(true)
            }
        }
    }

    /// Returns mappings in deterministic source-then-generated order.
    #[must_use]
    pub fn entries(&self) -> &[SourceMapEntry] {
        &self.entries
    }

    /// Returns every mapping whose source range contains `address`.
    pub fn mappings_from(&self, address: CodeAddress) -> impl Iterator<Item = &SourceMapEntry> {
        self.entries
            .iter()
            .filter(move |entry| entry.source.contains(address))
    }

    /// Returns every mapping whose generated range contains `address`.
    pub fn mappings_to(&self, address: CodeAddress) -> impl Iterator<Item = &SourceMapEntry> {
        self.entries
            .iter()
            .filter(move |entry| entry.generated.contains(address))
    }

    /// Returns whether no region correspondence has been recorded.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Returns the number of distinct region correspondences.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.entries.len()
    }
}

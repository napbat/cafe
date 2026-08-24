//! Hidden-API restriction metadata used by boot-class-path DEX files.

/// Hidden-API flags in class-definition and member declaration order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HiddenApiClassData {
    /// One flag vector per class definition. Empty vectors represent zero flags.
    pub classes: Vec<Vec<u32>>,
    /// Original absolute section offset.
    pub data_offset: u32,
}

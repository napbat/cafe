//! Lossless Java text represented as exact UTF-16 code units.

use std::fmt;

/// Java text with both a convenient lossy view and its exact UTF-16 content.
///
/// Java class files and DEX files can retain unpaired surrogates. Equality,
/// hashing, and ordering therefore use the exact code units rather than the
/// replacement characters in the Rust string view.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct JavaText {
    text: String,
    utf16_units: Vec<u16>,
}

impl JavaText {
    /// Creates Java text from valid Unicode.
    #[must_use]
    pub fn new(value: &str) -> Self {
        Self {
            text: value.to_owned(),
            utf16_units: value.encode_utf16().collect(),
        }
    }

    /// Creates Java text from exact UTF-16 code units.
    #[must_use]
    pub fn from_utf16(utf16_units: Vec<u16>) -> Self {
        Self {
            text: String::from_utf16_lossy(&utf16_units),
            utf16_units,
        }
    }

    /// Returns a Rust string view, replacing unpaired surrogates with U+FFFD.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.text
    }

    /// Returns the exact Java UTF-16 code units.
    #[must_use]
    pub fn utf16_units(&self) -> &[u16] {
        &self.utf16_units
    }

    /// Consumes the value and returns its exact Java UTF-16 code units.
    #[must_use]
    pub fn into_utf16_units(self) -> Vec<u16> {
        self.utf16_units
    }

    /// Returns the number of UTF-16 code units.
    #[must_use]
    pub fn len(&self) -> usize {
        self.utf16_units.len()
    }

    /// Returns whether the text contains no UTF-16 code units.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.utf16_units.is_empty()
    }

    /// Returns whether every surrogate belongs to a valid pair.
    #[must_use]
    pub fn is_valid_unicode(&self) -> bool {
        char::decode_utf16(self.utf16_units.iter().copied()).all(|value| value.is_ok())
    }

    /// Compares the exact Java text with valid Unicode text.
    #[must_use]
    pub fn equals(&self, value: &str) -> bool {
        self.utf16_units.iter().copied().eq(value.encode_utf16())
    }
}

impl fmt::Display for JavaText {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.text.fmt(formatter)
    }
}

impl From<&str> for JavaText {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

impl From<String> for JavaText {
    fn from(value: String) -> Self {
        Self::new(&value)
    }
}

#[cfg(test)]
mod tests {
    use super::JavaText;

    const UNPAIRED_HIGH_SURROGATE: u16 = 0xd800;

    #[test]
    fn preserves_unpaired_surrogates() {
        let value = JavaText::from_utf16(vec![UNPAIRED_HIGH_SURROGATE]);

        assert!(!value.is_valid_unicode());
        assert_eq!(value.utf16_units(), [UNPAIRED_HIGH_SURROGATE]);
        assert_eq!(value.as_str(), "\u{fffd}");
    }
}

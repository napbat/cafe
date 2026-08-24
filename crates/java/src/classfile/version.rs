//! Typed mapping between JVM class-file versions and Java releases.

use std::fmt;

/// Class-file major version used by Java 1.1.
pub const JAVA_1_1_MAJOR_VERSION: u16 = 45;
/// First class-file major version using the modern `major - 44` release mapping.
pub const JAVA_2_MAJOR_VERSION: u16 = 46;
/// Offset between modern Java release numbers and class-file major versions.
pub const JAVA_RELEASE_MAJOR_OFFSET: u16 = 44;
/// Class-file major version used by Java 6.
pub const JAVA_6_MAJOR_VERSION: u16 = JAVA_RELEASE_MAJOR_OFFSET + 6;
/// Class-file major version used by Java 7.
pub const JAVA_7_MAJOR_VERSION: u16 = JAVA_RELEASE_MAJOR_OFFSET + 7;
/// Class-file major version used by Java 8.
pub const JAVA_8_MAJOR_VERSION: u16 = JAVA_RELEASE_MAJOR_OFFSET + 8;
/// Class-file major version used by Java 9.
pub const JAVA_9_MAJOR_VERSION: u16 = JAVA_RELEASE_MAJOR_OFFSET + 9;
/// Class-file major version used by Java 11.
pub const JAVA_11_MAJOR_VERSION: u16 = JAVA_RELEASE_MAJOR_OFFSET + 11;
/// Class-file major version used by Java 12, the first release with preview class files.
pub const JAVA_12_MAJOR_VERSION: u16 = JAVA_RELEASE_MAJOR_OFFSET + 12;
/// Class-file major version used by Java 16.
pub const JAVA_16_MAJOR_VERSION: u16 = JAVA_RELEASE_MAJOR_OFFSET + 16;
/// Class-file major version used by Java 17.
pub const JAVA_17_MAJOR_VERSION: u16 = JAVA_RELEASE_MAJOR_OFFSET + 17;
/// Class-file major version used by Java 26.
pub const JAVA_26_MAJOR_VERSION: u16 = JAVA_RELEASE_MAJOR_OFFSET + 26;
/// Minor version used by ordinary, non-preview class files.
pub const STANDARD_CLASS_MINOR_VERSION: u16 = 0;
/// Minor version marking a preview-feature class file.
pub const PREVIEW_CLASS_MINOR_VERSION: u16 = u16::MAX;

const JAVA_1_1_RELEASE_NUMBER: u16 = 1;

/// A Java release represented by a class-file major version.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum JavaRelease {
    /// Java 1.1, whose conventional version label contains a minor component.
    Java1_1,
    /// Java 2 and later releases.
    Number(u16),
}

impl JavaRelease {
    /// Maps a class-file major version to its Java release.
    #[must_use]
    pub const fn from_class_major(major_version: u16) -> Option<Self> {
        match major_version {
            JAVA_1_1_MAJOR_VERSION => Some(Self::Java1_1),
            major if major >= JAVA_2_MAJOR_VERSION => {
                Some(Self::Number(major - JAVA_RELEASE_MAJOR_OFFSET))
            }
            _ => None,
        }
    }

    /// Returns the integral Java release number.
    ///
    /// Java 1.1 returns `1`; use [`Display`](fmt::Display) for its conventional
    /// `1.1` label.
    #[must_use]
    pub const fn number(self) -> u16 {
        match self {
            Self::Java1_1 => JAVA_1_1_RELEASE_NUMBER,
            Self::Number(number) => number,
        }
    }

    /// Maps this Java release to its class-file major version when representable.
    #[must_use]
    pub const fn class_major(self) -> Option<u16> {
        match self {
            Self::Java1_1 => Some(JAVA_1_1_MAJOR_VERSION),
            Self::Number(number) if number >= 2 => number.checked_add(JAVA_RELEASE_MAJOR_OFFSET),
            Self::Number(_) => None,
        }
    }
}

impl fmt::Display for JavaRelease {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Java1_1 => formatter.write_str("1.1"),
            Self::Number(number) => number.fmt(formatter),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{JAVA_1_1_MAJOR_VERSION, JavaRelease};

    #[test]
    fn maps_legacy_modern_and_unknown_versions() {
        assert_eq!(
            JavaRelease::from_class_major(JAVA_1_1_MAJOR_VERSION),
            Some(JavaRelease::Java1_1)
        );
        assert_eq!(
            JavaRelease::from_class_major(52),
            Some(JavaRelease::Number(8))
        );
        assert_eq!(
            JavaRelease::from_class_major(69),
            Some(JavaRelease::Number(25))
        );
        assert_eq!(JavaRelease::from_class_major(44), None);
    }
}

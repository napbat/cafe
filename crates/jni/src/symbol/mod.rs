//! Canonical JNI dynamic-link symbol escaping and lookup names.

use std::fmt;
use std::fmt::Write;

use thiserror::Error;

use crate::descriptor::Utf16Offset;
use crate::method::NativeMethod;

/// Prefix of every conventionally linked JNI native symbol.
pub const JNI_SYMBOL_PREFIX: &str = "Java_";
/// Separator between escaped class and method names.
pub const JNI_MEMBER_SEPARATOR: char = '_';
/// Separator before an overloaded method's parameter descriptor.
pub const JNI_OVERLOAD_SEPARATOR: &str = "__";
/// Escape emitted for an internal-name package separator.
pub const JNI_PACKAGE_SEPARATOR_ESCAPE: &str = "_";
/// Escape emitted for an underscore code unit.
pub const JNI_UNDERSCORE_ESCAPE: &str = "_1";
/// Escape emitted for an object-descriptor terminator.
pub const JNI_OBJECT_END_ESCAPE: &str = "_2";
/// Escape emitted for an array-descriptor marker.
pub const JNI_ARRAY_ESCAPE: &str = "_3";
/// Prefix emitted before the four hexadecimal digits of another UTF-16 unit.
pub const JNI_UTF16_ESCAPE_PREFIX: &str = "_0";
/// Number of hexadecimal digits used to escape one UTF-16 code unit.
pub const JNI_UTF16_ESCAPE_HEX_WIDTH: usize = 4;

const ASCII_UPPERCASE_START: u16 = 'A' as u16;
const ASCII_UPPERCASE_END: u16 = 'Z' as u16;
const ASCII_LOWERCASE_START: u16 = 'a' as u16;
const ASCII_LOWERCASE_END: u16 = 'z' as u16;
const ASCII_DIGIT_START: u16 = '0' as u16;
const ASCII_DIGIT_END: u16 = '9' as u16;
const PREVIOUS_UTF16_UNIT_DISTANCE: usize = 1;

/// UTF-16 unit with a dedicated JNI symbol escape.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u16)]
pub enum SymbolUnit {
    /// Internal JVM package separator.
    PackageSeparator = '/' as u16,
    /// Literal underscore.
    Underscore = '_' as u16,
    /// Object-descriptor terminator.
    ObjectEnd = ';' as u16,
    /// Array-descriptor marker.
    Array = '[' as u16,
}

impl SymbolUnit {
    /// Returns the exact UTF-16 code unit.
    #[must_use]
    pub const fn unit(self) -> u16 {
        self as u16
    }

    fn from_unit(unit: u16) -> Option<Self> {
        match unit {
            value if value == Self::PackageSeparator.unit() => Some(Self::PackageSeparator),
            value if value == Self::Underscore.unit() => Some(Self::Underscore),
            value if value == Self::ObjectEnd.unit() => Some(Self::ObjectEnd),
            value if value == Self::Array.unit() => Some(Self::Array),
            _ => None,
        }
    }
}

/// Original digit that makes JNI's conventional escape mapping ambiguous.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u16)]
pub enum AmbiguousDigit {
    /// ASCII zero.
    Zero = '0' as u16,
    /// ASCII one.
    One = '1' as u16,
    /// ASCII two.
    Two = '2' as u16,
    /// ASCII three.
    Three = '3' as u16,
}

impl AmbiguousDigit {
    /// Returns the exact UTF-16 code unit.
    #[must_use]
    pub const fn unit(self) -> u16 {
        self as u16
    }

    fn from_unit(unit: u16) -> Option<Self> {
        match unit {
            value if value == Self::Zero.unit() => Some(Self::Zero),
            value if value == Self::One.unit() => Some(Self::One),
            value if value == Self::Two.unit() => Some(Self::Two),
            value if value == Self::Three.unit() => Some(Self::Three),
            _ => None,
        }
    }
}

/// Native symbol form selected by JNI lookup rules.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SymbolStyle {
    /// Class and method name only.
    Short,
    /// Class, method, and parameter descriptor for overload resolution.
    Long,
}

/// Declaration component being escaped into a native symbol.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SymbolComponent {
    /// Internal binary class name.
    ClassName,
    /// Java method name.
    MethodName,
    /// Method parameter descriptor without parentheses.
    ParameterDescriptor,
}

impl fmt::Display for SymbolComponent {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ClassName => formatter.write_str("class name"),
            Self::MethodName => formatter.write_str("method name"),
            Self::ParameterDescriptor => formatter.write_str("parameter descriptor"),
        }
    }
}

/// Failure of JNI's intentionally partial dynamic symbol mapping.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Error)]
#[error(
    "cannot escape {component}: original digit U+{unit:04X} is ambiguous at UTF-16 offset {offset}"
)]
pub struct SymbolError {
    component: SymbolComponent,
    offset: Utf16Offset,
    digit: AmbiguousDigit,
    unit: u16,
}

impl SymbolError {
    /// Returns the declaration component that cannot be escaped.
    #[must_use]
    pub const fn component(self) -> SymbolComponent {
        self.component
    }

    /// Returns the zero-based UTF-16 position of the ambiguous digit.
    #[must_use]
    pub const fn offset(self) -> Utf16Offset {
        self.offset
    }

    /// Returns the typed original digit.
    #[must_use]
    pub const fn digit(self) -> AmbiguousDigit {
        self.digit
    }
}

/// ASCII name exported by a conventionally linked JNI native library.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct NativeSymbol(String);

impl NativeSymbol {
    /// Returns the complete native symbol spelling.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for NativeSymbol {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

/// JNI lookup candidates in the VM-defined short-then-long order.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct LookupSymbols {
    short: NativeSymbol,
    long: NativeSymbol,
}

impl LookupSymbols {
    /// Returns the short class-and-method symbol searched first by the VM.
    #[must_use]
    pub const fn short(&self) -> &NativeSymbol {
        &self.short
    }

    /// Returns the overload-qualified symbol searched second by the VM.
    #[must_use]
    pub const fn long(&self) -> &NativeSymbol {
        &self.long
    }

    /// Iterates candidates in VM lookup order.
    #[must_use]
    pub fn iter(&self) -> impl ExactSizeIterator<Item = &NativeSymbol> {
        [&self.short, &self.long].into_iter()
    }
}

impl NativeMethod {
    /// Builds one canonical JNI dynamic-link symbol.
    ///
    /// # Errors
    ///
    /// Returns an error when JNI's escape mapping rejects an original digit at
    /// a precursor boundary.
    pub fn symbol(&self, style: SymbolStyle) -> Result<NativeSymbol, SymbolError> {
        let mut symbol = String::from(JNI_SYMBOL_PREFIX);
        append_escaped(
            &mut symbol,
            self.owner().utf16_units(),
            SymbolComponent::ClassName,
        )?;
        symbol.push(JNI_MEMBER_SEPARATOR);
        append_escaped(
            &mut symbol,
            self.name().utf16_units(),
            SymbolComponent::MethodName,
        )?;
        if style == SymbolStyle::Long {
            symbol.push_str(JNI_OVERLOAD_SEPARATOR);
            append_escaped(
                &mut symbol,
                self.descriptor().parameter_utf16_units(),
                SymbolComponent::ParameterDescriptor,
            )?;
        }
        Ok(NativeSymbol(symbol))
    }

    /// Builds both JNI lookup candidates in VM search order.
    ///
    /// # Errors
    ///
    /// Returns an error when either symbol cannot use JNI's escape mapping.
    pub fn lookup_symbols(&self) -> Result<LookupSymbols, SymbolError> {
        Ok(LookupSymbols {
            short: self.symbol(SymbolStyle::Short)?,
            long: self.symbol(SymbolStyle::Long)?,
        })
    }
}

fn append_escaped(
    output: &mut String,
    units: &[u16],
    component: SymbolComponent,
) -> Result<(), SymbolError> {
    for (position, &unit) in units.iter().enumerate() {
        if let Some(digit) = AmbiguousDigit::from_unit(unit)
            && (position == Utf16Offset::START.get()
                || units.get(position - PREVIOUS_UTF16_UNIT_DISTANCE).copied()
                    == Some(SymbolUnit::PackageSeparator.unit()))
        {
            return Err(SymbolError {
                component,
                offset: Utf16Offset::new(position),
                digit,
                unit,
            });
        }

        match SymbolUnit::from_unit(unit) {
            Some(SymbolUnit::PackageSeparator) => output.push_str(JNI_PACKAGE_SEPARATOR_ESCAPE),
            Some(SymbolUnit::Underscore) => output.push_str(JNI_UNDERSCORE_ESCAPE),
            Some(SymbolUnit::ObjectEnd) => output.push_str(JNI_OBJECT_END_ESCAPE),
            Some(SymbolUnit::Array) => output.push_str(JNI_ARRAY_ESCAPE),
            None if is_ascii_alphanumeric(unit) => {
                let character = char::from_u32(u32::from(unit))
                    .expect("an ASCII UTF-16 unit is a valid Unicode scalar");
                output.push(character);
            }
            None => {
                output.push_str(JNI_UTF16_ESCAPE_PREFIX);
                write!(output, "{unit:0JNI_UTF16_ESCAPE_HEX_WIDTH$x}")
                    .expect("writing to a String cannot fail");
            }
        }
    }
    Ok(())
}

const fn is_ascii_alphanumeric(unit: u16) -> bool {
    (unit >= ASCII_UPPERCASE_START && unit <= ASCII_UPPERCASE_END)
        || (unit >= ASCII_LOWERCASE_START && unit <= ASCII_LOWERCASE_END)
        || (unit >= ASCII_DIGIT_START && unit <= ASCII_DIGIT_END)
}

#[cfg(test)]
mod tests {
    use super::{AmbiguousDigit, SymbolComponent, SymbolStyle};
    use crate::method::{InvocationKind, NativeMethod};

    #[test]
    fn produces_short_and_long_jni_symbols() {
        let method = NativeMethod::new(
            "p/q/r/A",
            "f_value",
            "(ILjava/lang/String;[I)D",
            InvocationKind::Instance,
        )
        .unwrap();

        assert_eq!(
            method.symbol(SymbolStyle::Short).unwrap().as_str(),
            "Java_p_q_r_A_f_1value"
        );
        assert_eq!(
            method.symbol(SymbolStyle::Long).unwrap().as_str(),
            "Java_p_q_r_A_f_1value__ILjava_lang_String_2_3I"
        );
    }

    #[test]
    fn escapes_surrogate_code_units_separately() {
        let method =
            NativeMethod::new("sample/Native", "wave😀", "()V", InvocationKind::Static).unwrap();

        assert_eq!(
            method.symbol(SymbolStyle::Short).unwrap().as_str(),
            "Java_sample_Native_wave_0d83d_0de00"
        );
    }

    #[test]
    fn rejects_unchanged_low_digits_at_precursor_boundaries() {
        let method =
            NativeMethod::new("sample/0Native", "open", "()V", InvocationKind::Static).unwrap();
        let error = method.symbol(SymbolStyle::Short).unwrap_err();

        assert_eq!(error.component(), SymbolComponent::ClassName);
        assert_eq!(error.offset().get(), 7);
        assert_eq!(error.digit(), AmbiguousDigit::Zero);
        assert_eq!(method.registration().name().as_str(), "open");
    }
}

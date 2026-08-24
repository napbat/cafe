//! Native method identities, invocation kinds, and ABI prototypes.

use std::fmt;

use crate::Result;
use crate::descriptor::{MethodDescriptor, NativeType};
use crate::text::JavaText;

/// Number of VM-supplied parameters before declared Java parameters.
pub const JNI_FIXED_PARAMETER_COUNT: usize = 2;

/// Whether a native declaration is invoked with an instance or class receiver.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum InvocationKind {
    /// Instance native method receiving a `jobject`.
    Instance,
    /// Static native method receiving a `jclass`.
    Static,
}

/// Type of the receiver parameter supplied by the VM.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ReceiverType {
    /// Instance receiver represented by `jobject`.
    Object,
    /// Declaring class represented by `jclass`.
    Class,
}

impl InvocationKind {
    /// Returns the JNI receiver type for this invocation kind.
    #[must_use]
    pub const fn receiver_type(self) -> ReceiverType {
        match self {
            Self::Instance => ReceiverType::Object,
            Self::Static => ReceiverType::Class,
        }
    }
}

impl ReceiverType {
    /// Returns the JNI ABI type used for this receiver.
    #[must_use]
    pub const fn native_type(self) -> NativeType {
        match self {
            Self::Object => NativeType::Object,
            Self::Class => NativeType::Class,
        }
    }
}

/// Zero-based position of a Java-declared method parameter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ParameterIndex(usize);

impl ParameterIndex {
    /// Creates an index from its zero-based position.
    #[must_use]
    pub const fn new(value: usize) -> Self {
        Self(value)
    }

    /// Returns the zero-based position.
    #[must_use]
    pub const fn get(self) -> usize {
        self.0
    }
}

/// Semantic role of one native ABI parameter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NativeParameterRole {
    /// Per-thread JNI environment supplied first by the VM.
    Environment,
    /// Instance or class receiver supplied second by the VM.
    Receiver(ReceiverType),
    /// Parameter declared in the Java method descriptor.
    Argument(ParameterIndex),
}

/// One typed parameter in a native method ABI prototype.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NativeParameter {
    role: NativeParameterRole,
    native_type: NativeType,
}

impl NativeParameter {
    /// Returns this parameter's semantic role.
    #[must_use]
    pub const fn role(self) -> NativeParameterRole {
        self.role
    }

    /// Returns this parameter's JNI ABI type.
    #[must_use]
    pub const fn native_type(self) -> NativeType {
        self.native_type
    }
}

/// Complete JNI ABI prototype for one native method.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct NativePrototype {
    return_type: NativeType,
    parameters: Vec<NativeParameter>,
}

impl NativePrototype {
    /// Returns the JNI ABI return type.
    #[must_use]
    pub const fn return_type(&self) -> NativeType {
        self.return_type
    }

    /// Returns parameters in native calling order.
    #[must_use]
    pub fn parameters(&self) -> &[NativeParameter] {
        &self.parameters
    }
}

/// Overload-qualified identity of one Java native declaration.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct NativeMethodId {
    owner: JavaText,
    name: JavaText,
    descriptor: JavaText,
}

impl NativeMethodId {
    /// Returns the declaring class's internal JVM name.
    #[must_use]
    pub const fn owner(&self) -> &JavaText {
        &self.owner
    }

    /// Returns the exact method name.
    #[must_use]
    pub const fn name(&self) -> &JavaText {
        &self.name
    }

    /// Returns the exact JVM method descriptor.
    #[must_use]
    pub const fn descriptor(&self) -> &JavaText {
        &self.descriptor
    }
}

impl fmt::Display for NativeMethodId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}.{}{}", self.owner, self.name, self.descriptor)
    }
}

/// Name-and-descriptor key used by JNI explicit native registration.
///
/// This key remains available even when dynamic symbol escaping is impossible.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NativeRegistration<'a> {
    name: &'a JavaText,
    descriptor: &'a JavaText,
}

impl<'a> NativeRegistration<'a> {
    /// Returns the exact Java method name.
    #[must_use]
    pub const fn name(self) -> &'a JavaText {
        self.name
    }

    /// Returns the exact JVM method descriptor.
    #[must_use]
    pub const fn descriptor(self) -> &'a JavaText {
        self.descriptor
    }
}

/// One Java native declaration with a parsed descriptor and invocation kind.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct NativeMethod {
    id: NativeMethodId,
    descriptor: MethodDescriptor,
    invocation: InvocationKind,
}

impl NativeMethod {
    /// Creates a native declaration from valid Unicode names and a descriptor.
    ///
    /// # Errors
    ///
    /// Returns an error if the method descriptor is malformed.
    pub fn new(
        owner: impl Into<JavaText>,
        name: impl Into<JavaText>,
        descriptor: &str,
        invocation: InvocationKind,
    ) -> Result<Self> {
        Ok(Self::from_parts(
            owner.into(),
            name.into(),
            MethodDescriptor::parse(descriptor)?,
            invocation,
        ))
    }

    /// Creates a native declaration from exact Java UTF-16 code units.
    ///
    /// # Errors
    ///
    /// Returns an error if the method descriptor is malformed.
    pub fn from_utf16(
        owner: Vec<u16>,
        name: Vec<u16>,
        descriptor: Vec<u16>,
        invocation: InvocationKind,
    ) -> Result<Self> {
        Ok(Self::from_parts(
            JavaText::from_utf16(owner),
            JavaText::from_utf16(name),
            MethodDescriptor::from_utf16(descriptor)?,
            invocation,
        ))
    }

    /// Creates a native declaration from already typed components.
    #[must_use]
    pub fn from_parts(
        owner: JavaText,
        name: JavaText,
        descriptor: MethodDescriptor,
        invocation: InvocationKind,
    ) -> Self {
        let id = NativeMethodId {
            owner,
            name,
            descriptor: descriptor.text().clone(),
        };
        Self {
            id,
            descriptor,
            invocation,
        }
    }

    /// Returns the overload-qualified declaration identity.
    #[must_use]
    pub const fn id(&self) -> &NativeMethodId {
        &self.id
    }

    /// Returns the declaring class's internal JVM name.
    #[must_use]
    pub const fn owner(&self) -> &JavaText {
        self.id.owner()
    }

    /// Returns the exact method name.
    #[must_use]
    pub const fn name(&self) -> &JavaText {
        self.id.name()
    }

    /// Returns the parsed JVM method descriptor.
    #[must_use]
    pub const fn descriptor(&self) -> &MethodDescriptor {
        &self.descriptor
    }

    /// Returns whether the VM supplies an object or class receiver.
    #[must_use]
    pub const fn invocation(&self) -> InvocationKind {
        self.invocation
    }

    /// Returns the key used by explicit `RegisterNatives` registration.
    #[must_use]
    pub const fn registration(&self) -> NativeRegistration<'_> {
        NativeRegistration {
            name: self.name(),
            descriptor: self.descriptor.text(),
        }
    }

    /// Builds the complete JNI ABI prototype in native calling order.
    #[must_use]
    pub fn prototype(&self) -> NativePrototype {
        let mut parameters =
            Vec::with_capacity(self.descriptor.parameters().len() + JNI_FIXED_PARAMETER_COUNT);
        parameters.push(NativeParameter {
            role: NativeParameterRole::Environment,
            native_type: NativeType::Environment,
        });
        let receiver = self.invocation.receiver_type();
        parameters.push(NativeParameter {
            role: NativeParameterRole::Receiver(receiver),
            native_type: receiver.native_type(),
        });
        parameters.extend(self.descriptor.parameters().iter().enumerate().map(
            |(index, parameter)| NativeParameter {
                role: NativeParameterRole::Argument(ParameterIndex::new(index)),
                native_type: parameter.native_type(),
            },
        ));
        NativePrototype {
            return_type: self.descriptor.return_type().native_type(),
            parameters,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{InvocationKind, NativeMethod, NativeParameterRole, ParameterIndex, ReceiverType};
    use crate::descriptor::NativeType;

    #[test]
    fn builds_instance_prototype_in_jni_order() {
        let method = NativeMethod::new(
            "sample/Native",
            "transform",
            "(ILjava/lang/String;)[B",
            InvocationKind::Instance,
        )
        .unwrap();
        let prototype = method.prototype();

        assert_eq!(prototype.return_type(), NativeType::ByteArray);
        assert_eq!(prototype.parameters().len(), 4);
        assert_eq!(
            prototype.parameters()[0].role(),
            NativeParameterRole::Environment
        );
        assert_eq!(
            prototype.parameters()[1].role(),
            NativeParameterRole::Receiver(ReceiverType::Object)
        );
        assert_eq!(
            prototype.parameters()[2].role(),
            NativeParameterRole::Argument(ParameterIndex::new(0))
        );
        assert_eq!(prototype.parameters()[3].native_type(), NativeType::String);
    }

    #[test]
    fn static_methods_receive_a_class() {
        let method =
            NativeMethod::new("sample/Native", "open", "()V", InvocationKind::Static).unwrap();

        assert_eq!(
            method.prototype().parameters()[1].native_type(),
            NativeType::Class
        );
    }
}

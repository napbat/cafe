//! JVM class-file declarations and whole-compilation-unit decompilation.

use std::collections::BTreeSet;
use std::fmt::Write as _;

use disassembler::ExactText;
use java::analysis::ReferenceHierarchy;
use java::classfile::{
    CLASS_INITIALIZER_NAME, ClassAccessFlags, ClassFile, Constant, ConstantPool, FieldAccessFlags,
    InnerClassAccessFlags, KnownAttribute, KnownAttributeKind, MODULE_INFO_CLASS_NAME,
    MethodAccessFlags, MethodInfo, MethodParametersAttribute,
};
use java::descriptor::{MethodDescriptor, parse_field, parse_method};

use crate::diagnostic::{Diagnostic, DiagnosticCode, MethodIdentity};
use crate::environment::MethodExceptionCatalog;
use crate::method::{BodyKind, BodyRequest, render};
use crate::model::{DecompiledClass, GeneratedSpan, SourceMapEntry};
use crate::names::{SourceNames, identifier, package_and_simple, string_literal};
use crate::options::DecompilerOptions;
use crate::signature::{
    ClassSignature as GenericClassSignature, MethodSignature as GenericMethodSignature,
};
use crate::writer::{IndentedOffsetMap, SourceWriter};
use crate::{Error, Result};

/// Decompiles one parsed JVM class file with default recovery policies.
///
/// # Errors
///
/// Returns an error when class metadata cannot form a Java declaration. A
/// method-specific lifting or rendering failure is retained as a diagnostic and
/// a throwing method body instead.
pub fn decompile_class(class: &ClassFile) -> Result<DecompiledClass> {
    decompile_class_with_options(class, &DecompilerOptions::default())
}

/// Parses and decompiles one complete JVM class file.
///
/// # Errors
///
/// Returns an error for malformed class bytes or declaration metadata.
pub fn decompile_class_bytes(bytes: &[u8]) -> Result<DecompiledClass> {
    decompile_class(&ClassFile::parse(bytes)?)
}

/// Decompiles a class with explicit recovery policies.
///
/// # Errors
///
/// Returns an error when class metadata cannot form a Java declaration.
pub fn decompile_class_with_options(
    class: &ClassFile,
    options: &DecompilerOptions,
) -> Result<DecompiledClass> {
    let names = SourceNames::from_class(class)?;
    render_class(class, options, &names, None, None, None, |method| {
        java::mlil::lift_method(class, method)
    })
}

/// Decompiles a class using caller-supplied hierarchy relationships for JVM
/// frame merges and reference assignability.
///
/// # Errors
///
/// Returns an error when class metadata cannot form a Java declaration.
pub fn decompile_class_with_hierarchy(
    class: &ClassFile,
    hierarchy: &dyn ReferenceHierarchy,
    options: &DecompilerOptions,
) -> Result<DecompiledClass> {
    let names = SourceNames::from_class(class)?;
    render_class(
        class,
        options,
        &names,
        None,
        Some(hierarchy),
        None,
        |method| java::mlil::lift_method_with_hierarchy(class, method, hierarchy),
    )
}

#[derive(Clone, Copy)]
pub(crate) struct MemberDeclaration<'a> {
    pub(crate) simple_name: &'a str,
    pub(crate) access_flags: InnerClassAccessFlags,
}

pub(crate) fn decompile_class_in_unit(
    class: &ClassFile,
    hierarchy: Option<&dyn ReferenceHierarchy>,
    method_exceptions: &MethodExceptionCatalog,
    options: &DecompilerOptions,
    names: &SourceNames,
    member: Option<MemberDeclaration<'_>>,
) -> Result<DecompiledClass> {
    match hierarchy {
        Some(hierarchy) => render_class(
            class,
            options,
            names,
            member,
            Some(hierarchy),
            Some(method_exceptions),
            |method| java::mlil::lift_method_with_hierarchy(class, method, hierarchy),
        ),
        None => render_class(
            class,
            options,
            names,
            member,
            None,
            Some(method_exceptions),
            |method| java::mlil::lift_method(class, method),
        ),
    }
}

#[allow(clippy::too_many_lines)]
fn render_class(
    class: &ClassFile,
    options: &DecompilerOptions,
    names: &SourceNames,
    member: Option<MemberDeclaration<'_>>,
    hierarchy: Option<&dyn ReferenceHierarchy>,
    method_exceptions: Option<&MethodExceptionCatalog>,
    mut lift: impl FnMut(&MethodInfo) -> java::mlil::Result<Option<mlil::Function>>,
) -> Result<DecompiledClass> {
    class.validate()?;
    let internal_name = class.class_name()?.to_owned();
    if internal_name == MODULE_INFO_CLASS_NAME
        || class.access_flags.contains(ClassAccessFlags::MODULE)
    {
        return Err(Error::UnsupportedArtifact(
            "module-info requires module-declaration source recovery".to_owned(),
        ));
    }
    let (package, raw_simple_name) = member.map_or_else(
        || package_and_simple(&internal_name),
        |member| (None, member.simple_name.to_owned()),
    );
    let (simple_name, escaped_class_name) = identifier(&raw_simple_name);
    let mut diagnostics = Vec::new();
    if escaped_class_name {
        diagnostics.push(Diagnostic::class_warning(
            DiagnosticCode::EscapedIdentifier,
            &internal_name,
            format!("class name `{raw_simple_name}` was escaped as `{simple_name}`"),
        ));
    }
    let declaration_kind = declaration_kind(class, &internal_name, &mut diagnostics);
    let generic_class =
        crate::signature::class_attribute(class, &internal_name, &mut diagnostics, names)?;
    let helper = helper_name(class)?;
    let mut writer = SourceWriter::default();
    if let Some(package) = package {
        writer.line(&format!("package {package};"));
        writer.blank();
    }
    writer.line(&class_header(
        class,
        declaration_kind,
        &simple_name,
        names,
        member.map(|member| member.access_flags),
        generic_class.as_ref(),
    )?);
    writer.indent();

    let mut wrote_member = false;
    for field in &class.fields {
        if !options.include_synthetic_members
            && field.access_flags.contains(FieldAccessFlags::SYNTHETIC)
        {
            continue;
        }
        if wrote_member {
            writer.blank();
        }
        writer.line(&field_source(
            class,
            field,
            declaration_kind == DeclarationKind::Interface,
            &internal_name,
            &mut diagnostics,
            names,
        )?);
        wrote_member = true;
    }

    let mut source_map = Vec::new();
    let mut helper_used = false;
    // Methods of this class that declare no exceptions: calls to them
    // provably cannot raise checked exceptions, so their statements skip
    // the rethrow launder.
    let unchecked_calls: BTreeSet<(String, String)> = class
        .methods
        .iter()
        .filter(
            |method| match method.known_attribute(KnownAttributeKind::Exceptions) {
                Some(KnownAttribute::Exceptions(attribute)) => attribute.indices.is_empty(),
                _ => true,
            },
        )
        .filter_map(|method| {
            Some((
                method.name(&class.constant_pool).ok()?.to_owned(),
                method.descriptor(&class.constant_pool).ok()?.to_owned(),
            ))
        })
        .collect();
    for (method_index, method) in class.methods.iter().enumerate() {
        if !options.include_synthetic_members
            && method.access_flags.contains(MethodAccessFlags::SYNTHETIC)
        {
            continue;
        }
        let name = method.name(&class.constant_pool)?.to_owned();
        let descriptor_text = method.descriptor(&class.constant_pool)?.to_owned();
        let descriptor = parse_method(&descriptor_text)?;
        let identity = MethodIdentity::new(&name, &descriptor_text);
        if method.access_flags.contains(MethodAccessFlags::BRIDGE)
            && class
                .methods
                .iter()
                .enumerate()
                .any(|(other_index, other)| {
                    other_index != method_index
                        && !other.access_flags.contains(MethodAccessFlags::BRIDGE)
                        && other
                            .name(&class.constant_pool)
                            .is_ok_and(|other_name| other_name == name)
                })
        {
            diagnostics.push(Diagnostic::method_warning(
                DiagnosticCode::DeclarationApproximation,
                &internal_name,
                identity,
                "bridge method is omitted because Java source cannot declare its erased duplicate",
            ));
            continue;
        }
        if declaration_kind == DeclarationKind::Interface && name == CLASS_INITIALIZER_NAME {
            diagnostics.push(Diagnostic::method_error(
                DiagnosticCode::UnsupportedSemantics,
                &internal_name,
                identity,
                "interface class initialization cannot be represented by a Java initializer block",
            ));
            continue;
        }
        let parameter_names = parameter_names(
            class,
            method,
            &descriptor,
            &internal_name,
            &identity,
            &mut diagnostics,
        )?;
        let generic_method = crate::signature::method_attribute(
            class,
            method,
            &descriptor,
            &internal_name,
            &identity,
            &mut diagnostics,
            names,
        )?;
        if wrote_member {
            writer.blank();
        }
        if name == CLASS_INITIALIZER_NAME {
            writer.line("static {");
        } else {
            writer.line(&method_header(
                class,
                method,
                &name,
                &descriptor,
                &parameter_names,
                &simple_name,
                declaration_kind,
                &internal_name,
                &mut diagnostics,
                names,
                generic_method.as_ref(),
            )?);
        }
        let has_body = method.code().is_some()
            && !method.access_flags.contains(MethodAccessFlags::ABSTRACT)
            && !method.access_flags.contains(MethodAccessFlags::NATIVE);
        if !has_body {
            wrote_member = true;
            continue;
        }
        writer.indent();
        match lift(method) {
            Ok(Some(function)) => {
                // One variable per SSA lifetime: storage-derived slot reuse
                // separates into single-typed locals before rendering, and
                // every non-variable identity survives the rebuild.
                let function = function
                    .split_variables()
                    .map(|split| split.function)
                    .unwrap_or(function);
                let request = BodyRequest {
                    function: &function,
                    owner: &internal_name,
                    method: identity.clone(),
                    parameters: &descriptor.parameters,
                    parameter_names: &parameter_names,
                    return_type: &descriptor.return_type,
                    kind: BodyKind::for_method(
                        &name,
                        !method.access_flags.contains(MethodAccessFlags::STATIC),
                        class.access_flags.contains(ClassAccessFlags::ENUM),
                    ),
                    options,
                    rethrow: &helper,
                    names,
                    unchecked_calls: &unchecked_calls,
                    hierarchy,
                    method_exceptions,
                };
                let rendered = render(&request);
                helper_used |= rendered.source.contains(&helper);
                let base = writer.push_source(&rendered.source);
                translate_source_map(
                    &mut source_map,
                    rendered.source_map,
                    &rendered.source,
                    base,
                    2,
                );
                diagnostics.extend(rendered.diagnostics);
            }
            Ok(None) => {
                emit_throwing_stub(
                    &mut writer,
                    "method has no code",
                    name == CLASS_INITIALIZER_NAME,
                );
                diagnostics.push(Diagnostic::method_error(
                    DiagnosticCode::MlilLiftFailed,
                    &internal_name,
                    identity,
                    "method declared executable code but lifting returned no body",
                ));
            }
            Err(error) => {
                if name == java::classfile::INSTANCE_INITIALIZER_NAME {
                    writer.line("super();");
                }
                emit_throwing_stub(
                    &mut writer,
                    &error.to_string(),
                    name == CLASS_INITIALIZER_NAME,
                );
                diagnostics.push(Diagnostic::method_error(
                    DiagnosticCode::MlilLiftFailed,
                    &internal_name,
                    identity,
                    error.to_string(),
                ));
            }
        }
        writer.dedent();
        writer.line("}");
        wrote_member = true;
    }

    if helper_used {
        if wrote_member {
            writer.blank();
        }
        writer.line("@java.lang.SuppressWarnings(\"unchecked\")");
        let visibility = if declaration_kind == DeclarationKind::Interface {
            "public "
        } else {
            "private "
        };
        writer.line(&format!(
            "{visibility}static <T extends java.lang.Throwable> java.lang.RuntimeException {helper}(java.lang.Throwable value) throws T {{"
        ));
        writer.indent();
        writer.line("throw (T) value;");
        writer.dedent();
        writer.line("}");
    }
    writer.dedent();
    writer.line("}");
    Ok(DecompiledClass {
        source: writer.finish(),
        diagnostics,
        source_map,
    })
}

fn emit_throwing_stub(writer: &mut SourceWriter, message: &str, class_initializer: bool) {
    let throwing = format!(
        "throw new java.lang.UnsupportedOperationException({});",
        crate::names::rust_string_literal(message)
    );
    if class_initializer {
        writer.line("if (java.lang.Boolean.TRUE.booleanValue()) {");
        writer.indent();
        writer.line(&throwing);
        writer.dedent();
        writer.line("}");
    } else {
        writer.line(&throwing);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DeclarationKind {
    Class,
    Interface,
}

fn declaration_kind(
    class: &ClassFile,
    name: &str,
    diagnostics: &mut Vec<Diagnostic>,
) -> DeclarationKind {
    if class.access_flags.contains(ClassAccessFlags::ANNOTATION) {
        diagnostics.push(Diagnostic::class_warning(
            DiagnosticCode::DeclarationApproximation,
            name,
            "annotation declaration is rendered as an interface until annotation defaults are lowered",
        ));
        DeclarationKind::Interface
    } else if class.access_flags.contains(ClassAccessFlags::ENUM) {
        diagnostics.push(Diagnostic::class_warning(
            DiagnosticCode::DeclarationApproximation,
            name,
            "enum declaration is rendered as a class until enum constants are reconstructed",
        ));
        DeclarationKind::Class
    } else if class.access_flags.contains(ClassAccessFlags::INTERFACE) {
        DeclarationKind::Interface
    } else {
        if class.known_attribute(KnownAttributeKind::Record).is_some() {
            diagnostics.push(Diagnostic::class_warning(
                DiagnosticCode::DeclarationApproximation,
                name,
                "record declaration is rendered as a class until canonical components are reconstructed",
            ));
        }
        DeclarationKind::Class
    }
}

fn class_header(
    class: &ClassFile,
    kind: DeclarationKind,
    simple_name: &str,
    names: &SourceNames,
    member_flags: Option<InnerClassAccessFlags>,
    generic: Option<&GenericClassSignature>,
) -> Result<String> {
    let mut parts = Vec::new();
    if let Some(flags) = member_flags {
        for (flag, name) in [
            (InnerClassAccessFlags::PUBLIC, "public"),
            (InnerClassAccessFlags::PRIVATE, "private"),
            (InnerClassAccessFlags::PROTECTED, "protected"),
            (InnerClassAccessFlags::STATIC, "static"),
        ] {
            if flags.contains(flag) {
                parts.push(name.to_owned());
            }
        }
        if kind == DeclarationKind::Class && flags.contains(InnerClassAccessFlags::ABSTRACT) {
            parts.push("abstract".to_owned());
        }
        if kind == DeclarationKind::Class && flags.contains(InnerClassAccessFlags::FINAL) {
            parts.push("final".to_owned());
        }
    } else {
        if class.access_flags.contains(ClassAccessFlags::PUBLIC) {
            parts.push("public".to_owned());
        }
        if kind == DeclarationKind::Class && class.access_flags.contains(ClassAccessFlags::ABSTRACT)
        {
            parts.push("abstract".to_owned());
        }
        if kind == DeclarationKind::Class && class.access_flags.contains(ClassAccessFlags::FINAL) {
            parts.push("final".to_owned());
        }
    }
    parts.push(match kind {
        DeclarationKind::Class => "class".to_owned(),
        DeclarationKind::Interface => "interface".to_owned(),
    });
    parts.push(format!(
        "{simple_name}{}",
        generic.map_or("", |signature| signature.type_parameters.as_str())
    ));
    if kind == DeclarationKind::Class && !class.access_flags.contains(ClassAccessFlags::ENUM) {
        let superclass = match generic {
            Some(signature) => Some(signature.superclass.clone()),
            None => class.super_name()?.map(|name| names.class_name(name)),
        };
        if let Some(superclass) = superclass
            && superclass != "java.lang.Object"
        {
            parts.push("extends".to_owned());
            parts.push(superclass);
        }
    }
    let interfaces = if let Some(signature) = generic {
        signature.interfaces.clone()
    } else {
        class
            .interfaces
            .iter()
            .map(|&index| {
                class
                    .constant_pool
                    .class_name(index)
                    .map(|name| names.class_name(name))
            })
            .collect::<java::Result<Vec<_>>>()?
    };
    if !interfaces.is_empty() {
        parts.push(if kind == DeclarationKind::Interface {
            "extends".to_owned()
        } else {
            "implements".to_owned()
        });
        parts.push(interfaces.join(", "));
    }
    Ok(format!("{} {{", parts.join(" ")))
}

fn field_source(
    class: &ClassFile,
    field: &java::classfile::FieldInfo,
    interface: bool,
    class_name: &str,
    diagnostics: &mut Vec<Diagnostic>,
    names: &SourceNames,
) -> Result<String> {
    let raw_name = field.name(&class.constant_pool)?;
    let (name, changed) = identifier(raw_name);
    if changed {
        diagnostics.push(Diagnostic::class_warning(
            DiagnosticCode::EscapedIdentifier,
            class_name,
            format!("field name `{raw_name}` was escaped as `{name}`"),
        ));
    }
    let descriptor = field.descriptor(&class.constant_pool)?;
    let field_type = parse_field(descriptor)?;
    let generic_type =
        crate::signature::field_attribute(class, field, class_name, raw_name, diagnostics, names)?;
    let constant_initializer = constant_value(&class.constant_pool, field, descriptor)?;
    let mut modifiers = field_modifiers(field.access_flags);
    if interface {
        ensure_modifier(&mut modifiers, "public");
        ensure_modifier(&mut modifiers, "static");
        ensure_modifier(&mut modifiers, "final");
    } else if field.access_flags.contains(FieldAccessFlags::FINAL)
        && (!field.access_flags.contains(FieldAccessFlags::STATIC)
            || constant_initializer.is_none())
    {
        modifiers.retain(|modifier| *modifier != "final");
        diagnostics.push(Diagnostic::class_warning(
            DiagnosticCode::DeclarationApproximation,
            class_name,
            format!(
                "field `{raw_name}` omits `final` until source-level definite assignment is reconstructed"
            ),
        ));
    }
    let initializer = constant_initializer
        .or_else(|| interface.then(|| crate::names::default_value(&field_type).to_owned()));
    Ok(format!(
        "{}{} {name}{};",
        if modifiers.is_empty() {
            String::new()
        } else {
            format!("{} ", modifiers.join(" "))
        },
        generic_type.unwrap_or_else(|| names.value_type(&field_type)),
        initializer.map_or_else(String::new, |value| format!(" = {value}"))
    ))
}

#[allow(clippy::too_many_arguments)]
fn method_header(
    class: &ClassFile,
    method: &MethodInfo,
    raw_name: &str,
    descriptor: &MethodDescriptor,
    parameter_names: &[String],
    simple_name: &str,
    declaration_kind: DeclarationKind,
    class_name: &str,
    diagnostics: &mut Vec<Diagnostic>,
    names: &SourceNames,
    generic: Option<&GenericMethodSignature>,
) -> Result<String> {
    let constructor = raw_name == java::classfile::INSTANCE_INITIALIZER_NAME;
    let (name, changed) = if constructor {
        (simple_name.to_owned(), false)
    } else {
        identifier(raw_name)
    };
    if changed {
        diagnostics.push(Diagnostic::method_warning(
            DiagnosticCode::EscapedIdentifier,
            class_name,
            MethodIdentity::new(raw_name, method.descriptor(&class.constant_pool)?),
            format!("method name `{raw_name}` was escaped as `{name}`"),
        ));
    }
    let mut modifiers = method_modifiers(method.access_flags, constructor);
    if declaration_kind == DeclarationKind::Interface
        && method.code().is_some()
        && !method.access_flags.contains(MethodAccessFlags::STATIC)
        && !method.access_flags.contains(MethodAccessFlags::PRIVATE)
    {
        ensure_modifier(&mut modifiers, "default");
        modifiers.retain(|modifier| *modifier != "abstract");
    }
    let parameters = descriptor
        .parameters
        .iter()
        .zip(parameter_names)
        .enumerate()
        .map(|(index, (value_type, name))| {
            let rendered_type = generic
                .and_then(|signature| signature.parameters.get(index))
                .cloned()
                .unwrap_or_else(|| names.value_type(value_type));
            if method.access_flags.contains(MethodAccessFlags::VARARGS)
                && index + 1 == descriptor.parameters.len()
                && let Some(element) = rendered_type.strip_suffix("[]")
            {
                return format!("{element}... {name}");
            }
            format!("{rendered_type} {name}")
        })
        .collect::<Vec<_>>()
        .join(", ");
    let throws = if let Some(signature) = generic.filter(|signature| !signature.throws.is_empty()) {
        format!(" throws {}", signature.throws.join(", "))
    } else {
        throws_clause(class, method, names)?
    };
    let type_parameters = generic.map_or("", |signature| signature.type_parameters.as_str());
    let generic_prefix = if type_parameters.is_empty() {
        String::new()
    } else {
        format!("{type_parameters} ")
    };
    let declaration = if constructor {
        format!("{generic_prefix}{name}({parameters}){throws}")
    } else {
        let return_type = generic.map_or_else(
            || names.return_type(&descriptor.return_type),
            |signature| signature.return_type.clone(),
        );
        format!("{generic_prefix}{return_type} {name}({parameters}){throws}")
    };
    let prefix = if modifiers.is_empty() {
        String::new()
    } else {
        format!("{} ", modifiers.join(" "))
    };
    if method.code().is_some()
        && !method.access_flags.contains(MethodAccessFlags::ABSTRACT)
        && !method.access_flags.contains(MethodAccessFlags::NATIVE)
    {
        Ok(format!("{prefix}{declaration} {{"))
    } else {
        Ok(format!("{prefix}{declaration};"))
    }
}

fn parameter_names(
    class: &ClassFile,
    method: &MethodInfo,
    descriptor: &MethodDescriptor,
    class_name: &str,
    identity: &MethodIdentity,
    diagnostics: &mut Vec<Diagnostic>,
) -> Result<Vec<String>> {
    let metadata = match method.known_attribute(KnownAttributeKind::MethodParameters) {
        Some(KnownAttribute::MethodParameters(attribute)) => Some(attribute),
        _ => None,
    };
    let mut used = BTreeSet::new();
    descriptor
        .parameters
        .iter()
        .enumerate()
        .map(|(index, _)| {
            let raw = metadata
                .and_then(|attribute: &MethodParametersAttribute| attribute.parameters.get(index))
                .filter(|parameter| parameter.name_index != 0)
                .map(|parameter| class.constant_pool.utf8(parameter.name_index))
                .transpose()?
                .map_or_else(|| format!("parameter{index}"), str::to_owned);
            let (mut name, changed) = identifier(&raw);
            if changed {
                diagnostics.push(Diagnostic::method_warning(
                    DiagnosticCode::EscapedIdentifier,
                    class_name,
                    identity.clone(),
                    format!("parameter name `{raw}` was escaped as `{name}`"),
                ));
            }
            if !used.insert(name.clone()) {
                write!(name, "_{index}").expect("writing to a String cannot fail");
                used.insert(name.clone());
            }
            Ok(name)
        })
        .collect()
}

fn throws_clause(class: &ClassFile, method: &MethodInfo, names: &SourceNames) -> Result<String> {
    let Some(KnownAttribute::Exceptions(attribute)) =
        method.known_attribute(KnownAttributeKind::Exceptions)
    else {
        return Ok(String::new());
    };
    let exceptions = attribute
        .indices
        .iter()
        .map(|&index| {
            class
                .constant_pool
                .class_name(index)
                .map(|name| names.class_name(name))
        })
        .collect::<java::Result<Vec<_>>>()?;
    Ok(if exceptions.is_empty() {
        String::new()
    } else {
        format!(" throws {}", exceptions.join(", "))
    })
}

fn constant_value(
    pool: &ConstantPool,
    field: &java::classfile::FieldInfo,
    descriptor: &str,
) -> Result<Option<String>> {
    let Some(KnownAttribute::ConstantValue(attribute)) =
        field.known_attribute(KnownAttributeKind::ConstantValue)
    else {
        return Ok(None);
    };
    let value = match pool.get(attribute.index)? {
        Constant::Integer(value) if descriptor == "Z" => {
            if *value == 0 {
                "false".to_owned()
            } else {
                "true".to_owned()
            }
        }
        Constant::Integer(value) if descriptor == "B" => format!("(byte) {value}"),
        Constant::Integer(value) if descriptor == "C" => format!("(char) {value}"),
        Constant::Integer(value) if descriptor == "S" => format!("(short) {value}"),
        Constant::Integer(value) => value.to_string(),
        Constant::Long(value) => format!("{value}L"),
        Constant::Float(value) => {
            format!("java.lang.Float.intBitsToFloat(0x{:08x})", value.to_bits())
        }
        Constant::Double(value) => format!(
            "java.lang.Double.longBitsToDouble(0x{:016x}L)",
            value.to_bits()
        ),
        Constant::String { string_index } => {
            let exact = pool.utf8_constant(*string_index)?;
            string_literal(&ExactText::from_utf16(exact.utf16_units().to_vec()))
        }
        constant => {
            return Err(Error::UnsupportedArtifact(format!(
                "ConstantValue uses unsupported {} constant",
                constant.tag_name()
            )));
        }
    };
    Ok(Some(value))
}

fn field_modifiers(flags: FieldAccessFlags) -> Vec<&'static str> {
    let mut values = Vec::new();
    for (flag, name) in [
        (FieldAccessFlags::PUBLIC, "public"),
        (FieldAccessFlags::PRIVATE, "private"),
        (FieldAccessFlags::PROTECTED, "protected"),
        (FieldAccessFlags::STATIC, "static"),
        (FieldAccessFlags::FINAL, "final"),
        (FieldAccessFlags::VOLATILE, "volatile"),
        (FieldAccessFlags::TRANSIENT, "transient"),
    ] {
        if flags.contains(flag) {
            values.push(name);
        }
    }
    values
}

fn method_modifiers(flags: MethodAccessFlags, constructor: bool) -> Vec<&'static str> {
    let mut values = Vec::new();
    for (flag, name) in [
        (MethodAccessFlags::PUBLIC, "public"),
        (MethodAccessFlags::PRIVATE, "private"),
        (MethodAccessFlags::PROTECTED, "protected"),
        (MethodAccessFlags::STATIC, "static"),
        (MethodAccessFlags::FINAL, "final"),
        (MethodAccessFlags::SYNCHRONIZED, "synchronized"),
        (MethodAccessFlags::NATIVE, "native"),
        (MethodAccessFlags::ABSTRACT, "abstract"),
        (MethodAccessFlags::STRICT, "strictfp"),
    ] {
        if flags.contains(flag) && !(constructor && flag == MethodAccessFlags::STATIC) {
            values.push(name);
        }
    }
    values
}

fn ensure_modifier(modifiers: &mut Vec<&'static str>, value: &'static str) {
    if !modifiers.contains(&value) {
        modifiers.push(value);
    }
}

fn helper_name(class: &ClassFile) -> Result<String> {
    let names = class
        .methods
        .iter()
        .map(|method| method.name(&class.constant_pool))
        .collect::<java::Result<BTreeSet<_>>>()?;
    let mut candidate = "$cafe$rethrow".to_owned();
    let mut suffix = 0usize;
    while names.contains(candidate.as_str()) {
        suffix += 1;
        candidate = format!("$cafe$rethrow${suffix}");
    }
    Ok(candidate)
}

fn translate_source_map(
    output: &mut Vec<SourceMapEntry>,
    entries: Vec<SourceMapEntry>,
    body: &str,
    base: usize,
    indent: usize,
) {
    let offsets = IndentedOffsetMap::new(body, base, indent);
    for mut entry in entries {
        entry.generated = GeneratedSpan {
            start: offsets.translate(entry.generated.start),
            end: offsets.translate(entry.generated.end),
        };
        output.push(entry);
    }
}

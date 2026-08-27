//! Enclosing-class compilation units for JVM member classes.

use std::collections::{BTreeMap, BTreeSet};

use java::analysis::ReferenceHierarchy;
use java::classfile::{ClassFile, InnerClassAccessFlags, KnownAttribute, KnownAttributeKind};

use crate::class::{MemberDeclaration, decompile_class_in_unit};
use crate::environment::MethodExceptionCatalog;
use crate::model::{DecompiledClass, GeneratedSpan, SourceMapEntry};
use crate::names::SourceNames;
use crate::writer::IndentedOffsetMap;
use crate::{DecompilerOptions, Error, Result};

#[derive(Debug, Clone)]
struct MemberMetadata {
    outer: String,
    simple: String,
    access_flags: InnerClassAccessFlags,
}

struct RenderContext<'a, 'class> {
    classes: &'a BTreeMap<String, &'class ClassFile>,
    relationships: &'a BTreeMap<String, MemberMetadata>,
    children: &'a BTreeMap<String, Vec<String>>,
    hierarchy: Option<&'a dyn ReferenceHierarchy>,
    method_exceptions: &'a MethodExceptionCatalog,
    options: &'a DecompilerOptions,
    names: &'a SourceNames,
}

/// Decompiles one top-level class and its member classes into one Java
/// compilation unit using default recovery policies.
///
/// `members` may contain direct and recursively nested member classes in any
/// order. Local and anonymous classes are deliberately not accepted because
/// their declaration sites require method-level source reconstruction.
///
/// # Errors
///
/// Returns an error when the class family is disconnected, duplicated, has
/// inconsistent `InnerClasses` metadata, or cannot form Java declarations.
pub fn decompile_compilation_unit(
    root: &ClassFile,
    members: &[&ClassFile],
) -> Result<DecompiledClass> {
    decompile_compilation_unit_with_options(root, members, &DecompilerOptions::default())
}

/// Decompiles one top-level class and its member classes with explicit
/// recovery policies.
///
/// # Errors
///
/// Returns an error when member metadata cannot form one enclosing Java
/// compilation unit or when any declaration is malformed.
pub fn decompile_compilation_unit_with_options(
    root: &ClassFile,
    members: &[&ClassFile],
    options: &DecompilerOptions,
) -> Result<DecompiledClass> {
    render_compilation_unit(root, members, None, None, options)
}

/// Decompiles one top-level class and its member classes using caller-supplied
/// hierarchy relationships for JVM frame merges and reference assignability.
///
/// # Errors
///
/// Returns an error when member metadata cannot form one enclosing Java
/// compilation unit or when any declaration is malformed.
pub fn decompile_compilation_unit_with_hierarchy(
    root: &ClassFile,
    members: &[&ClassFile],
    hierarchy: &dyn ReferenceHierarchy,
    options: &DecompilerOptions,
) -> Result<DecompiledClass> {
    render_compilation_unit(root, members, Some(hierarchy), None, options)
}

/// Decompiles one compilation unit with classpath hierarchy and method
/// exception declarations.
///
/// Supplying the archive-wide method catalog lets generated Java omit
/// synthetic checked-exception laundering for calls whose declarations are
/// known not to throw checked exceptions. A missing hierarchy retains
/// conservative reference casts and JVM frame merges.
///
/// # Errors
///
/// Returns an error when member metadata, declarations, or classpath method
/// metadata cannot form Java source.
pub fn decompile_compilation_unit_with_environment(
    root: &ClassFile,
    members: &[&ClassFile],
    hierarchy: Option<&dyn ReferenceHierarchy>,
    method_exceptions: &MethodExceptionCatalog,
    options: &DecompilerOptions,
) -> Result<DecompiledClass> {
    render_compilation_unit(root, members, hierarchy, Some(method_exceptions), options)
}

fn render_compilation_unit<'a>(
    root: &'a ClassFile,
    members: &[&'a ClassFile],
    hierarchy: Option<&dyn ReferenceHierarchy>,
    method_exceptions: Option<&MethodExceptionCatalog>,
    options: &DecompilerOptions,
) -> Result<DecompiledClass> {
    let root_name = root.class_name()?.to_owned();
    let mut classes = BTreeMap::new();
    classes.insert(root_name.clone(), root);
    for &member in members {
        let name = member.class_name()?.to_owned();
        if classes.insert(name.clone(), member).is_some() {
            return Err(Error::UnsupportedArtifact(format!(
                "compilation unit contains duplicate class `{name}`"
            )));
        }
    }
    let owned_method_exceptions;
    let method_exceptions = if let Some(method_exceptions) = method_exceptions {
        method_exceptions
    } else {
        owned_method_exceptions = MethodExceptionCatalog::from_classes(classes.values().copied())?;
        &owned_method_exceptions
    };

    let mut relationships = BTreeMap::new();
    for (name, &class) in classes.iter().filter(|(name, _)| **name != root_name) {
        let metadata = member_metadata(class)?.ok_or_else(|| {
            Error::UnsupportedArtifact(format!("class `{name}` is not a named member class"))
        })?;
        if !classes.contains_key(&metadata.outer) {
            return Err(Error::UnsupportedArtifact(format!(
                "member class `{name}` names missing enclosing class `{}`",
                metadata.outer
            )));
        }
        relationships.insert(name.clone(), metadata);
    }
    validate_tree(&root_name, &classes, &relationships)?;

    let names = SourceNames::from_classes(classes.values().copied())?;
    let mut children: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (name, metadata) in &relationships {
        children
            .entry(metadata.outer.clone())
            .or_default()
            .push(name.clone());
    }
    let context = RenderContext {
        classes: &classes,
        relationships: &relationships,
        children: &children,
        hierarchy,
        method_exceptions,
        options,
        names: &names,
    };
    render_node(&root_name, &context)
}

fn member_metadata(class: &ClassFile) -> Result<Option<MemberMetadata>> {
    if class
        .known_attribute(KnownAttributeKind::EnclosingMethod)
        .is_some()
    {
        return Ok(None);
    }
    let Some(KnownAttribute::InnerClasses(attribute)) =
        class.known_attribute(KnownAttributeKind::InnerClasses)
    else {
        return Ok(None);
    };
    let mut found: Option<MemberMetadata> = None;
    for entry in &attribute.classes {
        if entry.inner_class_info_index != class.this_class {
            continue;
        }
        if entry.outer_class_info_index == 0 || entry.inner_name_index == 0 {
            return Ok(None);
        }
        let candidate = MemberMetadata {
            outer: class
                .constant_pool
                .class_name(entry.outer_class_info_index)?
                .to_owned(),
            simple: class.constant_pool.utf8(entry.inner_name_index)?.to_owned(),
            access_flags: entry.access_flags,
        };
        if let Some(existing) = &found
            && (existing.outer != candidate.outer
                || existing.simple != candidate.simple
                || existing.access_flags != candidate.access_flags)
        {
            return Err(Error::UnsupportedArtifact(format!(
                "class `{}` has inconsistent self entries in `InnerClasses`",
                class.class_name()?
            )));
        }
        found = Some(candidate);
    }
    Ok(found)
}

fn validate_tree(
    root: &str,
    classes: &BTreeMap<String, &ClassFile>,
    relationships: &BTreeMap<String, MemberMetadata>,
) -> Result<()> {
    for name in classes.keys().filter(|name| name.as_str() != root) {
        let mut current = name.as_str();
        let mut visited = BTreeSet::new();
        while current != root {
            if !visited.insert(current.to_owned()) {
                return Err(Error::UnsupportedArtifact(format!(
                    "member-class cycle includes `{current}`"
                )));
            }
            current = relationships
                .get(current)
                .ok_or_else(|| {
                    Error::UnsupportedArtifact(format!(
                        "member class `{name}` is disconnected from root `{root}`"
                    ))
                })?
                .outer
                .as_str();
        }
    }
    Ok(())
}

fn render_node(name: &str, context: &RenderContext<'_, '_>) -> Result<DecompiledClass> {
    let class = context.classes.get(name).copied().ok_or_else(|| {
        Error::UnsupportedArtifact(format!("compilation-unit class `{name}` is missing"))
    })?;
    let member = context
        .relationships
        .get(name)
        .map(|metadata| MemberDeclaration {
            simple_name: metadata.simple.as_str(),
            access_flags: metadata.access_flags,
        });
    let mut rendered = decompile_class_in_unit(
        class,
        context.hierarchy,
        context.method_exceptions,
        context.options,
        context.names,
        member,
    )?;
    let nested = context
        .children
        .get(name)
        .into_iter()
        .flatten()
        .map(|child| render_node(child, context))
        .collect::<Result<Vec<_>>>()?;
    merge_nested(&mut rendered, nested)?;
    Ok(rendered)
}

fn merge_nested(parent: &mut DecompiledClass, children: Vec<DecompiledClass>) -> Result<()> {
    if children.is_empty() {
        return Ok(());
    }
    let Some(prefix) = parent.source.strip_suffix("}\n") else {
        return Err(Error::UnsupportedArtifact(
            "generated class declaration has no closing brace".to_owned(),
        ));
    };
    let mut source = prefix.to_owned();
    source.push('\n');
    for (index, mut child) in children.into_iter().enumerate() {
        if index != 0 {
            source.push('\n');
        }
        let base = source.len();
        for line in child.source.lines() {
            source.push_str("    ");
            source.push_str(line);
            source.push('\n');
        }
        translate_nested_maps(&mut child.source_map, &child.source, base);
        parent.source_map.extend(child.source_map);
        parent.diagnostics.extend(child.diagnostics);
    }
    source.push_str("}\n");
    parent.source = source;
    Ok(())
}

fn translate_nested_maps(entries: &mut [SourceMapEntry], source: &str, base: usize) {
    let offsets = IndentedOffsetMap::new(source, base, 1);
    for entry in entries {
        entry.generated = GeneratedSpan {
            start: offsets.translate(entry.generated.start),
            end: offsets.translate(entry.generated.end),
        };
    }
}

//! Cross-release class-file corpus and typed-attribute round-trip coverage.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use java::classfile::{Attribute, BytesAttribute, ClassFile, KnownAttribute, MarkerAttribute};
use java::jar::JarFile;
use java::{Error, Result};

#[test]
fn round_trips_every_class_across_compiler_targets() -> Result<()> {
    let root = fixture_root().join("classes");
    let paths = class_paths(&root)?;
    assert!(!paths.is_empty(), "compiled class-file corpus is missing");
    assert!(
        paths
            .iter()
            .any(|path| path.components().any(|part| part.as_os_str() == "ecj17")),
        "Eclipse ECJ fixtures are missing"
    );

    let mut attributes = BTreeSet::new();
    let mut majors = BTreeSet::new();
    for path in paths {
        let bytes = fs::read(&path)?;
        let class = ClassFile::parse(&bytes).map_err(|error| corpus_error(&path, &error))?;
        let assembled = class
            .to_bytes()
            .map_err(|error| corpus_error(&path, &error))?;
        assert_eq!(assembled, bytes, "round trip changed {}", path.display());
        majors.insert(class.major_version);
        collect_attributes(&class.attributes, &mut attributes);
        for field in &class.fields {
            collect_attributes(&field.attributes, &mut attributes);
        }
        for method in &class.methods {
            collect_attributes(&method.attributes, &mut attributes);
        }
    }
    assert_eq!(majors, BTreeSet::from([52, 55, 61, 67]));
    for expected in [
        "AnnotationDefault",
        "BootstrapMethods",
        "Code",
        "ConstantValue",
        "Deprecated",
        "EnclosingMethod",
        "Exceptions",
        "InnerClasses",
        "LineNumberTable",
        "LocalVariableTable",
        "LocalVariableTypeTable",
        "MethodParameters",
        "NestHost",
        "NestMembers",
        "PermittedSubclasses",
        "Record",
        "RuntimeInvisibleAnnotations",
        "RuntimeInvisibleParameterAnnotations",
        "RuntimeInvisibleTypeAnnotations",
        "RuntimeVisibleAnnotations",
        "RuntimeVisibleParameterAnnotations",
        "RuntimeVisibleTypeAnnotations",
        "Signature",
        "SourceFile",
        "StackMapTable",
    ] {
        assert!(attributes.contains(expected), "corpus lacks {expected}");
    }
    Ok(())
}

#[test]
fn round_trips_module_attributes_from_a_modular_jar() -> Result<()> {
    let jar = JarFile::open(fixture_root().join("module-corpus.jar"))?;
    jar.validate_archive()?;
    let bytes = jar.read_entry("module-info.class")?;
    let class = ClassFile::parse(&bytes)?;
    assert_eq!(class.to_bytes()?, bytes);
    let mut attributes = BTreeSet::new();
    collect_attributes(&class.attributes, &mut attributes);
    assert!(attributes.contains("Module"));
    assert!(attributes.contains("ModulePackages"));
    assert!(attributes.contains("ModuleMainClass"));
    Ok(())
}

#[test]
fn assembles_and_reparses_manually_added_standard_attributes() -> Result<()> {
    let path = fixture_root()
        .join("classes")
        .join("java8")
        .join("legacy")
        .join("LegacyCorpus.class");
    let mut class = ClassFile::parse(&fs::read(path)?)?;
    let source_debug_name = class.constant_pool.push_utf8("SourceDebugExtension")?;
    let synthetic_name = class.constant_pool.push_utf8("Synthetic")?;
    class
        .attributes
        .push(Attribute::Known(KnownAttribute::SourceDebugExtension(
            BytesAttribute {
                name_index: source_debug_name,
                bytes: b"SMAP\nLegacyCorpus.java\nJava\n".to_vec(),
            },
        )));
    class
        .attributes
        .push(Attribute::Known(KnownAttribute::Synthetic(
            MarkerAttribute {
                name_index: synthetic_name,
            },
        )));

    let bytes = class.to_bytes()?;
    let reparsed = ClassFile::parse(&bytes)?;
    assert_eq!(reparsed.to_bytes()?, bytes);
    assert!(reparsed.attributes.iter().any(|attribute| {
        matches!(
            attribute,
            Attribute::Known(KnownAttribute::SourceDebugExtension(attribute))
                if attribute.bytes.starts_with(b"SMAP")
        )
    }));
    assert!(
        reparsed.attributes.iter().any(|attribute| {
            matches!(attribute, Attribute::Known(KnownAttribute::Synthetic(_)))
        })
    );
    Ok(())
}

fn collect_attributes(attributes: &[Attribute], names: &mut BTreeSet<&'static str>) {
    for attribute in attributes {
        match attribute {
            Attribute::Code(code) => {
                names.insert("Code");
                collect_attributes(&code.attributes, names);
            }
            Attribute::Known(known) => {
                names.insert(known.name());
                if let KnownAttribute::Record(record) = known {
                    for component in &record.components {
                        collect_attributes(&component.attributes, names);
                    }
                }
            }
            Attribute::Raw(_) => {}
        }
    }
}

fn class_paths(root: &Path) -> Result<Vec<PathBuf>> {
    let mut result = Vec::new();
    visit_classes(root, &mut result)?;
    result.sort();
    Ok(result)
}

fn visit_classes(path: &Path, output: &mut Vec<PathBuf>) -> Result<()> {
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let path = entry.path();
        if entry.file_type()?.is_dir() {
            visit_classes(&path, output)?;
        } else if path
            .extension()
            .is_some_and(|extension| extension == "class")
        {
            output.push(path);
        }
    }
    Ok(())
}

fn fixture_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("testdata")
}

fn corpus_error(path: &Path, error: &Error) -> Error {
    Error::InvalidJar(format!("fixture `{}`: {error}", path.display()))
}

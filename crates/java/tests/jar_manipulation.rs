//! End-to-end coverage for editable JAR behavior.

use java::Error;
use java::jar::{
    EntryKind, EntryMetadata, ExtraField, ExtraFieldPlacement, JarFile, Manifest, SignaturePolicy,
};

#[test]
fn creates_edits_and_reopens_complete_jar_state() -> Result<(), Error> {
    let mut jar = JarFile::new();
    jar.set_archive_comment(vec![0xff, b'c', b'a', b'f', b'e'])?;
    jar.add_directory("META-INF/")?;

    let mut manifest = Manifest::new();
    manifest.main_mut().set("Created-By", "cafe")?;
    jar.set_manifest(&manifest)?;
    jar.set_multi_release(true)?;

    let mut metadata = EntryMetadata {
        comment: "resource metadata".to_owned(),
        ..EntryMetadata::default()
    };
    metadata.extra_fields.push(ExtraField {
        header_id: 0xcafe,
        data: vec![1, 2, 3],
        placement: ExtraFieldPlacement::LocalAndCentral,
    });
    let resource = jar.add_file_with_metadata("data/value.txt", b"base".to_vec(), metadata)?;
    jar.add_versioned_file(11, "data/value.txt", b"java 11".to_vec())?;
    jar.add_versioned_file(17, "data/value.txt", b"java 17".to_vec())?;
    jar.add_symlink("latest", "data/value.txt")?;
    jar.set_service_providers(
        "example.Service",
        ["example.First", "example.Second", "example.First"],
    )?;

    let bytes = jar.to_bytes()?;
    let reopened = JarFile::from_bytes(bytes.clone())?;
    assert_eq!(reopened.to_bytes()?, bytes);
    let validation = reopened.validate_archive()?;
    assert_eq!(validation.service_configurations, 1);
    assert_eq!(validation.symlinks, 1);
    assert!(validation.multi_release);
    assert_eq!(reopened.archive_comment(), [0xff, b'c', b'a', b'f', b'e']);
    assert!(reopened.is_multi_release()?);
    assert_eq!(
        reopened.service_providers("example.Service")?,
        Some(vec![
            "example.First".to_owned(),
            "example.Second".to_owned()
        ])
    );
    assert_eq!(
        reopened
            .resolve_entry("data/value.txt", 8)?
            .expect("base resource")
            .release,
        None
    );
    assert_eq!(
        reopened
            .resolve_entry("data/value.txt", 11)?
            .expect("Java 11 resource")
            .release,
        Some(11)
    );
    let selected = reopened
        .resolve_entry("data/value.txt", 21)?
        .expect("highest eligible resource");
    assert_eq!(selected.release, Some(17));
    assert_eq!(reopened.read_entry_by_id(selected.id)?, b"java 17");

    let info = reopened
        .entries()?
        .into_iter()
        .find(|entry| entry.name == "data/value.txt")
        .expect("resource inventory");
    assert_eq!(info.id, resource);
    assert_eq!(info.kind, EntryKind::File);
    assert_eq!(info.metadata.comment, "resource metadata");
    assert!(
        info.metadata
            .extra_fields
            .iter()
            .any(|field| field.header_id == 0xcafe && field.data == [1, 2, 3])
    );

    let mut edited = reopened;
    edited.rename_entry_by_id(resource, "data/renamed.txt")?;
    edited.replace_entry_by_id(resource, b"changed".to_vec())?;
    let service_id = edited
        .entry_ids_named("META-INF/services/example.Service")
        .into_iter()
        .next()
        .expect("service entry");
    edited.remove_entry_by_id(service_id)?;
    edited.move_entry(resource, 0)?;

    let edited_bytes = edited.to_bytes()?;
    let edited = JarFile::from_bytes(edited_bytes)?;
    assert_eq!(
        edited.entry_name(edited.entry_ids()[0])?,
        "data/renamed.txt"
    );
    assert_eq!(edited.read_entry("data/renamed.txt")?, b"changed");
    assert!(edited.read_entry("data/value.txt").is_err());
    assert!(edited.service_providers("example.Service")?.is_none());
    Ok(())
}

#[test]
fn signed_rewrites_require_an_explicit_policy() -> Result<(), Error> {
    let mut manifest = Manifest::new();
    manifest
        .main_mut()
        .set("SHA-256-Digest-Manifest", "stale")?;
    manifest
        .ensure_section("example/Main.class")?
        .attributes_mut()
        .set("SHA-256-Digest", "stale")?;

    let mut signed = JarFile::new();
    signed.set_manifest(&manifest)?;
    signed.add_file("META-INF/SIGNER.SF", b"signature metadata".to_vec())?;
    signed.add_file("META-INF/SIGNER.RSA", b"signature block".to_vec())?;
    signed.add_file("example/Main.class", b"original".to_vec())?;
    let source = signed.to_bytes_with_signature_policy(SignaturePolicy::Preserve)?;

    let mut edited = JarFile::from_bytes(source)?;
    edited.replace_entry("example/Main.class", b"changed".to_vec())?;
    assert!(matches!(edited.to_bytes(), Err(Error::SignedJarMutation)));
    assert!(
        JarFile::from_bytes(edited.to_bytes_with_signature_policy(SignaturePolicy::Preserve)?)?
            .has_signature_artifacts()
    );

    let stripped =
        JarFile::from_bytes(edited.to_bytes_with_signature_policy(SignaturePolicy::Strip)?)?;
    assert!(!stripped.has_signature_artifacts());
    let manifest = stripped.manifest()?.expect("manifest retained");
    assert!(!manifest.main().contains("SHA-256-Digest-Manifest"));
    assert!(manifest.sections().is_empty());
    assert_eq!(stripped.read_entry("example/Main.class")?, b"changed");
    Ok(())
}

#[test]
fn rejects_unsafe_and_duplicate_new_names_transactionally() -> Result<(), Error> {
    let mut jar = JarFile::new();
    jar.add_file("safe.txt", Vec::new())?;
    let before = jar.entry_ids();
    assert!(jar.add_file("../escape.txt", Vec::new()).is_err());
    assert!(jar.add_file("safe.txt", Vec::new()).is_err());
    assert_eq!(jar.entry_ids(), before);
    assert!(jar.rename_entry("safe.txt", "/absolute.txt").is_err());
    assert_eq!(jar.entry_name(before[0])?, "safe.txt");
    Ok(())
}

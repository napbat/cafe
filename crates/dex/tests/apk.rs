//! APK editing and multidex provenance integration tests.

use dex::apk::{ApkFile, DexOrdinal, EntryKind, SignaturePolicy};
use dex::{DexFile, DexVersion};

#[test]
fn creates_edits_and_reopens_ordered_multidex_archives() {
    let mut apk = ApkFile::new();
    let primary = DexFile::new(DexVersion::V040);
    let secondary = DexFile::new(DexVersion::V039);
    let primary_id = apk.put_dex(DexOrdinal::PRIMARY, &primary).unwrap();
    let second_ordinal = DexOrdinal::new(2).unwrap();
    apk.put_dex(second_ordinal, &secondary).unwrap();
    let resource_id = apk
        .add_file("assets/value.txt", b"before".to_vec())
        .unwrap();
    apk.set_archive_comment(b"cafe".to_vec()).unwrap();

    let bytes = apk.to_bytes().unwrap();
    let mut reopened = ApkFile::from_bytes(bytes.clone()).unwrap();
    assert_eq!(reopened.to_bytes().unwrap(), bytes);
    assert_eq!(reopened.dex_entries().unwrap().len(), 2);
    assert_eq!(
        reopened.read_dex(second_ordinal).unwrap().file.version(),
        DexVersion::V039
    );
    assert_eq!(reopened.entry_kind(primary_id).unwrap(), EntryKind::File);

    reopened
        .replace_entry_by_id(resource_id, b"after".to_vec())
        .unwrap();
    reopened
        .rename_entry_by_id(resource_id, "assets/renamed.txt")
        .unwrap();
    assert_eq!(
        reopened.entry_name(resource_id).unwrap(),
        "assets/renamed.txt"
    );
    assert_eq!(reopened.read_entry_by_id(resource_id).unwrap(), b"after");

    let edited = reopened.to_bytes().unwrap();
    let reparsed = ApkFile::from_bytes(edited).unwrap();
    assert_eq!(reparsed.read_all_dex().unwrap().len(), 2);
    assert_eq!(reparsed.archive_comment(), b"cafe");
}

#[test]
fn rejects_gapped_multidex_and_rolls_back_invalid_edits() {
    let mut apk = ApkFile::new();
    let file = DexFile::new(DexVersion::V040);
    apk.put_dex(DexOrdinal::PRIMARY, &file).unwrap();
    apk.put_dex(DexOrdinal::new(3).unwrap(), &file).unwrap();
    assert!(apk.validate_dex_layout().is_err());

    let before = apk.entry_ids();
    let error = apk
        .try_edit(|apk| {
            apk.add_file("duplicate.txt", Vec::new())?;
            apk.add_file("duplicate.txt", Vec::new())?;
            Ok(())
        })
        .unwrap_err();
    assert!(error.to_string().contains("already contains"));
    assert_eq!(apk.entry_ids(), before);
}

#[test]
fn v1_signed_rewrites_require_an_explicit_policy() {
    let mut apk = ApkFile::new();
    apk.add_file("META-INF/APP.SF", b"signature".to_vec())
        .unwrap();
    apk.add_file("META-INF/APP.RSA", b"certificate".to_vec())
        .unwrap();
    let signed = apk
        .to_bytes_with_signature_policy(SignaturePolicy::Preserve)
        .unwrap();

    let mut reopened = ApkFile::from_bytes(signed).unwrap();
    reopened.add_file("new.txt", Vec::new()).unwrap();
    assert!(reopened.to_bytes().is_err());
    let preserved = reopened
        .to_bytes_with_signature_policy(SignaturePolicy::Preserve)
        .unwrap();
    assert!(
        ApkFile::from_bytes(preserved)
            .unwrap()
            .has_signature_artifacts()
    );
    let stripped = reopened
        .to_bytes_with_signature_policy(SignaturePolicy::Strip)
        .unwrap();
    assert!(
        !ApkFile::from_bytes(stripped)
            .unwrap()
            .has_signature_artifacts()
    );
}

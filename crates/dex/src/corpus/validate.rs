//! Corpus validation passes.

use crate::aab::{AabDexVisitControl, AabFile};
use crate::analysis::analyze_method_registers;
use crate::apk::{ApkFile, DexVisitControl};
use crate::disassembly::lift_method;
use crate::file::{DexContainer, DexFile, EncodedMethod};
use crate::program::{MethodBodyMode, ProgramOptions, lift_file_named_with_options};
use crate::{Error, instruction};

use super::model::{
    Corpus, CorpusArtifact, CorpusArtifactKind, CorpusFailure, CorpusMethod, CorpusReport,
    CorpusStage,
};

const DEX_VERSION_OFFSET: std::ops::Range<usize> = 4..7;
const DEX_041_VERSION: &[u8; 3] = b"041";

pub(super) fn corpus(corpus: &Corpus) -> CorpusReport {
    let mut report = CorpusReport::default();
    let mut artifacts = corpus.artifacts().iter().collect::<Vec<_>>();
    artifacts.sort_by(|left, right| left.name().cmp(right.name()));
    for artifact in artifacts {
        report.artifacts += 1;
        match artifact.kind() {
            CorpusArtifactKind::Dex => {
                validate_dex_payload(artifact, None, artifact.bytes(), &mut report);
            }
            CorpusArtifactKind::Apk => validate_apk(artifact, &mut report),
            CorpusArtifactKind::Aab => validate_aab(artifact, &mut report),
        }
    }
    report.failures.sort();
    report
}

fn validate_apk(artifact: &CorpusArtifact, report: &mut CorpusReport) {
    let apk = match ApkFile::from_bytes(artifact.bytes().to_vec()) {
        Ok(apk) => apk,
        Err(error) => {
            failure(
                report,
                artifact,
                None,
                None,
                None,
                None,
                CorpusStage::Container,
                &error,
            );
            return;
        }
    };
    match apk.to_bytes() {
        Ok(bytes) if bytes != artifact.bytes() => difference_failure(
            report,
            artifact,
            None,
            None,
            CorpusStage::Assembly,
            artifact.bytes(),
            &bytes,
            "pristine APK output changed",
        ),
        Ok(_) => {}
        Err(error) => failure(
            report,
            artifact,
            None,
            None,
            None,
            None,
            CorpusStage::Assembly,
            &error,
        ),
    }
    let result: crate::Result<()> = apk.visit_dex_bytes(
        |_| true,
        |origin, bytes| {
            match bytes {
                Ok(bytes) => {
                    validate_dex_payload(
                        artifact,
                        Some(origin.entry_name.as_str()),
                        &bytes,
                        report,
                    );
                }
                Err(error) => failure(
                    report,
                    artifact,
                    Some(origin.entry_name.as_str()),
                    None,
                    None,
                    None,
                    CorpusStage::Read,
                    &error,
                ),
            }
            Ok(DexVisitControl::Continue)
        },
    );
    if let Err(error) = result {
        failure(
            report,
            artifact,
            None,
            None,
            None,
            None,
            CorpusStage::Container,
            &error,
        );
    }
}

fn validate_aab(artifact: &CorpusArtifact, report: &mut CorpusReport) {
    let aab = match AabFile::from_bytes(artifact.bytes().to_vec()) {
        Ok(aab) => aab,
        Err(error) => {
            failure(
                report,
                artifact,
                None,
                None,
                None,
                None,
                CorpusStage::Container,
                &error,
            );
            return;
        }
    };
    let result: crate::Result<()> = aab.visit_dex_bytes(
        |_| true,
        |origin, bytes| {
            match bytes {
                Ok(bytes) => {
                    validate_dex_payload(
                        artifact,
                        Some(origin.entry_name.as_str()),
                        &bytes,
                        report,
                    );
                }
                Err(error) => failure(
                    report,
                    artifact,
                    Some(origin.entry_name.as_str()),
                    None,
                    None,
                    None,
                    CorpusStage::Read,
                    &error,
                ),
            }
            Ok(AabDexVisitControl::Continue)
        },
    );
    if let Err(error) = result {
        failure(
            report,
            artifact,
            None,
            None,
            None,
            None,
            CorpusStage::Container,
            &error,
        );
    }
}

fn validate_dex_payload(
    artifact: &CorpusArtifact,
    entry: Option<&str>,
    bytes: &[u8],
    report: &mut CorpusReport,
) {
    report.dex_files += 1;
    if bytes.get(DEX_VERSION_OFFSET) == Some(DEX_041_VERSION) {
        validate_container(artifact, entry, bytes, report);
    } else {
        match DexFile::parse(bytes) {
            Ok(file) => {
                validate_round_trip(artifact, entry, None, &file, bytes, report);
                validate_file(artifact, entry, None, &file, report);
            }
            Err(error) => failure(
                report,
                artifact,
                entry,
                None,
                None,
                None,
                CorpusStage::DexParse,
                &error,
            ),
        }
    }
}

fn validate_container(
    artifact: &CorpusArtifact,
    entry: Option<&str>,
    bytes: &[u8],
    report: &mut CorpusReport,
) {
    let container = match DexContainer::parse(bytes) {
        Ok(container) => container,
        Err(error) => {
            failure(
                report,
                artifact,
                entry,
                None,
                None,
                None,
                CorpusStage::DexParse,
                &error,
            );
            return;
        }
    };
    match container.to_bytes() {
        Ok(encoded) if encoded != bytes => difference_failure(
            report,
            artifact,
            entry,
            None,
            CorpusStage::Assembly,
            bytes,
            &encoded,
            "DEX 041 container round trip changed bytes",
        ),
        Ok(_) => {}
        Err(error) => failure(
            report,
            artifact,
            entry,
            None,
            None,
            None,
            CorpusStage::Assembly,
            &error,
        ),
    }
    for (position, file) in container.members().iter().enumerate() {
        report.container_members += 1;
        let member = u32::try_from(position).ok();
        validate_file(artifact, entry, member, file, report);
    }
}

fn validate_round_trip(
    artifact: &CorpusArtifact,
    entry: Option<&str>,
    member: Option<u32>,
    file: &DexFile,
    original: &[u8],
    report: &mut CorpusReport,
) {
    match file.to_bytes() {
        Ok(encoded) if encoded != original => difference_failure(
            report,
            artifact,
            entry,
            member,
            CorpusStage::Assembly,
            original,
            &encoded,
            "DEX parse/assemble round trip changed bytes",
        ),
        Ok(_) => {}
        Err(error) => failure(
            report,
            artifact,
            entry,
            member,
            None,
            None,
            CorpusStage::Assembly,
            &error,
        ),
    }
}

fn validate_file(
    artifact: &CorpusArtifact,
    entry: Option<&str>,
    member: Option<u32>,
    file: &DexFile,
    report: &mut CorpusReport,
) {
    report.classes += file.classes().len();
    for class in file.classes() {
        let Some(data) = &class.class_data else {
            continue;
        };
        for method in data.direct_methods.iter().chain(&data.virtual_methods) {
            validate_method(artifact, entry, member, file, method, report);
        }
    }
    for method_bodies in [
        MethodBodyMode::DeclarationsOnly,
        MethodBodyMode::Disassemble,
    ] {
        match lift_file_named_with_options(
            file,
            entry.unwrap_or(artifact.name()),
            ProgramOptions { method_bodies },
        ) {
            Ok(_) => report.program_modules += 1,
            Err(error) => failure(
                report,
                artifact,
                entry,
                member,
                None,
                None,
                CorpusStage::Program,
                &error,
            ),
        }
    }
}

fn validate_method(
    artifact: &CorpusArtifact,
    entry: Option<&str>,
    member: Option<u32>,
    file: &DexFile,
    method: &EncodedMethod,
    report: &mut CorpusReport,
) {
    report.methods += 1;
    let identity = match file.resolve_method(method.method) {
        Ok(identity) => identity,
        Err(error) => {
            failure(
                report,
                artifact,
                entry,
                member,
                None,
                None,
                CorpusStage::Resolution,
                &error,
            );
            return;
        }
    };
    let class = Some(identity.owner.to_owned());
    let method_id = Some(CorpusMethod {
        name: identity.name.to_owned(),
        descriptor: identity.signature.clone(),
    });
    let Some(code) = &method.code else {
        return;
    };
    report.code_methods += 1;
    report.instructions += code.instructions.len();
    if let Err(error) = instruction::encode(&code.instructions) {
        failure(
            report,
            artifact,
            entry,
            member,
            class.clone(),
            method_id.clone(),
            CorpusStage::InstructionEncode,
            &error,
        );
    }
    match lift_method(file, method) {
        Ok(function) if function.body.is_some() => report.control_flow_graphs += 1,
        Ok(_) => {}
        Err(error) => failure(
            report,
            artifact,
            entry,
            member,
            class.clone(),
            method_id.clone(),
            CorpusStage::ControlFlow,
            &error,
        ),
    }
    match analyze_method_registers(file, method) {
        Ok(Some(_)) => report.register_analyses += 1,
        Ok(None) => {}
        Err(error) => failure(
            report,
            artifact,
            entry,
            member,
            class,
            method_id,
            CorpusStage::RegisterAnalysis,
            &error,
        ),
    }
}

#[allow(clippy::too_many_arguments)]
fn failure(
    report: &mut CorpusReport,
    artifact: &CorpusArtifact,
    entry: Option<&str>,
    member: Option<u32>,
    class: Option<String>,
    method: Option<CorpusMethod>,
    stage: CorpusStage,
    error: &Error,
) {
    report.failures.push(CorpusFailure {
        artifact: artifact.name().to_owned(),
        entry: entry.map(str::to_owned),
        dex_member: member,
        class,
        method,
        byte_offset: error_offset(error),
        stage,
        message: error.to_string(),
    });
}

#[allow(clippy::too_many_arguments)]
fn difference_failure(
    report: &mut CorpusReport,
    artifact: &CorpusArtifact,
    entry: Option<&str>,
    member: Option<u32>,
    stage: CorpusStage,
    expected: &[u8],
    actual: &[u8],
    message: &str,
) {
    report.failures.push(CorpusFailure {
        artifact: artifact.name().to_owned(),
        entry: entry.map(str::to_owned),
        dex_member: member,
        class: None,
        method: None,
        byte_offset: first_difference(expected, actual),
        stage,
        message: message.to_owned(),
    });
}

fn error_offset(error: &Error) -> usize {
    match error {
        Error::InvalidDex { offset, .. } => *offset,
        Error::InvalidInstruction { offset, .. } => usize::try_from(*offset)
            .ok()
            .and_then(|offset| offset.checked_mul(2))
            .unwrap_or(usize::MAX),
        Error::Method { source, .. }
        | Error::ApkEntry { source, .. }
        | Error::AabEntry { source, .. } => error_offset(source),
        _ => 0,
    }
}

fn first_difference(left: &[u8], right: &[u8]) -> usize {
    left.iter()
        .zip(right)
        .position(|(left, right)| left != right)
        .unwrap_or_else(|| left.len().min(right.len()))
}

#[cfg(test)]
mod tests {
    use super::super::{Corpus, CorpusArtifact, CorpusStage};
    use crate::apk::{ApkFile, DexOrdinal};
    use crate::{DexFile, DexVersion, Result};

    #[test]
    fn reports_multiple_members_without_fail_fast() -> Result<()> {
        let valid = DexFile::new(DexVersion::V040).to_bytes()?;
        let mut apk = ApkFile::new();
        apk.add_file("classes.dex", b"broken".to_vec())?;
        apk.add_file("classes2.dex", valid)?;
        let mut corpus = Corpus::new();
        corpus.push(CorpusArtifact::apk("sample.apk", apk.to_bytes()?));

        let report = corpus.validate();
        assert_eq!(report.dex_files, 2);
        assert_eq!(report.failures.len(), 1);
        assert_eq!(report.failures[0].entry.as_deref(), Some("classes.dex"));
        assert_eq!(report.failures[0].stage, CorpusStage::DexParse);
        assert_eq!(report.program_modules, 2);
        assert_eq!(DexOrdinal::PRIMARY.get(), 1);
        Ok(())
    }

    #[test]
    fn report_order_is_artifact_then_native_origin() {
        let mut corpus = Corpus::new();
        corpus.push(CorpusArtifact::dex("z.dex", b"bad-z".to_vec()));
        corpus.push(CorpusArtifact::dex("a.dex", b"bad-a".to_vec()));
        let report = corpus.validate();
        assert_eq!(report.failures.len(), 2);
        assert_eq!(report.failures[0].artifact, "a.dex");
        assert_eq!(report.failures[1].artifact, "z.dex");
    }
}

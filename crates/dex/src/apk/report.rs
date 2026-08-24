//! Structured reporting for lossy or unsupported APK rewrites.

use zip::CompressionMethod;

use super::entry::validate_entry_name;
use super::{ApkFile, SignaturePolicy, SignatureState};

const ZIP_32_BIT_MAXIMUM: u64 = u32::MAX as u64;
const CONFIGURED_ZIP_WRITER_VERSION: u8 = 45;

/// Severity of one predicted rewrite effect.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum RewriteSeverity {
    /// Informational change with no semantic metadata loss.
    Notice,
    /// Exact physical metadata or encoding cannot be reproduced.
    Loss,
    /// The configured encoder or signature policy rejects serialization.
    Blocking,
}

/// Typed reason an APK rewrite differs from its input or cannot proceed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum RewriteIssueKind {
    /// Local headers, the central directory, and offsets are regenerated.
    ZipStructureRebuilt,
    /// Original compressed bytes are replaced by a fresh codec stream.
    CompressedStreamReencoded,
    /// General-purpose ZIP flags are selected by the writer.
    HeaderFlagsRegenerated,
    /// A data descriptor is folded into regenerated header metadata.
    DataDescriptorNormalized,
    /// The exact ZIP writer/version marker is not configurable.
    WriterVersionNormalized,
    /// External attributes cannot be completely reconstructed from portable metadata.
    ExternalAttributesNormalized,
    /// A raw non-UTF-8 entry name would be changed.
    RawNameNotReproducible,
    /// The selected compression method has no configured encoder.
    UnsupportedCompression,
    /// Encrypted entries cannot be emitted.
    EncryptionUnsupported,
    /// The current writer configuration cannot emit required ZIP64 metadata.
    Zip64Unsupported,
    /// Entry metadata or its path is invalid for the writer.
    InvalidEntryMetadata,
    /// Signature material blocks the safe default rewrite.
    SignatureRejected,
    /// Signature material is intentionally retained but becomes stale.
    SignaturePreservedStale,
    /// Signature material is intentionally removed.
    SignatureStripped,
}

/// One deterministic predicted rewrite effect.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct RewriteIssue {
    /// Physical entry name, or `None` for an archive-wide effect.
    pub entry: Option<String>,
    /// Whether the effect is informational, lossy, or blocking.
    pub severity: RewriteSeverity,
    /// Machine-readable reason.
    pub kind: RewriteIssueKind,
    /// Human-readable detail.
    pub message: String,
}

/// Complete prediction for an APK serialization policy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RewriteReport {
    /// Signature policy evaluated by the report.
    pub signature_policy: SignaturePolicy,
    /// Signature state before serialization.
    pub signature_state: SignatureState,
    /// Whether serialization can return exact pristine input bytes.
    pub exact_pristine_output: bool,
    /// Number of entries that will be visited by the ZIP writer.
    pub entries: usize,
    /// Sorted rewrite effects.
    pub issues: Vec<RewriteIssue>,
}

impl RewriteReport {
    /// Returns whether serialization is predicted to succeed.
    #[must_use]
    pub fn can_rewrite(&self) -> bool {
        !self
            .issues
            .iter()
            .any(|issue| issue.severity == RewriteSeverity::Blocking)
    }

    /// Returns whether successful output loses any exact physical metadata.
    #[must_use]
    pub fn is_lossy(&self) -> bool {
        self.issues
            .iter()
            .any(|issue| issue.severity == RewriteSeverity::Loss)
    }
}

impl ApkFile {
    /// Predicts every known physical normalization, signature effect, and
    /// unsupported feature for a configured rewrite.
    ///
    /// A pristine archive has no rewrite effects unless `Strip` forces
    /// signature removal. The report performs no decompression and does not
    /// mutate the APK.
    #[must_use]
    pub fn rewrite_report(&self, policy: SignaturePolicy) -> RewriteReport {
        let exact_pristine_output = !self.dirty && policy != SignaturePolicy::Strip;
        let mut report = RewriteReport {
            signature_policy: policy,
            signature_state: self.signature_state(),
            exact_pristine_output,
            entries: self.entries.len(),
            issues: Vec::new(),
        };
        if exact_pristine_output {
            return report;
        }

        push(
            &mut report,
            None,
            RewriteSeverity::Notice,
            RewriteIssueKind::ZipStructureRebuilt,
            "ZIP local headers, central directory, and member offsets will be regenerated",
        );
        report_signature_policy(self, &mut report, policy);
        for entry in &self.entries {
            report_entry(&mut report, entry);
        }
        report.issues.sort();
        report.issues.dedup();
        report
    }
}

fn report_signature_policy(apk: &ApkFile, report: &mut RewriteReport, policy: SignaturePolicy) {
    if !apk.has_signature_artifacts() {
        return;
    }
    let (severity, kind, message) = match policy {
        SignaturePolicy::Reject => (
            RewriteSeverity::Blocking,
            RewriteIssueKind::SignatureRejected,
            "signature material is present and the safe policy rejects mutation",
        ),
        SignaturePolicy::Preserve => (
            RewriteSeverity::Loss,
            RewriteIssueKind::SignaturePreservedStale,
            "signature bytes will be retained but cannot remain cryptographically valid",
        ),
        SignaturePolicy::Strip => (
            RewriteSeverity::Loss,
            RewriteIssueKind::SignatureStripped,
            "v1 files, source-stamp metadata, and the signing block will be removed",
        ),
    };
    push(report, None, severity, kind, message);
}

fn report_entry(report: &mut RewriteReport, entry: &super::entry::ApkEntry) {
    report_entry_constraints(report, entry);
    let Some(original) = entry.original_stats.as_ref() else {
        return;
    };
    if original.raw_name != entry.name.as_bytes() {
        push(
            report,
            Some(&entry.name),
            RewriteSeverity::Blocking,
            RewriteIssueKind::RawNameNotReproducible,
            "the original raw member name is not the exact UTF-8 current name",
        );
    }
    if original.large_file
        || original.size > ZIP_32_BIT_MAXIMUM
        || original.compressed_size > ZIP_32_BIT_MAXIMUM
    {
        push(
            report,
            Some(&entry.name),
            RewriteSeverity::Blocking,
            RewriteIssueKind::Zip64Unsupported,
            "the configured writer disables per-entry ZIP64 output",
        );
    }
    if entry.metadata.compression != CompressionMethod::Stored
        || original.compressed_size != original.size
    {
        push(
            report,
            Some(&entry.name),
            RewriteSeverity::Loss,
            RewriteIssueKind::CompressedStreamReencoded,
            "the original compressed byte stream and codec tuning cannot be reproduced",
        );
    }
    report_original_metadata(report, entry, original);
}

fn report_entry_constraints(report: &mut RewriteReport, entry: &super::entry::ApkEntry) {
    if let Err(error) = validate_entry_name(&entry.name, entry.kind) {
        push(
            report,
            Some(&entry.name),
            RewriteSeverity::Blocking,
            RewriteIssueKind::InvalidEntryMetadata,
            error.to_string(),
        );
    }
    if entry.encrypted {
        push(
            report,
            Some(&entry.name),
            RewriteSeverity::Blocking,
            RewriteIssueKind::EncryptionUnsupported,
            "encrypted ZIP members cannot be rewritten",
        );
    }
    if !matches!(
        entry.metadata.compression,
        CompressionMethod::Stored | CompressionMethod::Deflated
    ) {
        push(
            report,
            Some(&entry.name),
            RewriteSeverity::Blocking,
            RewriteIssueKind::UnsupportedCompression,
            format!(
                "compression method {:?} has no configured encoder",
                entry.metadata.compression
            ),
        );
    }
    if let Err(error) = entry.metadata.write_options(&entry.name) {
        push(
            report,
            Some(&entry.name),
            RewriteSeverity::Blocking,
            RewriteIssueKind::InvalidEntryMetadata,
            error.to_string(),
        );
    }
}

fn report_original_metadata(
    report: &mut RewriteReport,
    entry: &super::entry::ApkEntry,
    original: &super::entry::OriginalEntryStats,
) {
    if original.flags != 0 {
        push(
            report,
            Some(&entry.name),
            RewriteSeverity::Loss,
            RewriteIssueKind::HeaderFlagsRegenerated,
            format!(
                "original general-purpose ZIP flags were 0x{:04x}",
                original.flags
            ),
        );
    }
    if original.using_data_descriptor {
        push(
            report,
            Some(&entry.name),
            RewriteSeverity::Loss,
            RewriteIssueKind::DataDescriptorNormalized,
            "the original entry used a trailing data descriptor",
        );
    }
    if original.version_made_by != CONFIGURED_ZIP_WRITER_VERSION {
        push(
            report,
            Some(&entry.name),
            RewriteSeverity::Loss,
            RewriteIssueKind::WriterVersionNormalized,
            format!(
                "original ZIP writer version {} is not configurable",
                original.version_made_by
            ),
        );
    }
    if original.external_attributes != 0 && entry.metadata.unix_mode.is_none() {
        push(
            report,
            Some(&entry.name),
            RewriteSeverity::Loss,
            RewriteIssueKind::ExternalAttributesNormalized,
            format!(
                "external attributes 0x{:08x} lack a portable Unix-mode representation",
                original.external_attributes
            ),
        );
    }
}

fn push(
    report: &mut RewriteReport,
    entry: Option<&str>,
    severity: RewriteSeverity,
    kind: RewriteIssueKind,
    message: impl Into<String>,
) {
    report.issues.push(RewriteIssue {
        entry: entry.map(str::to_owned),
        severity,
        kind,
        message: message.into(),
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Result;

    #[test]
    fn pristine_and_reencoded_reports_are_explicit() -> Result<()> {
        let mut source = ApkFile::new();
        source.add_file("classes.dex", b"payload".to_vec())?;
        let bytes = source.to_bytes()?;
        let mut parsed = ApkFile::from_bytes(bytes)?;
        assert!(
            parsed
                .rewrite_report(SignaturePolicy::Reject)
                .exact_pristine_output
        );

        parsed.put_file("assets/new.txt", b"new".to_vec())?;
        let report = parsed.rewrite_report(SignaturePolicy::Reject);
        assert!(!report.exact_pristine_output);
        assert!(report.can_rewrite());
        assert!(
            report
                .issues
                .iter()
                .any(|issue| issue.kind == RewriteIssueKind::ZipStructureRebuilt)
        );
        Ok(())
    }

    #[test]
    fn unsupported_codecs_are_blocking() -> Result<()> {
        let mut apk = ApkFile::new();
        let id = apk.add_file("asset.bin", vec![1])?;
        let mut metadata = apk.entry_metadata(id)?.clone();
        metadata.compression = CompressionMethod::BZIP2;
        apk.set_entry_metadata(id, metadata)?;
        let report = apk.rewrite_report(SignaturePolicy::Reject);
        assert!(!report.can_rewrite());
        assert!(
            report
                .issues
                .iter()
                .any(|issue| issue.kind == RewriteIssueKind::UnsupportedCompression)
        );
        Ok(())
    }
}

//! Recognition and safe removal of standard JAR signature material.

use crate::Result;

use super::{EntryId, JarFile};

/// Observable signature state of an editable JAR.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SignatureState {
    /// No standard top-level signature artifacts are present.
    Unsigned,
    /// Signature artifacts are present and the opened archive is unchanged.
    Present,
    /// Signature artifacts remain after an in-memory mutation and may be stale.
    PotentiallyInvalidated,
}

/// Returns whether a JAR entry is a standard top-level signature artifact.
///
/// Matching is ASCII case-insensitive. `META-INF/MANIFEST.MF` is deliberately
/// excluded because it also belongs to unsigned JARs.
#[must_use]
pub fn is_signature_entry(name: &str) -> bool {
    let Some(remainder) = strip_prefix_ascii_case(name, "META-INF/") else {
        return false;
    };
    if remainder.is_empty() || remainder.contains('/') {
        return false;
    }
    let uppercase = remainder.to_ascii_uppercase();
    uppercase.starts_with("SIG-")
        || [".SF", ".DSA", ".RSA", ".EC"]
            .iter()
            .any(|suffix| uppercase.ends_with(suffix))
}

impl JarFile {
    /// Returns the current signature state.
    #[must_use]
    pub fn signature_state(&self) -> SignatureState {
        if !self.has_signature_artifacts() {
            SignatureState::Unsigned
        } else if self.dirty {
            SignatureState::PotentiallyInvalidated
        } else {
            SignatureState::Present
        }
    }

    /// Returns stable IDs of standard signature artifacts in archive order.
    #[must_use]
    pub fn signature_entry_ids(&self) -> Vec<EntryId> {
        self.entries
            .iter()
            .filter(|entry| is_signature_entry(&entry.name))
            .map(|entry| entry.id)
            .collect()
    }

    /// Returns whether standard signature artifacts are present.
    #[must_use]
    pub fn has_signature_artifacts(&self) -> bool {
        self.entries
            .iter()
            .any(|entry| is_signature_entry(&entry.name))
    }

    /// Removes standard signature files and manifest digest attributes.
    ///
    /// The manifest is parsed before any entry is removed, so malformed
    /// manifest data leaves the archive unchanged.
    ///
    /// # Errors
    ///
    /// Returns an error if a present manifest is ambiguous, unreadable, or
    /// malformed.
    pub fn strip_signatures(&mut self) -> Result<usize> {
        let rewritten_manifest = if let Some(mut manifest) = self.manifest()? {
            let before = manifest.clone();
            manifest.strip_digest_attributes();
            (manifest != before)
                .then(|| manifest.to_bytes())
                .transpose()?
        } else {
            None
        };
        let signature_ids = self.signature_entry_ids();
        let removed = signature_ids.len();
        if let Some(bytes) = rewritten_manifest
            && let Some(id) = self.manifest_entry_id()?
        {
            self.replace_entry_by_id(id, bytes)?;
        }
        if !signature_ids.is_empty() {
            self.entries
                .retain(|entry| !signature_ids.contains(&entry.id));
            self.dirty = true;
        }
        Ok(removed)
    }
}

fn strip_prefix_ascii_case<'a>(value: &'a str, prefix: &str) -> Option<&'a str> {
    let candidate = value.get(..prefix.len())?;
    candidate
        .eq_ignore_ascii_case(prefix)
        .then(|| &value[prefix.len()..])
}

#[cfg(test)]
mod tests {
    use super::is_signature_entry;

    #[test]
    fn recognizes_only_top_level_signature_artifacts() {
        assert!(is_signature_entry("META-INF/EXAMPLE.SF"));
        assert!(is_signature_entry("meta-inf/SIG-CUSTOM"));
        assert!(is_signature_entry("META-INF/EXAMPLE.RSA"));
        assert!(!is_signature_entry("META-INF/MANIFEST.MF"));
        assert!(!is_signature_entry("META-INF/sub/EXAMPLE.SF"));
        assert!(!is_signature_entry("example.sf"));
    }
}

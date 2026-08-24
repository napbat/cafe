//! Java service-provider configuration resources.

use std::collections::HashSet;

use crate::{Error, Result};

use super::reader::EntryReader;
use super::{EntryId, EntryKind, JarFile};

/// Archive prefix containing `ServiceLoader` provider configurations.
pub const SERVICE_PREFIX: &str = "META-INF/services/";

/// Parsed service-provider configuration from one JAR entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceConfiguration {
    /// Stable identity of the physical configuration entry.
    pub entry_id: EntryId,
    /// Binary name of the service type.
    pub service: String,
    /// Ordered, de-duplicated provider binary names.
    pub providers: Vec<String>,
}

/// Returns whether an entry names a non-empty service configuration.
#[must_use]
pub fn is_service_entry(name: &str) -> bool {
    name.strip_prefix(SERVICE_PREFIX)
        .is_some_and(|service| !service.is_empty() && !service.contains('/'))
}

impl JarFile {
    /// Parses every service-provider configuration in archive order.
    ///
    /// # Errors
    ///
    /// Returns an error for unreadable, non-UTF-8, duplicated, or malformed
    /// provider configuration entries.
    pub fn service_configurations(&self) -> Result<Vec<ServiceConfiguration>> {
        let mut configurations = Vec::new();
        let mut services = HashSet::new();
        let mut reader = EntryReader::new(self);
        for entry in &self.entries {
            if entry.kind != EntryKind::File || !is_service_entry(&entry.name) {
                continue;
            }
            let service = entry
                .name
                .strip_prefix(SERVICE_PREFIX)
                .ok_or_else(|| Error::InvalidJar("invalid service entry prefix".to_owned()))?;
            validate_binary_name(service, "service")?;
            if !services.insert(service) {
                return Err(Error::AmbiguousJarEntry {
                    name: entry.name.clone(),
                    count: 2,
                });
            }
            configurations.push(ServiceConfiguration {
                entry_id: entry.id,
                service: service.to_owned(),
                providers: parse_providers(
                    &reader
                        .read(entry)
                        .map_err(|error| error.in_jar_entry(entry.name.clone()))?,
                    &entry.name,
                )?,
            });
        }
        Ok(configurations)
    }

    /// Reads providers for one service binary name.
    ///
    /// # Errors
    ///
    /// Returns an error if the service name is invalid or its configuration is
    /// ambiguous, unreadable, non-UTF-8, or malformed.
    pub fn service_providers(&self, service: &str) -> Result<Option<Vec<String>>> {
        let entry_name = service_entry_name(service)?;
        let matches = self.entry_ids_named(&entry_name);
        match matches.as_slice() {
            [] => Ok(None),
            [id] => Ok(Some(parse_providers(
                &self.read_entry_by_id(*id)?,
                &entry_name,
            )?)),
            _ => Err(Error::AmbiguousJarEntry {
                name: entry_name,
                count: matches.len(),
            }),
        }
    }

    /// Adds or replaces a service-provider configuration.
    ///
    /// Provider order is retained and duplicate names are written once.
    ///
    /// # Errors
    ///
    /// Returns an error for invalid service/provider names or an ambiguous
    /// existing configuration.
    pub fn set_service_providers<I, S>(&mut self, service: &str, providers: I) -> Result<EntryId>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let entry_name = service_entry_name(service)?;
        let providers = normalize_providers(providers)?;
        let bytes = render_providers(&providers);
        self.put_file(entry_name, bytes)
    }

    /// Adds one provider unless it is already configured.
    ///
    /// # Errors
    ///
    /// Returns an error for invalid names or an unreadable or ambiguous
    /// existing configuration.
    pub fn add_service_provider(&mut self, service: &str, provider: &str) -> Result<EntryId> {
        validate_binary_name(provider, "provider")?;
        let mut providers = self.service_providers(service)?.unwrap_or_default();
        if !providers.iter().any(|existing| existing == provider) {
            providers.push(provider.to_owned());
        }
        self.set_service_providers(service, providers)
    }

    /// Removes one provider from a service configuration.
    ///
    /// The configuration entry is retained when its provider list becomes
    /// empty.
    ///
    /// # Errors
    ///
    /// Returns an error for invalid names or an unreadable or ambiguous
    /// existing configuration.
    pub fn remove_service_provider(&mut self, service: &str, provider: &str) -> Result<bool> {
        validate_binary_name(provider, "provider")?;
        let Some(mut providers) = self.service_providers(service)? else {
            return Ok(false);
        };
        let old_len = providers.len();
        providers.retain(|existing| existing != provider);
        if providers.len() == old_len {
            return Ok(false);
        }
        self.set_service_providers(service, providers)?;
        Ok(true)
    }

    /// Removes and returns a service configuration when present.
    ///
    /// # Errors
    ///
    /// Returns an error for an invalid service name or an ambiguous or
    /// unreadable existing entry.
    pub fn remove_service_configuration(&mut self, service: &str) -> Result<Option<Vec<String>>> {
        let entry_name = service_entry_name(service)?;
        let Some(providers) = self.service_providers(service)? else {
            return Ok(None);
        };
        self.remove_entry(&entry_name)?;
        Ok(Some(providers))
    }
}

fn service_entry_name(service: &str) -> Result<String> {
    validate_binary_name(service, "service")?;
    Ok(format!("{SERVICE_PREFIX}{service}"))
}

pub(super) fn parse_providers(bytes: &[u8], entry: &str) -> Result<Vec<String>> {
    let text = std::str::from_utf8(bytes).map_err(|_| Error::UnsupportedJarEntry {
        entry: entry.to_owned(),
        message: "service configuration is not UTF-8".to_owned(),
    })?;
    let mut providers = Vec::new();
    let mut seen = HashSet::new();
    for (index, line) in text.lines().enumerate() {
        let provider = line
            .split_once('#')
            .map_or(line, |(prefix, _)| prefix)
            .trim();
        if provider.is_empty() {
            continue;
        }
        validate_binary_name(provider, "provider").map_err(|error| {
            Error::InvalidJar(format!(
                "service configuration `{entry}` line {}: {error}",
                index + 1
            ))
        })?;
        if seen.insert(provider) {
            providers.push(provider.to_owned());
        }
    }
    Ok(providers)
}

fn normalize_providers<I, S>(providers: I) -> Result<Vec<String>>
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
{
    let mut result = Vec::new();
    let mut seen = HashSet::new();
    for provider in providers {
        let provider = provider.into();
        validate_binary_name(&provider, "provider")?;
        if seen.insert(provider.clone()) {
            result.push(provider);
        }
    }
    Ok(result)
}

fn render_providers(providers: &[String]) -> Vec<u8> {
    let mut output = providers.join("\n").into_bytes();
    if !providers.is_empty() {
        output.push(b'\n');
    }
    output
}

pub(super) fn validate_binary_name(name: &str, role: &str) -> Result<()> {
    let invalid = name.is_empty()
        || name.starts_with('.')
        || name.ends_with('.')
        || name.contains("..")
        || name.chars().any(|character| {
            character.is_whitespace() || matches!(character, '/' | ';' | '[' | '#')
        });
    if invalid {
        return Err(Error::InvalidJar(format!(
            "invalid Java {role} binary name `{name}`"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{parse_providers, render_providers};

    #[test]
    fn parses_comments_whitespace_and_duplicates() {
        let providers = parse_providers(
            b" example.First # comment\n\nexample.Second\r\nexample.First\n",
            "META-INF/services/example.Service",
        )
        .expect("valid providers");
        assert_eq!(providers, ["example.First", "example.Second"]);
        assert_eq!(
            render_providers(&providers),
            b"example.First\nexample.Second\n"
        );
    }
}

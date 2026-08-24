//! Full-archive class-file and bytecode validation.

use std::collections::BTreeMap;
use std::collections::HashSet;

use crate::bytecode::{self, Operand};
use crate::classfile::ClassFile;
use crate::descriptor;
use crate::{Error, Result};

use super::entry::validate_entry_name;
use super::{
    EntryKind, JarFile, SERVICE_PREFIX, is_class_entry, is_service_entry, parse_versioned_entry,
};

/// Aggregate results from structurally validating every JAR entry.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ArchiveValidationReport {
    /// Total number of physical ZIP members.
    pub entries: usize,
    /// Number of regular files.
    pub files: usize,
    /// Number of directory markers.
    pub directories: usize,
    /// Number of Unix symbolic links.
    pub symlinks: usize,
    /// Total uncompressed payload bytes read and CRC-checked.
    pub uncompressed_bytes: u64,
    /// Number of JVM class-file entries.
    pub class_entries: usize,
    /// Number of service-provider configuration entries.
    pub service_configurations: usize,
    /// Number of standard top-level signature artifacts.
    pub signature_artifacts: usize,
    /// Whether the manifest enables multi-release lookup.
    pub multi_release: bool,
}

/// Aggregate results from parsing and decoding every class in a JAR.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ValidationReport {
    /// Number of parsed `.class` entries.
    pub classes: usize,
    /// Total uncompressed bytes occupied by class entries.
    pub class_bytes: u64,
    /// Number of field declarations.
    pub fields: usize,
    /// Number of method declarations.
    pub methods: usize,
    /// Number of methods containing bytecode.
    pub code_methods: usize,
    /// Number of decoded JVM instructions.
    pub instructions: usize,
    /// Number of shared control-flow graphs constructed through cfglib.
    pub control_flow_graphs: usize,
    /// Total number of basic blocks across the constructed graphs.
    pub basic_blocks: usize,
    /// Total number of ordinary and exceptional control-flow edges.
    pub control_flow_edges: usize,
    /// Class count grouped by class-file major version.
    pub major_versions: BTreeMap<u16, usize>,
}

impl JarFile {
    /// Validates archive names, uniqueness, entry payloads, manifest structure,
    /// service configurations, and multi-release paths.
    ///
    /// Reading each payload also verifies decompression and CRC-32 through the
    /// ZIP reader.
    ///
    /// # Errors
    ///
    /// Returns an error identifying the first unsafe, duplicate, malformed,
    /// encrypted, unreadable, or unsupported entry.
    pub fn validate_archive(&self) -> Result<ArchiveValidationReport> {
        let mut report = ArchiveValidationReport {
            entries: self.entries.len(),
            signature_artifacts: self.signature_entry_ids().len(),
            ..ArchiveValidationReport::default()
        };
        let mut names = HashSet::with_capacity(self.entries.len());
        for entry in &self.entries {
            validate_entry_name(&entry.name, entry.kind)?;
            if !names.insert(&entry.name) {
                return Err(Error::DuplicateJarEntry(entry.name.clone()));
            }
            if entry.encrypted {
                return Err(Error::UnsupportedJarEntry {
                    entry: entry.name.clone(),
                    message: "encrypted members are not valid portable JAR entries".to_owned(),
                });
            }
            let bytes = self
                .read_entry_by_id(entry.id)
                .map_err(|error| error.in_jar_entry(entry.name.clone()))?;
            report.uncompressed_bytes =
                report
                    .uncompressed_bytes
                    .checked_add(bytes.len() as u64)
                    .ok_or_else(|| Error::InvalidJar("archive byte count overflow".to_owned()))?;
            match entry.kind {
                EntryKind::File => report.files += 1,
                EntryKind::Directory => {
                    report.directories += 1;
                    if !bytes.is_empty() {
                        return Err(Error::UnsupportedJarEntry {
                            entry: entry.name.clone(),
                            message: "directory marker contains payload bytes".to_owned(),
                        });
                    }
                }
                EntryKind::Symlink => {
                    report.symlinks += 1;
                    std::str::from_utf8(&bytes).map_err(|_| Error::UnsupportedJarEntry {
                        entry: entry.name.clone(),
                        message: "symbolic-link target is not UTF-8".to_owned(),
                    })?;
                }
            }
            report.class_entries +=
                usize::from(entry.kind == EntryKind::File && is_class_entry(&entry.name));
            report.service_configurations += usize::from(is_service_entry(&entry.name));
            if entry.kind == EntryKind::File
                && entry.name.starts_with("META-INF/versions/")
                && parse_versioned_entry(&entry.name).is_none()
            {
                return Err(Error::InvalidJar(format!(
                    "malformed multi-release entry `{}`",
                    entry.name
                )));
            }
            if entry.kind == EntryKind::File
                && entry.name.starts_with(SERVICE_PREFIX)
                && !is_service_entry(&entry.name)
            {
                return Err(Error::InvalidJar(format!(
                    "malformed service configuration entry `{}`",
                    entry.name
                )));
            }
        }
        self.manifest()?;
        self.service_configurations()?;
        report.multi_release = self.is_multi_release()?;
        Ok(report)
    }

    /// Parses every class and decodes every method body in archive order.
    ///
    /// # Errors
    ///
    /// Returns an error identifying the first unreadable or invalid class entry.
    pub fn validate_all(&self) -> Result<ValidationReport> {
        self.validate_archive()?;
        let mut report = ValidationReport::default();
        for entry in &self.entries {
            if entry.kind != EntryKind::File || !is_class_entry(&entry.name) {
                continue;
            }
            let name = entry.name.clone();
            let bytes = self.read_entry_by_id(entry.id)?;
            let size = bytes.len() as u64;

            let class = validate_class_round_trip(&bytes, &name)?;
            record_class(&mut report, &class, size);
            validate_fields(&class, &name)?;
            validate_methods(&class, &name, &mut report)?;
            validate_control_flow(&class, &name, &mut report)?;
        }
        Ok(report)
    }
}

fn validate_class_round_trip(bytes: &[u8], entry: &str) -> Result<ClassFile> {
    let class = ClassFile::parse(bytes).map_err(|error| error.in_jar_entry(entry))?;
    let assembled = class
        .to_bytes()
        .map_err(|error| error.in_jar_entry(entry))?;
    if assembled != bytes {
        return Err(Error::invalid_assembly(
            "parse/assemble round trip changed the class-file bytes",
        )
        .in_jar_entry(entry));
    }
    class
        .class_name()
        .map_err(|error| error.in_jar_entry(entry))?;
    Ok(class)
}

fn record_class(report: &mut ValidationReport, class: &ClassFile, size: u64) {
    report.classes += 1;
    report.class_bytes += size;
    report.fields += class.fields.len();
    report.methods += class.methods.len();
    *report
        .major_versions
        .entry(class.major_version)
        .or_default() += 1;
}

fn validate_fields(class: &ClassFile, entry: &str) -> Result<()> {
    for field in &class.fields {
        let descriptor = field
            .descriptor(&class.constant_pool)
            .map_err(|error| error.in_jar_entry(entry))?;
        descriptor::parse_field(descriptor).map_err(|error| error.in_jar_entry(entry))?;
    }
    Ok(())
}

fn validate_methods(class: &ClassFile, entry: &str, report: &mut ValidationReport) -> Result<()> {
    let owner = class
        .class_name()
        .map_err(|error| error.in_jar_entry(entry))?
        .to_owned();
    for method in &class.methods {
        let name = method
            .name(&class.constant_pool)
            .map_err(|error| error.in_jar_entry(entry))?
            .to_owned();
        let descriptor = method
            .descriptor(&class.constant_pool)
            .map_err(|error| error.in_jar_entry(entry))?
            .to_owned();
        descriptor::parse_method(&descriptor).map_err(|error| {
            error
                .in_class_method(&owner, &name, &descriptor)
                .in_jar_entry(entry)
        })?;
        if let Some(code) = method.code() {
            let instructions = bytecode::decode_code(code).map_err(|error| {
                error
                    .in_class_method(&owner, &name, &descriptor)
                    .in_jar_entry(entry)
            })?;
            let encoded = bytecode::encode(&instructions).map_err(|error| {
                error
                    .in_class_method(&owner, &name, &descriptor)
                    .in_jar_entry(entry)
            })?;
            if encoded != code.code {
                return Err(Error::invalid_assembly(
                    "decode/encode round trip changed the method bytecode",
                )
                .in_class_method(&owner, &name, &descriptor)
                .in_jar_entry(entry));
            }
            validate_constant_references(class, &instructions, entry, &owner, &name, &descriptor)?;
            report.code_methods += 1;
            report.instructions += instructions.len();
        }
    }
    Ok(())
}

fn validate_constant_references(
    class: &ClassFile,
    instructions: &[bytecode::Instruction],
    entry: &str,
    owner: &str,
    method: &str,
    descriptor: &str,
) -> Result<()> {
    for instruction in instructions {
        if let Some(index) = referenced_constant(&instruction.operand) {
            class.constant_pool.describe(index).map_err(|error| {
                error
                    .in_class_method(owner, method, descriptor)
                    .in_jar_entry(entry)
            })?;
        }
    }
    Ok(())
}

const fn referenced_constant(operand: &Operand) -> Option<u16> {
    match operand {
        Operand::Constant(index)
        | Operand::InvokeDynamic(index)
        | Operand::InvokeInterface { index, .. }
        | Operand::MultiArray { index, .. } => Some(*index),
        Operand::None
        | Operand::Byte(_)
        | Operand::Short(_)
        | Operand::Local(_)
        | Operand::Increment { .. }
        | Operand::Branch(_)
        | Operand::TableSwitch { .. }
        | Operand::LookupSwitch { .. }
        | Operand::ArrayType(_) => None,
    }
}

fn validate_control_flow(
    class: &ClassFile,
    entry: &str,
    report: &mut ValidationReport,
) -> Result<()> {
    let disassembly =
        crate::disassembly::lower_class(class).map_err(|error| error.in_jar_entry(entry))?;
    for function in disassembly.functions {
        if let Some(body) = function.body {
            let graph = body
                .control_flow_graph()
                .map_err(|error| Error::from(error).in_jar_entry(entry))?;
            report.control_flow_graphs += 1;
            report.basic_blocks += graph.cfg().num_blocks();
            report.control_flow_edges += graph.cfg().num_edges();
        }
    }
    Ok(())
}

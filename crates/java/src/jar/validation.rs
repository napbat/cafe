//! Full-archive class-file and bytecode validation.

use std::collections::BTreeMap;

use crate::bytecode::{self, Operand};
use crate::classfile::ClassFile;
use crate::descriptor;
use crate::{Error, Result};

use super::{JarFile, is_class_entry, read_zip_file};

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
    /// Parses every class and decodes every method body in archive order.
    ///
    /// # Errors
    ///
    /// Returns an error identifying the first unreadable or invalid class entry.
    pub fn validate_all(&mut self) -> Result<ValidationReport> {
        let mut report = ValidationReport::default();
        for index in 0..self.archive.len() {
            let (name, size, bytes) = {
                let mut file = self.archive.by_index(index)?;
                if file.is_dir() || !is_class_entry(file.name()) {
                    continue;
                }
                let name = file.name().to_owned();
                let size = file.size();
                let bytes = read_zip_file(&mut file)?;
                (name, size, bytes)
            };

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
    for method in &class.methods {
        method
            .name(&class.constant_pool)
            .map_err(|error| error.in_jar_entry(entry))?;
        let descriptor = method
            .descriptor(&class.constant_pool)
            .map_err(|error| error.in_jar_entry(entry))?;
        descriptor::parse_method(descriptor).map_err(|error| error.in_jar_entry(entry))?;
        if let Some(code) = method.code() {
            let instructions =
                bytecode::decode_code(code).map_err(|error| error.in_jar_entry(entry))?;
            let encoded =
                bytecode::encode(&instructions).map_err(|error| error.in_jar_entry(entry))?;
            if encoded != code.code {
                return Err(Error::invalid_assembly(
                    "decode/encode round trip changed the method bytecode",
                )
                .in_jar_entry(entry));
            }
            validate_constant_references(class, &instructions, entry)?;
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
) -> Result<()> {
    for instruction in instructions {
        if let Some(index) = referenced_constant(&instruction.operand) {
            class
                .constant_pool
                .describe(index)
                .map_err(|error| error.in_jar_entry(entry))?;
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

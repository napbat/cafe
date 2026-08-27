//! Command-line access to Cafe's Java ecosystem tooling.

mod cli;
mod decompile;
mod error;
mod output;

use std::process::ExitCode;

use cafe::decompiler::DiagnosticSeverity;
use clap::Parser;

use crate::cli::{Cli, Command, DecompileArtifact};
use crate::decompile::RunReport;

const DEFAULT_DIAGNOSTIC_DISPLAY_LIMIT: usize = 100;

fn main() -> ExitCode {
    let cli = Cli::parse();
    let all_diagnostics = cli.all_diagnostics;
    match run(cli) {
        Ok(report) => render_report(&report, all_diagnostics),
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: Cli) -> error::Result<RunReport> {
    match cli.command {
        Command::Decompile(command) => match command.artifact {
            DecompileArtifact::Jar(command) => decompile::jar(&command),
        },
    }
}

fn render_report(report: &RunReport, all_diagnostics: bool) -> ExitCode {
    for notice in &report.notices {
        eprintln!("warning[{}]: {}", notice.scope, notice.message);
    }
    let display_limit = if all_diagnostics {
        report.diagnostics.len()
    } else {
        DEFAULT_DIAGNOSTIC_DISPLAY_LIMIT
    };
    for item in report.diagnostics.iter().take(display_limit) {
        let diagnostic = &item.diagnostic;
        let severity = match diagnostic.severity {
            DiagnosticSeverity::Warning => "warning",
            DiagnosticSeverity::Error => "error",
        };
        let method = diagnostic
            .method
            .as_ref()
            .map_or_else(String::new, |method| {
                format!("#{}{}", method.name, method.descriptor)
            });
        eprintln!(
            "{severity}[{:?}] {}{method} ({}): {}",
            diagnostic.code, diagnostic.class_name, item.entry, diagnostic.message
        );
    }
    let omitted = report.diagnostics.len().saturating_sub(display_limit);
    if omitted != 0 {
        eprintln!(
            "warning: {omitted} additional diagnostics omitted; pass --all-diagnostics to print them"
        );
    }
    for failure in &report.failures {
        eprintln!(
            "error[{}] {}: {}",
            failure.stage.name(),
            failure.entry,
            failure.message
        );
    }
    println!(
        "decompiled {} of {} selected classes to `{}` ({} skipped, {} diagnostics, {} failed)",
        report.written,
        report.selected,
        report.output.display(),
        report.skipped,
        report.diagnostics.len(),
        report.failures.len()
    );
    if report.is_complete() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

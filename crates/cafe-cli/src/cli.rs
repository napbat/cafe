//! Command-line syntax.

use std::num::NonZeroUsize;
use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(
    name = "cafe",
    version,
    about = "Analyze and decompile Java ecosystem bytecode",
    arg_required_else_help = true
)]
pub(crate) struct Cli {
    /// Print every recovery diagnostic instead of the bounded console sample.
    #[arg(long, global = true)]
    pub(crate) all_diagnostics: bool,

    #[command(subcommand)]
    pub(crate) command: Command,
}

#[derive(Debug, Subcommand)]
pub(crate) enum Command {
    /// Recover higher-level artifacts from bytecode.
    Decompile(DecompileCommand),
}

#[derive(Debug, Args)]
pub(crate) struct DecompileCommand {
    #[command(subcommand)]
    pub(crate) artifact: DecompileArtifact,
}

#[derive(Debug, Subcommand)]
pub(crate) enum DecompileArtifact {
    /// Decompile the effective class view of a JAR into Java source files.
    Jar(JarCommand),
}

#[derive(Debug, Args)]
pub(crate) struct JarCommand {
    /// JAR archive to decompile.
    #[arg(value_name = "JAR")]
    pub(crate) input: PathBuf,

    /// Directory that will receive package-qualified `.java` files.
    #[arg(short, long, value_name = "DIRECTORY")]
    pub(crate) output: PathBuf,

    /// Target Java release used to select multi-release JAR entries.
    ///
    /// When omitted, Cafe selects the newest version present in the archive.
    #[arg(long, value_name = "JAVA_RELEASE")]
    pub(crate) release: Option<u16>,

    /// Overwrite existing regular source files.
    #[arg(long)]
    pub(crate) force: bool,

    /// Omit fields and methods marked synthetic by the class file.
    #[arg(long)]
    pub(crate) exclude_synthetic: bool,

    /// Render every executable method through the exact state-machine form.
    #[arg(long)]
    pub(crate) state_machine: bool,

    /// Number of classes to decompile concurrently.
    ///
    /// When omitted, Cafe selects a bounded value from the available CPUs.
    #[arg(short, long, value_name = "JOBS")]
    pub(crate) jobs: Option<NonZeroUsize>,
}

#[cfg(test)]
mod tests {
    use std::num::NonZeroUsize;

    use clap::{CommandFactory, Parser};

    use super::{Cli, Command, DecompileArtifact};

    #[test]
    fn clap_contract_is_valid() {
        Cli::command().debug_assert();
    }

    #[test]
    fn parses_jar_decompilation_options() {
        let cli = Cli::try_parse_from([
            "cafe",
            "--all-diagnostics",
            "decompile",
            "jar",
            "application.jar",
            "--output",
            "source",
            "--release",
            "17",
            "--force",
            "--exclude-synthetic",
            "--state-machine",
            "--jobs",
            "4",
        ])
        .expect("valid command line");

        assert!(cli.all_diagnostics);
        let Command::Decompile(command) = cli.command;
        let DecompileArtifact::Jar(command) = command.artifact;
        assert_eq!(command.input.to_string_lossy(), "application.jar");
        assert_eq!(command.output.to_string_lossy(), "source");
        assert_eq!(command.release, Some(17));
        assert!(command.force);
        assert!(command.exclude_synthetic);
        assert!(command.state_machine);
        assert_eq!(command.jobs.map(NonZeroUsize::get), Some(4));
    }
}

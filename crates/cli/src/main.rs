#![forbid(unsafe_code)]

mod bellhop;
mod kraken;

use std::process::ExitCode;

use clap::{Parser, Subcommand, ValueEnum};

#[derive(Clone, Copy, Debug, ValueEnum)]
enum Format {
    Legacy,
    Hdf5,
    Both,
}

impl From<Format> for output::directory::Format {
    fn from(value: Format) -> Self {
        match value {
            Format::Legacy => Self::Legacy,
            Format::Hdf5 => Self::Hdf5,
            Format::Both => Self::Both,
        }
    }
}

#[derive(Debug, Parser)]
#[command(
    name = "pelagic",
    version,
    about = "Two-dimensional underwater acoustics"
)]
struct Cli {
    #[arg(long, global = true, value_enum, default_value = "text")]
    diagnostics: Diagnostics,
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Report public protocol versions and the pinned numerical reference.
    Version {
        #[arg(long)]
        json: bool,
    },
    /// BELLHOP ray, arrival and pressure-field calculations.
    #[command(subcommand)]
    Bellhop(bellhop::Command),
    /// KRAKEN/KRAKENC normal modes and FIELD calculations.
    #[command(subcommand)]
    Kraken(kraken::Command),
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum Diagnostics {
    Text,
    Json,
}

impl Diagnostics {
    fn render<T: serde::Serialize + std::fmt::Display>(self, diagnostics: &[T]) {
        if diagnostics.is_empty() {
            return;
        }
        match self {
            Self::Text => {
                for diagnostic in diagnostics {
                    eprintln!("{diagnostic}");
                }
            }
            Self::Json => eprintln!("{}", serde_json::json!({"diagnostics": diagnostics})),
        }
    }
}

fn main() -> ExitCode {
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(error) => {
            let args: Vec<_> = std::env::args_os().collect();
            let json = args.iter().any(|a| a == "--diagnostics=json")
                || args
                    .windows(2)
                    .any(|a| a[0] == "--diagnostics" && a[1] == "json");
            if json && error.use_stderr() {
                eprintln!(
                    "{}",
                    serde_json::json!({"diagnostics": [{"code":"PG0002", "severity":"error", "field":"arguments", "message":error.to_string(), "source":null}]})
                );
            } else {
                let _ = error.print();
            }
            return ExitCode::from(if error.use_stderr() { 2 } else { 0 });
        }
    };
    let (exit_code, warnings) = output::diagnostic::capture(|| match cli.command {
        Command::Bellhop(command) => bellhop::execute(command, cli.diagnostics),
        Command::Kraken(command) => kraken::execute(command, cli.diagnostics),
        Command::Version { json } => {
            if json {
                println!(
                    "{}",
                    serde_json::json!({
                        "version": env!("CARGO_PKG_VERSION"), "diagnostics_version": 1,
                        "case_schema_versions": {"bellhop": ::bellhop::json::SCHEMA_VERSION, "kraken": ::kraken::json::SCHEMA_VERSION},
                        "hdf5_schema_versions": {"bellhop": output::bellhop::SCHEMA_VERSION, "kraken": output::kraken::SCHEMA_VERSION},
                        "compatibility_reference": "Acoustics Toolbox v2023.5",
                        "reference_commit": "475108519289c6fb488b58980c644ea14eccc604"
                    })
                );
            } else {
                println!("pelagic {}", env!("CARGO_PKG_VERSION"));
            }
            ExitCode::SUCCESS
        }
    });
    cli.diagnostics.render(&warnings);
    exit_code
}

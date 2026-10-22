#![forbid(unsafe_code)]

use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use crate::Diagnostics;
use bellhop::Case;
use bellhop::diagnostic::{Diagnostic, DiagnosticReport, LoadOutcome, SourceLocation};
use bellhop::solver::{SimulationLimits, run as run_simulation};
use clap::Subcommand;

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Validate a JSON/legacy case. Use - for self-contained JSON on stdin.
    Validate { case: PathBuf },
    /// Run a JSON/legacy case and safely update a result directory.
    Run {
        /// Input path, or - for self-contained JSON on stdin.
        case: PathBuf,
        /// Result directory; stdin defaults to `stdin`.
        #[arg(short, long)]
        output: Option<PathBuf>,
        #[arg(long, value_enum, default_value = "legacy")]
        format: crate::Format,
    },
    /// Export a JSON/legacy case as self-contained JSON; accepts - on stdin.
    Export { case: PathBuf },
}

#[allow(clippy::too_many_lines)] // Keep command dispatch and its exit-code handling together.
pub fn execute(command: Command, diagnostics: Diagnostics) -> ExitCode {
    let path = match &command {
        Command::Validate { case } | Command::Run { case, .. } | Command::Export { case } => case,
    };
    let input = match load_case(path) {
        Ok(input) => input,
        Err(report) => {
            diagnostics.render(report.diagnostics());
            return ExitCode::from(2);
        }
    };
    diagnostics.render(&input.outcome.warnings);
    let case = &input.outcome.value;
    match command {
        Command::Validate { .. } => {
            println!(
                "valid 2D BELLHOP case: {} ({:?}, {} launch angles)",
                input.path.display(),
                case.environment.run.kind,
                case.environment.trace.launch_angles_degrees.len()
            );
            ExitCode::SUCCESS
        }
        Command::Export { .. } => {
            let document = match bellhop::json::export_case_document(case) {
                Ok(document) => document,
                Err(report) => {
                    diagnostics.render(report.diagnostics());
                    return ExitCode::from(2);
                }
            };
            let mut stdout = io::stdout().lock();
            match serde_json::to_writer_pretty(&mut stdout, &document)
                .and_then(|()| writeln!(stdout).map_err(serde_json::Error::io))
            {
                Ok(()) => ExitCode::SUCCESS,
                Err(error) => output_error(
                    diagnostics,
                    &input.path,
                    &format!("unable to write JSON output: {error}"),
                ),
            }
        }
        Command::Run { output, format, .. } => {
            let output_path =
                output.unwrap_or_else(|| PathBuf::from(input.path.file_stem().unwrap_or_default()));
            let result = match run_simulation(case, SimulationLimits::default()) {
                Ok(result) => result,
                Err(report) => {
                    diagnostics.render(report.diagnostics());
                    return ExitCode::from(3);
                }
            };
            let stem = match output::directory::stem(&input.path) {
                Ok(stem) => stem,
                Err(error) => return output_error(diagnostics, &output_path, &error),
            };
            let filename = input.path.file_name().unwrap_or_default().to_string_lossy();
            if let Err(error) = output::bellhop::write_directory(
                &output_path,
                &stem,
                case,
                &filename,
                &input.bytes,
                &result,
                &input.outcome.warnings,
                &input.paths.iter().map(PathBuf::as_path).collect::<Vec<_>>(),
                format.into(),
            ) {
                return output_error(diagnostics, &output_path, &error);
            }
            let ray_count: usize = result.sources.iter().map(|s| s.rays.len()).sum();
            let eigenray_count: usize = result
                .eigenray_sources
                .iter()
                .flat_map(|s| &s.receivers)
                .map(|r| r.eigenrays.len())
                .sum();
            let arrival_count: usize = result
                .arrival_sources
                .iter()
                .flat_map(|s| &s.receivers)
                .map(|r| r.arrivals.len())
                .sum();
            let field_sample_count: usize =
                result.field_sources.iter().map(|s| s.samples.len()).sum();
            let point_count: usize = result
                .sources
                .iter()
                .flat_map(|s| &s.rays)
                .chain(
                    result
                        .eigenray_sources
                        .iter()
                        .flat_map(|s| &s.receivers)
                        .flat_map(|r| &r.eigenrays),
                )
                .map(|r| r.points.len())
                .sum();
            println!(
                "wrote {} ({ray_count} rays, {eigenray_count} eigenrays, {arrival_count} arrivals, {field_sample_count} field samples, {point_count} trajectory points)",
                output_path.display()
            );
            ExitCode::SUCCESS
        }
    }
}

fn output_error(diagnostics: Diagnostics, path: &Path, message: &str) -> ExitCode {
    diagnostics.render(&[Diagnostic::error(
        "BH0402",
        message,
        "output",
        SourceLocation::file(path),
    )]);
    ExitCode::from(4)
}

struct LoadedInput {
    path: PathBuf,
    outcome: LoadOutcome<Case>,
    bytes: Vec<u8>,
    paths: Vec<PathBuf>,
}

fn load_case(path: &Path) -> Result<LoadedInput, DiagnosticReport> {
    let stdin = path == Path::new("-");
    if stdin
        || path
            .extension()
            .is_some_and(|extension| extension == "json")
    {
        let source = if stdin { Path::new("stdin.json") } else { path };
        let bytes = if stdin {
            let mut bytes = Vec::new();
            io::stdin().read_to_end(&mut bytes).map(|_| bytes)
        } else {
            fs::read(path)
        }
        .map_err(|error| DiagnosticReport::from_diagnostic(Diagnostic::io(source, &error)))?;
        let outcome = bellhop::json::load_case_document_named(&bytes, source)
            .map_err(bellhop::json::DocumentError::into_report)?;
        Ok(LoadedInput {
            path: source.to_path_buf(),
            outcome,
            bytes,
            paths: if stdin {
                vec![]
            } else {
                vec![path.to_path_buf()]
            },
        })
    } else {
        let (outcome, source, paths) = bellhop::legacy::load_case_with_inputs(path)?.into_parts();
        Ok(LoadedInput {
            path: path.to_path_buf(),
            outcome,
            bytes: source.into_bytes(),
            paths,
        })
    }
}

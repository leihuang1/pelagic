#![forbid(unsafe_code)]

use std::fs;
use std::io::{self, Read, Write};
use std::num::NonZeroUsize;
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
    /// Run many cases, each into its own result directory, recording each in a status file.
    RunMany(crate::many::Args),
    /// Export a JSON/legacy case as self-contained JSON; accepts - on stdin.
    Export { case: PathBuf },
}

pub fn execute(
    command: Command,
    diagnostics: Diagnostics,
    threads: Option<NonZeroUsize>,
) -> ExitCode {
    match command {
        Command::Validate { case } => validate(&case, diagnostics),
        Command::Run {
            case,
            output,
            format,
        } => match run(&case, output, format, &mut |d| diagnostics.render(d)) {
            Ok(summary) => {
                println!("{summary}");
                ExitCode::SUCCESS
            }
            Err(code) => ExitCode::from(code),
        },
        Command::RunMany(args) => {
            let model = Many {
                format: args.format,
            };
            crate::many::execute(&args, threads, diagnostics, &model)
        }
        Command::Export { case } => export(&case, diagnostics),
    }
}

/// Load a case, printing its diagnostics.
fn load_rendered(path: &Path, diagnostics: Diagnostics) -> Result<LoadedInput, ExitCode> {
    let input = load_case(path).map_err(|report| {
        diagnostics.render(report.diagnostics());
        ExitCode::from(2)
    })?;
    diagnostics.render(&input.outcome.warnings);
    Ok(input)
}

fn validate(path: &Path, diagnostics: Diagnostics) -> ExitCode {
    let input = match load_rendered(path, diagnostics) {
        Ok(input) => input,
        Err(code) => return code,
    };
    let case = &input.outcome.value;
    println!(
        "valid 2D BELLHOP case: {} ({:?}, {} launch angles)",
        input.path.display(),
        case.environment.run.kind,
        case.environment.trace.launch_angles_degrees.len()
    );
    ExitCode::SUCCESS
}

fn export(path: &Path, diagnostics: Diagnostics) -> ExitCode {
    let input = match load_rendered(path, diagnostics) {
        Ok(input) => input,
        Err(code) => return code,
    };
    let bytes = match exported(&input) {
        Ok(bytes) => bytes,
        Err(report) => {
            diagnostics.render(report.diagnostics());
            return ExitCode::from(2);
        }
    };
    match io::stdout().lock().write_all(&bytes) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            diagnostics.render(&[output_error(
                &input.path,
                &format!("unable to write JSON output: {error}"),
            )]);
            ExitCode::from(4)
        }
    }
}

/// The self-contained JSON that `export` prints.
fn exported(input: &LoadedInput) -> Result<Vec<u8>, DiagnosticReport> {
    let document = bellhop::json::export_case_document(&input.outcome.value)?;
    let mut bytes = serde_json::to_vec_pretty(&document).map_err(|error| {
        DiagnosticReport::from_diagnostic(output_error(
            &input.path,
            &format!("unable to encode JSON output: {error}"),
        ))
    })?;
    bytes.push(b'\n');
    Ok(bytes)
}

/// Solve a case and publish it to `output`, by default the input's stem, as
/// `run` does. Diagnostics go to `emit` as they arise. Returns the summary
/// line, or the exit code.
fn run(
    path: &Path,
    output: Option<PathBuf>,
    format: crate::Format,
    emit: &mut dyn FnMut(&[Diagnostic]),
) -> Result<String, u8> {
    let input = load_case(path).map_err(|report| {
        emit(report.diagnostics());
        2
    })?;
    solve(&input, output, format, emit)
}

/// The rest of `run`, once its input is loaded.
fn solve(
    input: &LoadedInput,
    output: Option<PathBuf>,
    format: crate::Format,
    emit: &mut dyn FnMut(&[Diagnostic]),
) -> Result<String, u8> {
    emit(&input.outcome.warnings);
    let case = &input.outcome.value;
    let output_path =
        output.unwrap_or_else(|| PathBuf::from(input.path.file_stem().unwrap_or_default()));
    let result = run_simulation(case, SimulationLimits::default()).map_err(|report| {
        emit(report.diagnostics());
        3
    })?;
    let mut failed = |error: &str| {
        emit(&[output_error(&output_path, error)]);
        4
    };
    let stem = output::directory::stem(&input.path).map_err(|error| failed(&error))?;
    let filename = input.path.file_name().unwrap_or_default().to_string_lossy();
    output::bellhop::write_directory(
        &output_path,
        &stem,
        case,
        &filename,
        &input.bytes,
        &result,
        &input.outcome.warnings,
        &input.paths.iter().map(PathBuf::as_path).collect::<Vec<_>>(),
        format.into(),
    )
    .map_err(|error| failed(&error))?;
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
    let field_sample_count: usize = result.field_sources.iter().map(|s| s.samples.len()).sum();
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
    Ok(format!(
        "wrote {} ({ray_count} rays, {eigenray_count} eigenrays, {arrival_count} arrivals, {field_sample_count} field samples, {point_count} trajectory points)",
        output_path.display()
    ))
}

/// BELLHOP for `run-many`.
struct Many {
    format: crate::Format,
}

impl crate::many::Model for Many {
    fn options(&self) -> serde_json::Value {
        serde_json::json!({})
    }

    fn load(&self, case: &Path) -> Result<Box<dyn crate::many::Loaded + '_>, crate::many::Outcome> {
        match load_case(case) {
            Ok(input) => Ok(Box::new(Loaded { input, many: self })),
            Err(report) => Err(crate::many::Outcome {
                code: 2,
                diagnostics: report
                    .diagnostics()
                    .iter()
                    .map(crate::many::Rendered::of)
                    .collect(),
                summary: None,
            }),
        }
    }
}

struct Loaded<'a> {
    input: LoadedInput,
    many: &'a Many,
}

impl crate::many::Loaded for Loaded<'_> {
    fn inputs(&self) -> Vec<&[u8]> {
        std::iter::once(self.input.bytes.as_slice())
            .chain(self.input.auxiliary.iter().map(String::as_bytes))
            .collect()
    }

    fn run(self: Box<Self>, output: &Path) -> crate::many::Outcome {
        let mut diagnostics = Vec::new();
        let result = solve(
            &self.input,
            Some(output.to_path_buf()),
            self.many.format,
            &mut |d| diagnostics.extend(d.iter().map(crate::many::Rendered::of)),
        );
        crate::many::Outcome {
            code: result.as_ref().err().copied().unwrap_or(0),
            diagnostics,
            summary: result.ok(),
        }
    }
}

fn output_error(path: &Path, message: &str) -> Diagnostic {
    Diagnostic::error("BH0402", message, "output", SourceLocation::file(path))
}

struct LoadedInput {
    path: PathBuf,
    outcome: LoadOutcome<Case>,
    bytes: Vec<u8>,
    paths: Vec<PathBuf>,
    /// Sources of `paths[1..]`, the consumed auxiliary files.
    auxiliary: Vec<String>,
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
            auxiliary: Vec::new(),
        })
    } else {
        let input = bellhop::legacy::load_case_with_inputs(path)?;
        let auxiliary = input.auxiliary_sources().to_vec();
        let (outcome, source, paths) = input.into_parts();
        Ok(LoadedInput {
            path: path.to_path_buf(),
            outcome,
            bytes: source.into_bytes(),
            paths,
            auxiliary,
        })
    }
}

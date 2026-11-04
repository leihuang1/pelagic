#![forbid(unsafe_code)]

use std::io::Write;
use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Subcommand, ValueEnum};
use kraken::input::{FieldInput, InputError};
use output::kraken::{DEFAULT_MAX_OUTPUT_BYTES, RunError, RunSummary, run_directory};

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Validate self-contained JSON or an ENV/FLP pair; - reads JSON from stdin.
    Validate {
        case: PathBuf,
        #[arg(long)]
        flp: Option<PathBuf>,
        #[arg(long, value_enum)]
        solver: Option<Solver>,
    },
    /// Run legacy .env/.flp or self-contained .json input and safely update a result directory.
    Run {
        case: PathBuf,
        /// Legacy FIELD geometry; defaults to the same-stem .flp. Invalid for JSON.
        #[arg(long)]
        flp: Option<PathBuf>,
        /// Legacy engine (default kraken); for JSON, assert the document's engine.
        #[arg(long, value_enum)]
        solver: Option<Solver>,
        /// Result directory, default `<case-stem>` in the current directory. Only owned files are replaced.
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Numerical output format; HTTP is unaffected.
        #[arg(long, value_enum, default_value = "legacy")]
        format: crate::Format,
        /// Cumulative file/payload quota, checked before datasets and after flushes.
        #[arg(long, default_value_t = DEFAULT_MAX_OUTPUT_BYTES, value_parser = clap::value_parser!(u64).range(1..))]
        max_output_bytes: u64,
    },
    /// Run many cases, each into its own result directory, recording each in a status file.
    RunMany {
        #[command(flatten)]
        many: crate::many::Args,
        /// Legacy engine (default kraken); for JSON, assert each document's engine.
        #[arg(long, value_enum)]
        solver: Option<Solver>,
        /// Cumulative file/payload quota of each case's result directory.
        #[arg(long, default_value_t = DEFAULT_MAX_OUTPUT_BYTES, value_parser = clap::value_parser!(u64).range(1..))]
        max_output_bytes: u64,
    },
    /// Export validated legacy inputs and all resources to self-contained JSON on stdout.
    Export {
        case: PathBuf,
        #[arg(long)]
        flp: Option<PathBuf>,
        #[arg(long, value_enum)]
        solver: Option<Solver>,
    },
}

#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum Solver {
    Kraken,
    Krakenc,
}

impl From<Solver> for kraken::ModeSolver {
    fn from(value: Solver) -> Self {
        match value {
            Solver::Kraken => Self::Kraken,
            Solver::Krakenc => Self::Krakenc,
        }
    }
}

pub fn execute(
    command: Command,
    diagnostics: crate::Diagnostics,
    threads: Option<NonZeroUsize>,
) -> ExitCode {
    let result = match command {
        Command::Validate { case, flp, solver } => validate(&case, flp.as_deref(), solver),
        Command::Run {
            case,
            flp,
            solver,
            output,
            format,
            max_output_bytes,
        } => run(
            &case,
            flp.as_deref(),
            solver,
            output,
            format,
            max_output_bytes,
        ),
        Command::RunMany {
            many,
            solver,
            max_output_bytes,
        } => {
            let model = Many {
                solver,
                format: many.format,
                max_output_bytes,
            };
            return crate::many::execute(&many, threads, diagnostics, &model);
        }
        Command::Export { case, flp, solver } => export(&case, flp.as_deref(), solver),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            if matches!(diagnostics, crate::Diagnostics::Text) {
                eprintln!("{error}");
            } else {
                diagnostics.render(&located(&error));
            }
            ExitCode::from(exit_code(&error))
        }
    }
}

/// Load a case; `-` names self-contained JSON on stdin.
fn load(case: &Path, flp: Option<&Path>, solver: Option<Solver>) -> Result<FieldInput, RunError> {
    let solver = solver.map(Into::into);
    if case == Path::new("-") {
        kraken::input::load_reader(std::io::stdin(), flp, solver)
    } else {
        kraken::input::load(case, flp, solver)
    }
    .map_err(input_failure)
}

fn input_failure(failure: InputError) -> RunError {
    RunError::Input(failure.into_report())
}

/// Solve a case and publish it to `output`, by default the input's stem.
fn run(
    case: &Path,
    flp: Option<&Path>,
    solver: Option<Solver>,
    output: Option<PathBuf>,
    format: crate::Format,
    max_output_bytes: u64,
) -> Result<(), RunError> {
    let input = load(case, flp, solver)?;
    let output = output.unwrap_or_else(|| {
        PathBuf::from(
            input
                .path()
                .file_stem()
                .unwrap_or_else(|| std::ffi::OsStr::new("kraken")),
        )
    });
    let summary = run_directory(&input, &output, format.into(), max_output_bytes)?;
    println!("{}", summary_line(&output, &summary));
    Ok(())
}

fn summary_line(output: &Path, summary: &RunSummary) -> String {
    format!(
        "wrote {} ({} frequencies, {} modes, {} pressures)",
        output.display(),
        summary.frequency_count,
        summary.mode_count,
        summary.pressure_count
    )
}

/// The error's diagnostics, with numerical fields located in their frequency.
fn located(error: &RunError) -> Vec<kraken::Diagnostic> {
    match error {
        RunError::Input(report) => report.diagnostics().to_vec(),
        RunError::Simulation {
            frequency_index,
            frequency_hz: _,
            report,
        } => report
            .diagnostics()
            .iter()
            .cloned()
            .map(|mut d| {
                d.field = format!("frequencies[{frequency_index}].{}", d.field);
                d
            })
            .collect(),
        RunError::Output(message) => vec![kraken::Diagnostic::new(
            "KR0402", message, "output", "<output>", 1, 1,
        )],
    }
}

fn exit_code(error: &RunError) -> u8 {
    match error {
        RunError::Input(_) => 2,
        RunError::Simulation { .. } => 3,
        RunError::Output(_) => 4,
    }
}

/// KRAKEN/KRAKENC for `run-many`.
struct Many {
    solver: Option<Solver>,
    format: crate::Format,
    max_output_bytes: u64,
}

impl crate::many::Model for Many {
    fn options(&self) -> serde_json::Value {
        // The quota decides failures and is recorded in the results.
        serde_json::json!({
            "solver": self.solver.map(|solver| match solver {
                Solver::Kraken => "kraken",
                Solver::Krakenc => "krakenc",
            }),
            "max_output_bytes": self.max_output_bytes,
        })
    }

    fn load(&self, case: &Path) -> Result<Box<dyn crate::many::Loaded + '_>, crate::many::Outcome> {
        // A JSON file that was read keeps its fingerprint even when it then fails.
        match kraken::input::load(case, None, self.solver.map(Into::into)) {
            Err(failure) if failure.snapshot().is_none() => {
                Err(outcome(case, Err(input_failure(failure))))
            }
            input => Ok(Box::new(Loaded { input, many: self })),
        }
    }
}

struct Loaded<'a> {
    input: Result<FieldInput, InputError>,
    many: &'a Many,
}

impl crate::many::Loaded for Loaded<'_> {
    fn inputs(&self) -> Vec<&[u8]> {
        match &self.input {
            Ok(input) => input.snapshots(),
            Err(failure) => failure.snapshot().map_or(&[][..], std::slice::from_ref),
        }
        .iter()
        .map(|snapshot| snapshot.source().as_bytes())
        .collect()
    }

    fn run(self: Box<Self>, output: &Path) -> crate::many::Outcome {
        let many = self.many;
        let result = self.input.map_err(input_failure).and_then(|input| {
            run_directory(&input, output, many.format.into(), many.max_output_bytes)
        });
        outcome(output, result)
    }
}

fn outcome(output: &Path, result: Result<RunSummary, RunError>) -> crate::many::Outcome {
    match result {
        Ok(summary) => crate::many::Outcome {
            code: 0,
            diagnostics: Vec::new(),
            summary: Some(summary_line(output, &summary)),
        },
        Err(error) => crate::many::Outcome {
            code: exit_code(&error),
            diagnostics: located(&error)
                .iter()
                .map(crate::many::Rendered::of)
                .collect(),
            summary: None,
        },
    }
}

fn validate(path: &Path, flp: Option<&Path>, solver: Option<Solver>) -> Result<(), RunError> {
    let input = load(path, flp, solver)?;
    let cases = input.cases();
    println!(
        "valid 2D KRAKEN case: {} ({:?}; frequencies={}, profiles={})",
        input.path().display(),
        cases[0].profiles()[0].mode_solver,
        cases.len(),
        cases[0].profiles().len()
    );
    Ok(())
}

fn export(path: &Path, flp: Option<&Path>, solver: Option<Solver>) -> Result<(), RunError> {
    let input = load(path, flp, solver)?;
    let bytes = exported(&input)?;
    if bytes.len() as u64 > kraken::legacy::MAX_INPUT_BYTES {
        return Err(RunError::Input(kraken::DiagnosticReport::one(
            kraken::Diagnostic::new(
                "KR0101",
                "exported JSON exceeds 1 MiB",
                "json",
                input.path(),
                1,
                1,
            ),
        )));
    }
    std::io::stdout()
        .lock()
        .write_all(&bytes)
        .map_err(|error| RunError::Output(error.to_string()))
}

/// The self-contained JSON that `export` prints, before its size check.
fn exported(input: &FieldInput) -> Result<Vec<u8>, RunError> {
    let document = input.document().map_err(RunError::Input)?;
    // Serialize completely before touching stdout: invalid/oversized exports emit no partial document.
    let mut bytes = kraken::json::encode_case_document(&document, input.path())
        .map_err(RunError::Input)?
        .into_bytes();
    bytes.push(b'\n');
    Ok(bytes)
}

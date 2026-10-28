#![forbid(unsafe_code)]

use std::io::{Read, Write};
use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Subcommand, ValueEnum};
use output::kraken::{
    DEFAULT_MAX_OUTPUT_BYTES, RunError, RunSummary, run_json_directory, run_legacy_directory,
};

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
        Command::Validate { case, flp, solver } => validate(&case, flp, solver),
        Command::Run {
            case,
            flp,
            solver,
            output,
            format,
            max_output_bytes,
        } => {
            let output = output.unwrap_or_else(|| {
                PathBuf::from(
                    source_path(&case)
                        .file_stem()
                        .unwrap_or_else(|| std::ffi::OsStr::new("kraken")),
                )
            });
            run(&case, flp, solver, &output, format, max_output_bytes)
                .map(|summary| println!("{}", summary_line(&output, &summary)))
        }
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
        Command::Export { case, flp, solver } => export(&case, flp, solver),
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

/// Solve a case and publish it to `output`, as `run` does.
fn run(
    case: &Path,
    flp: Option<PathBuf>,
    solver: Option<Solver>,
    output: &Path,
    format: crate::Format,
    max_output_bytes: u64,
) -> Result<RunSummary, RunError> {
    if case == Path::new("-") {
        reject_json_flp(flp.as_deref())?;
        let source = read_source(case)?;
        output::kraken::run_json_source_directory(
            Path::new("stdin.json"),
            &source,
            &[],
            output,
            solver.map(Into::into),
            format.into(),
            max_output_bytes,
        )
    } else if is_json(case) {
        reject_json_flp(flp.as_deref())?;
        run_json_directory(
            case,
            output,
            solver.map(Into::into),
            format.into(),
            max_output_bytes,
        )
    } else {
        run_legacy_directory(
            case,
            &flp.unwrap_or_else(|| case.with_extension("flp")),
            output,
            solver.unwrap_or(Solver::Kraken).into(),
            format.into(),
            max_output_bytes,
        )
    }
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
        let input = if is_json(case) {
            output::kraken::read_source(case).map(Input::Json)
        } else {
            kraken::input::load_legacy(
                case,
                &case.with_extension("flp"),
                self.solver.unwrap_or(Solver::Kraken).into(),
            )
            .map(Input::Legacy)
            .map_err(RunError::Input)
        };
        match input {
            Ok(input) => Ok(Box::new(Loaded {
                case: case.to_path_buf(),
                input,
                many: self,
            })),
            Err(error) => Err(outcome(case, Err(error))),
        }
    }
}

enum Input {
    Json(String),
    Legacy(kraken::input::FieldInput),
}

struct Loaded<'a> {
    case: PathBuf,
    input: Input,
    many: &'a Many,
}

impl crate::many::Loaded for Loaded<'_> {
    fn inputs(&self) -> Vec<&[u8]> {
        match &self.input {
            Input::Json(source) => vec![source.as_bytes()],
            Input::Legacy(input) => input
                .snapshots()
                .iter()
                .map(|snapshot| snapshot.source().as_bytes())
                .collect(),
        }
    }

    // `run_json_directory` and `run_legacy_directory`, after their reads.
    fn run(self: Box<Self>, output: &Path) -> crate::many::Outcome {
        let many = self.many;
        let result = match &self.input {
            Input::Json(source) => output::kraken::run_json_source_directory(
                &self.case,
                source,
                &[&self.case],
                output,
                many.solver.map(Into::into),
                many.format.into(),
                many.max_output_bytes,
            ),
            Input::Legacy(input) => output::kraken::run_legacy_input_directory(
                &self.case,
                input,
                output,
                many.format.into(),
                many.max_output_bytes,
            ),
        };
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

fn validate(path: &Path, flp: Option<PathBuf>, solver: Option<Solver>) -> Result<(), RunError> {
    let cases = load_cases(path, flp, solver)?;
    println!(
        "valid 2D KRAKEN case: {} ({:?}; frequencies={}, profiles={})",
        source_path(path).display(),
        cases[0].profiles()[0].mode_solver,
        cases.len(),
        cases[0].profiles().len()
    );
    Ok(())
}

fn source_path(path: &Path) -> &Path {
    if path == Path::new("-") {
        Path::new("stdin.json")
    } else {
        path
    }
}

fn report(path: &Path, code: &'static str, message: &str, field: &str) -> kraken::DiagnosticReport {
    kraken::DiagnosticReport::one(kraken::Diagnostic::new(code, message, field, path, 1, 1))
}

fn read_source(path: &Path) -> Result<String, RunError> {
    let mut source = String::new();
    let input: Box<dyn Read> = if path == Path::new("-") {
        Box::new(std::io::stdin())
    } else {
        Box::new(
            std::fs::File::open(path)
                .map_err(|e| RunError::Input(report(path, "KR0001", &e.to_string(), "input")))?,
        )
    };
    input
        .take(kraken::legacy::MAX_INPUT_BYTES + 1)
        .read_to_string(&mut source)
        .map_err(|e| {
            RunError::Input(report(source_path(path), "KR0001", &e.to_string(), "input"))
        })?;
    Ok(source)
}

fn is_json(path: &Path) -> bool {
    path.extension()
        .is_some_and(|extension| extension == "json")
}

fn reject_json_flp(flp: Option<&Path>) -> Result<(), RunError> {
    if flp.is_some() {
        Err(RunError::Input(report(
            Path::new("<case>"),
            "KR0202",
            "--flp is invalid for self-contained JSON",
            "flp",
        )))
    } else {
        Ok(())
    }
}

fn load_cases(
    path: &Path,
    flp: Option<PathBuf>,
    solver: Option<Solver>,
) -> Result<Vec<kraken::FieldCase>, RunError> {
    if is_json(path) || path == Path::new("-") {
        reject_json_flp(flp.as_deref())?;
        let source = read_source(path)?;
        let cases = kraken::json::load_case_document_named(source.as_bytes(), source_path(path))
            .map_err(RunError::Input)?;
        check_solver(path, solver, &cases)?;
        Ok(cases)
    } else {
        kraken::legacy::load_field_cases(
            path,
            flp.unwrap_or_else(|| path.with_extension("flp")),
            solver.unwrap_or(Solver::Kraken).into(),
        )
        .map_err(RunError::Input)
    }
}

fn check_solver(
    path: &Path,
    solver: Option<Solver>,
    cases: &[kraken::FieldCase],
) -> Result<(), RunError> {
    if solver.is_some_and(|solver| {
        kraken::ModeSolver::from(solver) != cases[0].profiles()[0].mode_solver
    }) {
        return Err(RunError::Input(report(
            source_path(path),
            "KR0202",
            "--solver must match the JSON document",
            "kraken.solver",
        )));
    }
    Ok(())
}

fn export(path: &Path, flp: Option<PathBuf>, solver: Option<Solver>) -> Result<(), RunError> {
    let bytes = exported(path, flp, solver)?;
    if bytes.len() as u64 > kraken::legacy::MAX_INPUT_BYTES {
        return Err(RunError::Input(report(
            source_path(path),
            "KR0101",
            "exported JSON exceeds 1 MiB",
            "json",
        )));
    }
    std::io::stdout()
        .lock()
        .write_all(&bytes)
        .map_err(|error| RunError::Output(error.to_string()))
}

/// The self-contained JSON that `export` prints, before its size check.
fn exported(
    path: &Path,
    flp: Option<PathBuf>,
    solver: Option<Solver>,
) -> Result<Vec<u8>, RunError> {
    let document = if is_json(path) || path == Path::new("-") {
        reject_json_flp(flp.as_deref())?;
        let source = read_source(path)?;
        let document = kraken::json::decode_case_document(source.as_bytes(), source_path(path))
            .map_err(RunError::Input)?;
        let cases = document.cases(source_path(path)).map_err(RunError::Input)?;
        check_solver(path, solver, &cases)?;
        document
    } else {
        kraken::input::load_legacy(
            path,
            &flp.unwrap_or_else(|| path.with_extension("flp")),
            solver.unwrap_or(Solver::Kraken).into(),
        )
        .map_err(RunError::Input)?
        .document()
        .map_err(RunError::Input)?
    };
    // Serialize completely before touching stdout: invalid/oversized exports emit no partial document.
    let mut bytes = kraken::json::encode_case_document(&document, source_path(path))
        .map_err(RunError::Input)?
        .into_bytes();
    bytes.push(b'\n');
    Ok(bytes)
}

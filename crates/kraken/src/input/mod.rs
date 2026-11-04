//! Case intake: any KRAKEN source becomes one [`FieldInput`], so adapters do not
//! sequence reading, parsing, the solver assertion or consumed inputs themselves.
pub(crate) mod attenuation;
pub mod json;
pub mod legacy;
pub(crate) mod material;

use std::error::Error;
use std::fmt;
use std::io::Read;
use std::path::{Path, PathBuf};

use crate::{Diagnostic, DiagnosticReport, FieldCase, ModeSolver};

/// Exact UTF-8 bytes consumed under one resource role: `json`, or a legacy
/// `env`, `flp`, `trc`, `brc`, `irc` or `sbp`.
#[derive(Clone, Debug, PartialEq)]
pub struct InputSnapshot {
    role: &'static str,
    path: PathBuf,
    source: String,
}

impl InputSnapshot {
    fn read(role: &'static str, path: &Path) -> Result<Self, DiagnosticReport> {
        Ok(Self {
            role,
            path: path.to_path_buf(),
            source: legacy::read_file(path)?,
        })
    }

    #[must_use]
    pub fn role(&self) -> &'static str {
        self.role
    }
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }
    #[must_use]
    pub fn source(&self) -> &str {
        &self.source
    }
}

/// Validated frequency blocks together with the snapshots actually parsed.
/// Neither cases nor snapshots can be changed through this read-only view.
#[derive(Clone, Debug, PartialEq)]
pub struct FieldInput {
    cases: Vec<FieldCase>,
    snapshots: Vec<InputSnapshot>,
    solver: ModeSolver,
    consumed: Vec<PathBuf>,
}

impl FieldInput {
    #[must_use]
    pub fn cases(&self) -> &[FieldCase] {
        &self.cases
    }
    #[must_use]
    pub fn snapshots(&self) -> &[InputSnapshot] {
        &self.snapshots
    }
    /// The primary input's named path: the JSON or ENV file, or `stdin.json`
    /// for a reader.
    #[must_use]
    pub fn path(&self) -> &Path {
        self.snapshots[0].path()
    }
    /// The files this input read, which a publication must not overwrite. A
    /// reader's source names no file.
    #[must_use]
    pub fn consumed_inputs(&self) -> &[PathBuf] {
        &self.consumed
    }
    /// The equivalent JSON document, keeping a legacy `.env` loss recipe. It is
    /// built from the snapshots on request, so loading and solving never
    /// depend on it.
    /// # Errors
    /// Returns the reader's diagnostics if a snapshot cannot be translated.
    pub fn document(&self) -> Result<json::CaseDocument, DiagnosticReport> {
        let snapshot = |role: &str| self.snapshots.iter().find(|s| s.role == role);
        if let Some(json) = snapshot("json") {
            return json::decode_case_document(json.source().as_bytes(), json.path());
        }
        let source = |role: &str| snapshot(role).map(InputSnapshot::source);
        let (env, flp) = (&self.snapshots[0], &self.snapshots[1]);
        legacy::field_document_with_resources(
            env.source(),
            flp.source(),
            env.path(),
            flp.path(),
            self.solver,
            ["trc", "brc", "irc"].map(source),
            source("sbp"),
        )
    }
    #[must_use]
    pub fn into_cases(self) -> Vec<FieldCase> {
        self.cases
    }
}

/// A failed load, with the JSON snapshot if one was read, so a caller can still
/// fingerprint the bytes that failed. A legacy failure carries none: its
/// fingerprint would cover every consumed input, which only a success establishes.
#[derive(Clone, Debug, PartialEq)]
pub struct InputError {
    report: DiagnosticReport,
    snapshot: Option<InputSnapshot>,
}

impl InputError {
    #[must_use]
    pub fn snapshot(&self) -> Option<&InputSnapshot> {
        self.snapshot.as_ref()
    }
    #[must_use]
    pub fn into_report(self) -> DiagnosticReport {
        self.report
    }
}

impl fmt::Display for InputError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.report.fmt(formatter)
    }
}

impl Error for InputError {}

impl From<DiagnosticReport> for InputError {
    fn from(report: DiagnosticReport) -> Self {
        Self {
            report,
            snapshot: None,
        }
    }
}

/// Load a `.json` document, or else a legacy ENV with `flp`, by default the
/// same-stem file. `solver` selects a legacy engine, by default KRAKEN, and for
/// JSON is an assertion, not an override.
/// # Errors
/// Returns read, parse and validation diagnostics, `flp` given for JSON, or a
/// JSON document whose engine is not `solver`.
pub fn load(
    path: &Path,
    flp: Option<&Path>,
    solver: Option<ModeSolver>,
) -> Result<FieldInput, InputError> {
    if path
        .extension()
        .is_some_and(|extension| extension == "json")
    {
        reject_flp(flp)?;
        let source = legacy::read_file_bounded(path)?;
        load_json(source, path, vec![path.to_path_buf()], solver)
    } else {
        let flp = flp.map_or_else(|| path.with_extension("flp"), Path::to_path_buf);
        Ok(load_legacy(
            path,
            &flp,
            solver.unwrap_or(ModeSolver::Kraken),
        )?)
    }
}

/// Load a JSON document from a stream. It is named `stdin.json` and consumes no
/// path; `flp` and `solver` are as for [`load`].
/// # Errors
/// Returns the same diagnostics as a `.json` file given to [`load`].
pub fn load_reader(
    reader: impl Read,
    flp: Option<&Path>,
    solver: Option<ModeSolver>,
) -> Result<FieldInput, InputError> {
    reject_flp(flp)?;
    let path = Path::new("stdin.json");
    load_json(
        legacy::read_bounded(reader, path)?,
        path,
        Vec::new(),
        solver,
    )
}

fn reject_flp(flp: Option<&Path>) -> Result<(), DiagnosticReport> {
    if flp.is_some() {
        return Err(DiagnosticReport::one(Diagnostic::new(
            "KR0202",
            "--flp is invalid for self-contained JSON",
            "flp",
            "<case>",
            1,
            1,
        )));
    }
    Ok(())
}

fn load_json(
    source: String,
    path: &Path,
    consumed: Vec<PathBuf>,
    solver: Option<ModeSolver>,
) -> Result<FieldInput, InputError> {
    let snapshot = InputSnapshot {
        role: "json",
        path: path.to_path_buf(),
        source,
    };
    let cases =
        json::load_case_document_named(snapshot.source.as_bytes(), path).and_then(|cases| {
            if solver.is_some_and(|solver| solver != cases[0].profiles()[0].mode_solver) {
                return Err(DiagnosticReport::one(Diagnostic::new(
                    "KR0202",
                    "--solver must match the JSON document",
                    "kraken.solver",
                    path,
                    1,
                    1,
                )));
            }
            Ok(cases)
        });
    match cases {
        Ok(cases) => Ok(FieldInput {
            solver: cases[0].profiles()[0].mode_solver,
            cases,
            snapshots: vec![snapshot],
            consumed,
        }),
        Err(report) => Err(InputError {
            report,
            snapshot: Some(snapshot),
        }),
    }
}

/// Read a complete legacy ENV/FLP sequence and only its required TRC/BRC/IRC/SBP.
/// ENV owns table stems; FLP owns the source-pattern stem. Reads are bounded by
/// the existing 1 MiB limit and every retained source is the exact parsed snapshot.
/// # Errors
/// Returns existing read, discovery, resource and case-validation diagnostics.
pub fn load_legacy(
    env_path: &Path,
    flp_path: &Path,
    solver: ModeSolver,
) -> Result<FieldInput, DiagnosticReport> {
    let env = InputSnapshot::read("env", env_path)?;
    let flp = InputSnapshot::read("flp", flp_path)?;
    let [surface, brc, irc] =
        legacy::field_table_extensions(env.source(), env_path, solver)?.map(|ext| {
            ext.map(|ext| InputSnapshot::read(ext, &env_path.with_extension(ext)))
                .transpose()
        });
    let tables = [surface?, brc?, irc?];
    let pattern = legacy::source_pattern_extension(flp.source(), flp_path)?
        .map(|ext| InputSnapshot::read(ext, &flp_path.with_extension(ext)))
        .transpose()?;
    let table_sources = tables
        .each_ref()
        .map(|table| table.as_ref().map(InputSnapshot::source));
    let pattern_source = pattern.as_ref().map(InputSnapshot::source);
    let cases = legacy::load_field_cases_with_resources(
        env.source(),
        flp.source(),
        env_path,
        flp_path,
        solver,
        table_sources,
        pattern_source,
    )?;
    let mut snapshots = vec![env, flp];
    snapshots.extend(tables.into_iter().flatten());
    snapshots.extend(pattern);
    Ok(FieldInput {
        consumed: snapshots.iter().map(|s| s.path.clone()).collect(),
        cases,
        snapshots,
        solver,
    })
}

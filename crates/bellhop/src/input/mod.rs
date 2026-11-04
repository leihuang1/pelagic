//! Case intake: any BELLHOP source becomes one [`CaseInput`], so adapters do not
//! sequence reading, parsing or consumed inputs themselves.

pub mod json;
pub mod legacy;

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use crate::Case;
use crate::diagnostic::{Diagnostic, DiagnosticReport, LoadOutcome};

/// A complete case with its exact primary source and every consumed input.
#[derive(Clone, Debug)]
pub struct CaseInput {
    path: PathBuf,
    outcome: LoadOutcome<Case>,
    source: Vec<u8>,
    consumed: Vec<PathBuf>,
    auxiliary_sources: Vec<String>,
}

impl CaseInput {
    /// The primary input's named path: the JSON or ENV file, or `stdin.json`
    /// for a reader.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    #[must_use]
    pub fn outcome(&self) -> &LoadOutcome<Case> {
        &self.outcome
    }

    /// The exact primary source.
    #[must_use]
    pub fn source(&self) -> &[u8] {
        &self.source
    }

    /// The files this input read, primary first, which a publication must not
    /// overwrite. A reader's source names no file.
    #[must_use]
    pub fn consumed_inputs(&self) -> &[PathBuf] {
        &self.consumed
    }

    /// Exact sources of the consumed auxiliary inputs, one per consumed input
    /// after the primary one and in the same order.
    #[must_use]
    pub fn auxiliary_sources(&self) -> &[String] {
        &self.auxiliary_sources
    }

    /// Consume the read-only input record without rereading or reconstructing provenance.
    #[must_use]
    pub fn into_parts(self) -> (LoadOutcome<Case>, Vec<u8>, Vec<PathBuf>) {
        (self.outcome, self.source, self.consumed)
    }
}

/// Load a `.json` document, or else a legacy `.env` with its auxiliary files.
/// # Errors
/// Returns read, parse and validation diagnostics.
pub fn load(path: &Path) -> Result<CaseInput, DiagnosticReport> {
    if path
        .extension()
        .is_some_and(|extension| extension == "json")
    {
        let source = fs::read(path)
            .map_err(|error| DiagnosticReport::from_diagnostic(Diagnostic::io(path, &error)))?;
        load_json(source, path, vec![path.to_path_buf()])
    } else {
        legacy::load_case_with_inputs(path)
    }
}

/// Load a JSON document from a stream. It is named `stdin.json` and consumes no path.
/// # Errors
/// Returns read, parse and validation diagnostics.
pub fn load_reader(mut reader: impl Read) -> Result<CaseInput, DiagnosticReport> {
    let path = Path::new("stdin.json");
    let mut source = Vec::new();
    reader
        .read_to_end(&mut source)
        .map_err(|error| DiagnosticReport::from_diagnostic(Diagnostic::io(path, &error)))?;
    load_json(source, path, Vec::new())
}

fn load_json(
    source: Vec<u8>,
    path: &Path,
    consumed: Vec<PathBuf>,
) -> Result<CaseInput, DiagnosticReport> {
    let outcome =
        json::load_case_document_named(&source, path).map_err(json::DocumentError::into_report)?;
    Ok(CaseInput {
        path: path.to_path_buf(),
        outcome,
        source,
        consumed,
        auxiliary_sources: Vec::new(),
    })
}

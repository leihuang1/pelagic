//! Versioned, self-contained JSON input format.
//!
//! The document is designed to be written by hand: grids instead of expanded
//! vectors, sound-speed rows like the `.env`, and defaults for everything the
//! `.env` itself defaults. It maps to the validated [`crate::Case`] through the
//! same validation path as legacy input, and no auxiliary files are resolved.

use std::error::Error;
use std::fmt;
use std::path::Path;

use serde::Deserialize;
use serde::de::DeserializeOwned;

use crate::Case;
use crate::diagnostic::{Diagnostic, DiagnosticReport, LoadOutcome, SourceLocation};

macro_rules! enum_map {
    ($import:ident, $export:ident, $doc:ty, $model:ty, { $($variant:ident),+ $(,)? }) => {
        fn $import(value: $doc) -> $model {
            match value { $(<$doc>::$variant => <$model>::$variant),+ }
        }
        fn $export(value: $model) -> $doc {
            match value { $(<$model>::$variant => <$doc>::$variant),+ }
        }
    };
}

pub mod document;
pub mod grid;

pub use document::CaseDocument;

/// The schema version, the document's `"pelagic"` field.
pub const SCHEMA_VERSION: u32 = 2;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DocumentErrorKind {
    Malformed,
    Semantic,
}

#[derive(Clone, Debug)]
pub struct DocumentError {
    kind: DocumentErrorKind,
    report: DiagnosticReport,
}

impl DocumentError {
    #[must_use]
    pub fn kind(&self) -> DocumentErrorKind {
        self.kind
    }

    #[must_use]
    pub fn report(&self) -> &DiagnosticReport {
        &self.report
    }

    #[must_use]
    pub fn into_report(self) -> DiagnosticReport {
        self.report
    }

    fn semantic(report: DiagnosticReport) -> Self {
        Self {
            kind: DocumentErrorKind::Semantic,
            report,
        }
    }
}

impl fmt::Display for DocumentError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.report.fmt(formatter)
    }
}

impl Error for DocumentError {}

/// Parses and validates a self-contained JSON case.
///
/// # Errors
///
/// Returns [`DocumentErrorKind::Malformed`] for JSON syntax/schema failures and
/// [`DocumentErrorKind::Semantic`] when the decoded case is inconsistent.
pub fn load_case_document(bytes: &[u8]) -> Result<LoadOutcome<Case>, DocumentError> {
    load_case_document_named(bytes, Path::new("request.json"))
}

/// Parses and validates a self-contained JSON case with a diagnostic source name.
///
/// The `"pelagic"` version and `"model"` are checked before the rest of the
/// document, so a wrong version or model gets a specific diagnostic.
///
/// # Errors
///
/// Returns [`DocumentErrorKind::Malformed`] for JSON syntax/schema failures and
/// [`DocumentErrorKind::Semantic`] when the decoded case is inconsistent.
pub fn load_case_document_named(
    bytes: &[u8],
    source_path: &Path,
) -> Result<LoadOutcome<Case>, DocumentError> {
    #[derive(Deserialize)]
    struct Probe {
        pelagic: Option<serde_json::Value>,
        model: Option<serde_json::Value>,
    }
    let probe: Probe = decode(bytes, source_path)?;
    let reject = |code, message: String, field: &str| DocumentError {
        kind: if code == "BH0103" {
            DocumentErrorKind::Malformed
        } else {
            DocumentErrorKind::Semantic
        },
        report: DiagnosticReport::from_diagnostic(error(source_path, code, message, field)),
    };
    match probe.pelagic {
        None => {
            return Err(reject(
                "BH0103",
                "invalid JSON case: missing field `pelagic` (the schema version, 2)".into(),
                "pelagic",
            ));
        }
        Some(version) if version.as_u64() != Some(u64::from(SCHEMA_VERSION)) => {
            return Err(reject(
                "BH0202",
                format!(
                    "unsupported JSON case schema version {version}; expected {SCHEMA_VERSION}"
                ),
                "pelagic",
            ));
        }
        Some(_) => {}
    }
    if let Some(model) = probe.model.filter(|model| model != "bellhop") {
        return Err(reject(
            "BH0202",
            format!("this is a {model} case, not a BELLHOP case"),
            "model",
        ));
    }
    document::load(bytes, source_path)
}

/// Serializes a loaded legacy or JSON case into the canonical version 2 document.
///
/// # Errors
///
/// Returns diagnostics if a loaded case contains unsupported legacy-only
/// behavior or is missing data required by its boundary condition.
pub fn export_case_document(case: &Case) -> Result<CaseDocument, DiagnosticReport> {
    CaseDocument::try_from(case)
}

/// Strictly decodes one JSON value, reporting the failing path and position.
fn decode<T: DeserializeOwned>(bytes: &[u8], source_path: &Path) -> Result<T, DocumentError> {
    let malformed = |message: String, field: String, line, column| DocumentError {
        kind: DocumentErrorKind::Malformed,
        report: DiagnosticReport::from_diagnostic(Diagnostic::error(
            "BH0103",
            format!("invalid JSON case: {message}"),
            field,
            SourceLocation::new(source_path, line, column),
        )),
    };
    let mut decoder = serde_json::Deserializer::from_slice(bytes);
    let value = serde_path_to_error::deserialize(&mut decoder).map_err(|error| {
        malformed(
            error.to_string(),
            error.path().to_string(),
            error.inner().line(),
            error.inner().column(),
        )
    })?;
    decoder.end().map_err(|error| {
        malformed(
            error.to_string(),
            "json".into(),
            error.line(),
            error.column(),
        )
    })?;
    Ok(value)
}

fn error(
    path: &Path,
    code: &'static str,
    message: impl Into<String>,
    field: impl Into<String>,
) -> Diagnostic {
    Diagnostic::error(code, message, field, SourceLocation::file(path))
}

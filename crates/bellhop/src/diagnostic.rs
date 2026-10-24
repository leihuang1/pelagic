use std::error::Error;
use std::fmt;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Warning,
    Error,
}

impl fmt::Display for Severity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Warning => formatter.write_str("warning"),
            Self::Error => formatter.write_str("error"),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize)]
pub struct SourceLocation {
    #[serde(serialize_with = "serialize_path")]
    pub path: PathBuf,
    pub line: usize,
    pub column: usize,
}

fn serialize_path<S: serde::Serializer>(path: &Path, serializer: S) -> Result<S::Ok, S::Error> {
    serializer.serialize_str(&path.to_string_lossy())
}

impl SourceLocation {
    #[must_use]
    pub fn new(path: impl Into<PathBuf>, line: usize, column: usize) -> Self {
        Self {
            path: path.into(),
            line,
            column,
        }
    }

    #[must_use]
    pub fn file(path: impl Into<PathBuf>) -> Self {
        Self::new(path, 1, 1)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize)]
pub struct Diagnostic {
    pub severity: Severity,
    pub code: &'static str,
    pub message: String,
    pub field: Option<String>,
    #[serde(rename = "source")]
    pub location: SourceLocation,
}

impl Diagnostic {
    #[must_use]
    pub fn error(
        code: &'static str,
        message: impl Into<String>,
        field: impl Into<String>,
        location: SourceLocation,
    ) -> Self {
        Self {
            severity: Severity::Error,
            code,
            message: message.into(),
            field: Some(field.into()),
            location,
        }
    }

    #[must_use]
    pub fn warning(
        code: &'static str,
        message: impl Into<String>,
        field: impl Into<String>,
        location: SourceLocation,
    ) -> Self {
        Self {
            severity: Severity::Warning,
            code,
            message: message.into(),
            field: Some(field.into()),
            location,
        }
    }

    #[must_use]
    pub fn io(path: &Path, error: &std::io::Error) -> Self {
        Self::error(
            "BH0001",
            format!("unable to read input: {error}"),
            "input",
            SourceLocation::file(path),
        )
    }
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{}:{}:{}: {}[{}]: {}",
            self.location.path.display(),
            self.location.line,
            self.location.column,
            self.severity,
            self.code,
            self.message
        )?;
        if let Some(field) = &self.field {
            write!(formatter, " ({field})")?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct DiagnosticReport {
    diagnostics: Vec<Diagnostic>,
}

impl DiagnosticReport {
    #[must_use]
    pub fn from_diagnostic(diagnostic: Diagnostic) -> Self {
        Self {
            diagnostics: vec![diagnostic],
        }
    }

    pub fn push(&mut self, diagnostic: Diagnostic) {
        self.diagnostics.push(diagnostic);
    }

    pub fn extend(&mut self, diagnostics: impl IntoIterator<Item = Diagnostic>) {
        self.diagnostics.extend(diagnostics);
    }

    #[must_use]
    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }

    pub(crate) fn diagnostics_mut(&mut self) -> &mut [Diagnostic] {
        &mut self.diagnostics
    }

    #[must_use]
    pub fn has_errors(&self) -> bool {
        self.diagnostics
            .iter()
            .any(|diagnostic| diagnostic.severity == Severity::Error)
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.diagnostics.is_empty()
    }
}

impl fmt::Display for DiagnosticReport {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (index, diagnostic) in self.diagnostics.iter().enumerate() {
            if index > 0 {
                writeln!(formatter)?;
            }
            write!(formatter, "{diagnostic}")?;
        }
        Ok(())
    }
}

impl Error for DiagnosticReport {}

#[derive(Clone, Debug)]
pub struct LoadOutcome<T> {
    pub value: T,
    pub warnings: Vec<Diagnostic>,
}

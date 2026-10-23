//! Strict, self-contained KRAKEN JSON input, with canonical solve-frequency materials.
//! Only unvalidated definitions are deserialized; cases always pass the shared validators.
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::{Case, CaseDefinition, Diagnostic, DiagnosticReport, FieldCase, FieldPropagation};

pub const SCHEMA_VERSION: u32 = 1;

/// Ordered frequency blocks, including repetitions. All resources are inline.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CaseDocument {
    pub schema_version: u32,
    pub frequencies: Vec<FieldDocument>,
}

/// One solve frequency, with full modal environments and FIELD geometry.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FieldDocument {
    pub propagation: FieldPropagation,
    pub profile_ranges_m: Vec<f64>,
    pub profiles: Vec<CaseDefinition>,
}

/// Parse a document using `request.json` as its diagnostic source.
/// # Errors
/// Returns bounded-input, schema or shared case/profile validation diagnostics.
pub fn load_case_document(bytes: &[u8]) -> Result<Vec<FieldCase>, DiagnosticReport> {
    load_case_document_named(bytes, Path::new("request.json"))
}

/// Parse exact bytes without resolving auxiliary files or sorting frequency blocks.
/// # Errors
/// Rejects oversized/malformed documents, unsupported versions and invalid cases.
pub fn load_case_document_named(
    bytes: &[u8],
    path: &Path,
) -> Result<Vec<FieldCase>, DiagnosticReport> {
    if bytes.len() as u64 > crate::legacy::MAX_INPUT_BYTES {
        return Err(error(path, "KR0101", "JSON input exceeds 1 MiB", "json"));
    }
    let mut decoder = serde_json::Deserializer::from_slice(bytes);
    let document: CaseDocument =
        serde_path_to_error::deserialize(&mut decoder).map_err(|error| {
            DiagnosticReport::one(Diagnostic::new(
                "KR0103",
                format!("invalid JSON case: {error}"),
                malformed_field(&error.path().to_string()),
                path,
                error.inner().line(),
                error.inner().column(),
            ))
        })?;
    decoder
        .end()
        .map_err(|cause| error(path, "KR0103", &cause.to_string(), "json"))?;
    if document.schema_version != SCHEMA_VERSION {
        return Err(error(
            path,
            "KR0202",
            "unsupported JSON schema version; expected 1",
            "schema_version",
        ));
    }
    if !(1..=crate::legacy::MAX_FREQUENCIES).contains(&document.frequencies.len()) {
        return Err(error(
            path,
            "KR0201",
            "require 1..=1000 frequency blocks",
            "frequencies",
        ));
    }
    let mut cases = Vec::with_capacity(document.frequencies.len());
    for (frequency, field) in document.frequencies.into_iter().enumerate() {
        let profiles = field
            .profiles
            .into_iter()
            .enumerate()
            .map(|(profile, definition)| {
                Case::from_definition(definition).map_err(|report| {
                    locate(
                        report,
                        path,
                        &format!("frequencies[{frequency}].profiles[{profile}]"),
                    )
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        let case = FieldCase::new(profiles, field.profile_ranges_m, field.propagation)
            .map_err(|report| locate(report, path, &format!("frequencies[{frequency}]")))?;
        cases.push(case);
    }
    validate_sequences(&cases, path)?;
    Ok(cases)
}

/// Export already validated cases without changing precision, units or frequency order.
/// Losses are effective dB/wavelength at each block's solve frequency, including volume loss.
/// # Errors
/// Rejects empty/oversized collections, mixed backends or compact JSON over 1 MiB.
pub fn export_case_document(cases: &[FieldCase]) -> Result<CaseDocument, DiagnosticReport> {
    let path = Path::new("<json>");
    validate_sequences(cases, path)?;
    // ponytail: explicit frequency blocks duplicate geometry; add templates only if document size matters.
    let document = CaseDocument {
        schema_version: SCHEMA_VERSION,
        frequencies: cases
            .iter()
            .map(|case| FieldDocument {
                propagation: case.propagation(),
                profile_ranges_m: case.ranges_m().to_vec(),
                profiles: case
                    .profiles()
                    .iter()
                    .cloned()
                    .map(Case::into_definition)
                    .collect(),
            })
            .collect(),
    };
    let oversized = || error(path, "KR0101", "exported JSON exceeds 1 MiB", "json");
    let mut buffer = vec![
        0;
        crate::legacy::MAX_INPUT_BYTES
            .try_into()
            .map_err(|_| oversized())?
    ];
    // A fixed slice bounds the sizing pass, even when a valid case has huge titles or vectors.
    serde_json::to_writer(std::io::Cursor::new(buffer.as_mut_slice()), &document).map_err(
        |cause| {
            if cause.is_io() {
                oversized()
            } else {
                error(path, "KR0103", &cause.to_string(), "json")
            }
        },
    )?;
    Ok(document)
}

fn validate_sequences(cases: &[FieldCase], path: &Path) -> Result<(), DiagnosticReport> {
    if !(1..=crate::legacy::MAX_FREQUENCIES).contains(&cases.len()) {
        return Err(error(
            path,
            "KR0201",
            "require 1..=1000 frequency blocks",
            "frequencies",
        ));
    }
    let solver = cases[0].profiles()[0].mode_solver;
    let mut values = 0;
    for (index, case) in cases.iter().enumerate() {
        if case.profiles()[0].mode_solver != solver {
            return Err(error(
                path,
                "KR0201",
                "frequency blocks must share the same mode solver",
                &format!("frequencies[{index}].profiles[0].mode_solver"),
            ));
        }
        values += case.ranges_m().len()
            + case
                .profiles()
                .iter()
                .map(crate::case::input_values)
                .sum::<usize>();
        if values > crate::case::MAX_SEQUENCE_VALUES {
            return Err(error(
                path,
                "KR0201",
                "profile/frequency cases exceed the cumulative input storage limit",
                "frequencies",
            ));
        }
    }
    Ok(())
}

fn malformed_field(path: &str) -> String {
    // Serde buffers a tagged payload when data precedes type; report its object consistently.
    for boundary in [".bottom_boundary", ".surface_boundary"] {
        if let Some((prefix, suffix)) = path.split_once(boundary)
            && (suffix.is_empty() || suffix.starts_with('.'))
        {
            return format!("{prefix}{boundary}");
        }
    }
    path.to_owned()
}

fn error(path: &Path, code: &'static str, message: &str, field: &str) -> DiagnosticReport {
    DiagnosticReport::one(Diagnostic::new(code, message, field, path, 1, 1))
}

fn locate(mut report: DiagnosticReport, path: &Path, prefix: &str) -> DiagnosticReport {
    for diagnostic in &mut report.diagnostics {
        diagnostic.path = path.to_path_buf();
        diagnostic.field = format!("{prefix}.{}", diagnostic.field);
    }
    report
}

pub(crate) mod complex {
    use num_complex::Complex64;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    #[derive(Serialize, Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Value {
        real: f64,
        imaginary: f64,
    }

    pub fn serialize<S: Serializer>(value: &Complex64, serializer: S) -> Result<S::Ok, S::Error> {
        Value {
            real: value.re,
            imaginary: value.im,
        }
        .serialize(serializer)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Complex64, D::Error> {
        let value = Value::deserialize(deserializer)?;
        Ok(Complex64::new(value.real, value.imaginary))
    }
}

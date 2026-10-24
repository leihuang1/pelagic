//! Complete legacy FIELD input snapshots, shared by export and HDF5 runners.
pub(crate) mod attenuation;
pub mod json;
pub mod legacy;
pub(crate) mod material;

use std::path::{Path, PathBuf};

use crate::{DiagnosticReport, FieldCase, ModeSolver};

/// Exact UTF-8 bytes consumed under one legacy resource role.
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
    /// The equivalent JSON document, keeping the `.env` loss recipe. It is
    /// built from the snapshots on request, so loading and solving never
    /// depend on it.
    /// # Errors
    /// Returns the reader's diagnostics if a snapshot cannot be translated.
    pub fn document(&self) -> Result<json::CaseDocument, DiagnosticReport> {
        let snapshot = |role: &str| self.snapshots.iter().find(|s| s.role == role);
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
        cases,
        snapshots,
        solver,
    })
}

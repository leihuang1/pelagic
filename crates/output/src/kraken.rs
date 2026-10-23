//! Sequential legacy/JSON KRAKEN/KRAKENC runs; HDF5 v1 and protected CLI native groups.
//! This schema is independent of BELLHOP schema v3; no solver result types are unified.

use std::fmt;
use std::fs;
use std::io::{Read, Write};
use std::path::Path;

use hdf5::{File, Group, H5Type};
use kraken::{
    Case, DiagnosticReport, FieldCase, FieldPropagation, ModeAddition, ModeSet, ModeSolver,
    PressureField, ProfileSimulationResult, SourceGeometry,
};
use sha2::{Digest, Sha256};

use super::hdf5::{hdf5_error, write_scalar_attribute, write_string_attribute};
use super::publication::{check_file_size, publish};

pub const SCHEMA_VERSION: u32 = 1;
pub const DEFAULT_MAX_OUTPUT_BYTES: u64 = 256 * 1024 * 1024;

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RunSummary {
    pub frequency_count: usize,
    pub mode_count: u64,
    pub pressure_count: u64,
}

#[derive(Debug)]
pub enum RunError {
    Input(DiagnosticReport),
    Simulation {
        frequency_index: usize,
        frequency_hz: f64,
        report: DiagnosticReport,
    },
    Output(String),
}

impl fmt::Display for RunError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Input(report) => report.fmt(formatter),
            Self::Simulation {
                frequency_index,
                frequency_hz,
                report,
            } => {
                write!(
                    formatter,
                    "frequency[{frequency_index}] ({frequency_hz} Hz): {report}"
                )
            }
            Self::Output(message) => write!(formatter, "error[KR0402]: {message}"),
        }
    }
}

impl std::error::Error for RunError {}

impl From<String> for RunError {
    fn from(message: String) -> Self {
        Self::Output(message)
    }
}

/// Solve one supported legacy pair and publish a complete, versioned HDF5 result.
///
/// Exact input snapshots are parsed and hashed once. Frequencies are solved/written
/// sequentially; no collection of all frequency results is retained. Dataset payload
/// and flushed file size are bounded by `max_output_bytes` (default 256 MiB in the CLI).
/// Existing per-frequency numerical work limits remain unchanged.
///
/// Publication is atomic: without `overwrite`, a hard link refuses an existing
/// destination, including one created during the run. With it, rename replaces the
/// old output only after every frequency succeeds. Inputs cannot be destinations.
/// A pre-existing `<output>.tmp` is never truncated or removed.
///
/// # Errors
///
/// Returns input diagnostics, numerical diagnostics, or a quota/filesystem/HDF5 error.
/// No incomplete result is published on failure.
pub fn run_legacy(
    env_path: &Path,
    flp_path: &Path,
    output_path: &Path,
    solver: ModeSolver,
    overwrite: bool,
    max_output_bytes: u64,
) -> Result<RunSummary, RunError> {
    if max_output_bytes == 0 {
        return Err(RunError::Output("max_output_bytes must be positive".into()));
    }
    let input = kraken::input::load_legacy(env_path, flp_path, solver).map_err(RunError::Input)?;
    let snapshots: Vec<_> = input
        .snapshots()
        .iter()
        .map(|snapshot| (snapshot.role(), snapshot.path(), snapshot.source()))
        .collect();
    run_cases(
        input.cases(),
        &snapshots,
        output_path,
        overwrite,
        max_output_bytes,
    )
}

/// Run a strict self-contained JSON snapshot without reading auxiliary files.
/// `solver`, if supplied, must match the document; it never overrides its physics.
/// Uses the same quotas, HDF5 v1 writer and atomic publication as `run_legacy`.
/// # Errors
/// Returns input, numerical or output errors; no incomplete product is published.
pub fn run_json(
    path: &Path,
    output_path: &Path,
    solver: Option<ModeSolver>,
    overwrite: bool,
    max_output_bytes: u64,
) -> Result<RunSummary, RunError> {
    let source = read_source(path)?;
    let cases =
        kraken::json::load_case_document_named(source.as_bytes(), path).map_err(RunError::Input)?;
    if solver.is_some_and(|solver| solver != cases[0].profiles()[0].mode_solver) {
        return Err(RunError::Input(input_report(
            path,
            "KR0202",
            "--solver must match the JSON document",
            "mode_solver",
        )));
    }
    run_cases(
        &cases,
        &[("json", path, source.as_str())],
        output_path,
        overwrite,
        max_output_bytes,
    )
}

fn check_field_payload(cases: &[FieldCase], max_output_bytes: u64) -> Result<(), RunError> {
    if max_output_bytes == 0 {
        return Err(RunError::Output("max_output_bytes must be positive".into()));
    }
    // Admit the known FIELD payload before doing numerical work. Modes are charged
    // as they are written, because their counts are not known before solving.
    let pressure_bytes = cases.iter().try_fold(0_u64, |total, sequence| {
        let case = &sequence.profiles()[0];
        total.checked_add(
            (case.source_depths_m.len() as u64)
                .checked_mul(case.receiver_depths_m.len() as u64)?
                .checked_mul(case.receiver_ranges_m.len() as u64)?
                .checked_mul(8)?,
        )
    });
    if pressure_bytes.is_none_or(|bytes| bytes > max_output_bytes) {
        return Err(RunError::Output(format!(
            "FIELD payload exceeds {max_output_bytes} bytes"
        )));
    }
    Ok(())
}

fn run_cases(
    cases: &[FieldCase],
    inputs: &[(&str, &Path, &str)],
    output_path: &Path,
    overwrite: bool,
    max_output_bytes: u64,
) -> Result<RunSummary, RunError> {
    check_field_payload(cases, max_output_bytes)?;
    publish(
        output_path,
        &inputs.iter().map(|(_, path, _)| *path).collect::<Vec<_>>(),
        overwrite,
        Some(max_output_bytes),
        "KR0402",
        |file, temporary| {
            write_run(
                cases,
                inputs,
                Some((file, temporary)),
                None,
                max_output_bytes,
            )
        },
    )
}

/// Run legacy inputs once and publish selected CLI formats as a protected file group.
/// # Errors
/// Returns input, ordered numerical, quota or recoverable publication errors.
pub fn run_legacy_directory(
    env: &Path,
    flp: &Path,
    output: &Path,
    solver: ModeSolver,
    format: super::directory::Format,
    maximum: u64,
) -> Result<RunSummary, RunError> {
    let input = kraken::input::load_legacy(env, flp, solver).map_err(RunError::Input)?;
    let snapshots = input
        .snapshots()
        .iter()
        .map(|s| (s.role(), s.path(), s.source()))
        .collect::<Vec<_>>();
    run_directory(
        input.cases(),
        &snapshots,
        output,
        &super::directory::stem(env)?,
        format,
        maximum,
    )
}

/// Run self-contained JSON once; an explicit solver remains an assertion, not an override.
/// # Errors
/// Returns input, ordered numerical, quota or recoverable publication errors.
pub fn run_json_directory(
    path: &Path,
    output: &Path,
    solver: Option<ModeSolver>,
    format: super::directory::Format,
    maximum: u64,
) -> Result<RunSummary, RunError> {
    let source = read_source(path)?;
    run_json_source_directory(path, &source, &[path], output, solver, format, maximum)
}

/// Run an exact JSON snapshot, including stdin, without rereading its source.
/// # Errors
/// Returns structured input, numerical or publication errors.
pub fn run_json_source_directory(
    path: &Path,
    source: &str,
    protected_inputs: &[&Path],
    output: &Path,
    solver: Option<ModeSolver>,
    format: super::directory::Format,
    maximum: u64,
) -> Result<RunSummary, RunError> {
    let cases =
        kraken::json::load_case_document_named(source.as_bytes(), path).map_err(RunError::Input)?;
    if solver.is_some_and(|s| s != cases[0].profiles()[0].mode_solver) {
        return Err(RunError::Input(input_report(
            path,
            "KR0202",
            "--solver must match the JSON document",
            "mode_solver",
        )));
    }
    run_directory_protected(
        &cases,
        &[("json", path, source)],
        protected_inputs,
        output,
        &super::directory::stem(path)?,
        format,
        maximum,
    )
}

fn run_directory(
    cases: &[FieldCase],
    inputs: &[(&str, &Path, &str)],
    output: &Path,
    stem: &str,
    format: super::directory::Format,
    maximum: u64,
) -> Result<RunSummary, RunError> {
    run_directory_protected(
        cases,
        inputs,
        &inputs.iter().map(|(_, p, _)| *p).collect::<Vec<_>>(),
        output,
        stem,
        format,
        maximum,
    )
}

fn run_directory_protected(
    cases: &[FieldCase],
    inputs: &[(&str, &Path, &str)],
    protected_inputs: &[&Path],
    output: &Path,
    stem: &str,
    format: super::directory::Format,
    maximum: u64,
) -> Result<RunSummary, RunError> {
    check_field_payload(cases, maximum)?;
    super::directory::publish(output, protected_inputs, Some(maximum), |scratch| {
        let path = scratch.join(format!("{stem}.h5"));
        let file = format
            .hdf5()
            .then(|| File::create_excl(&path).map_err(hdf5_error))
            .transpose()?;
        let summary = write_run(
            cases,
            inputs,
            file.as_ref().map(|f| (f, path.as_path())),
            Some((scratch, stem, format)),
            maximum,
        )?;
        if let Some(file) = file {
            file.close().map_err(hdf5_error)?;
        }
        Ok(summary)
    })
}

#[allow(clippy::too_many_lines, clippy::unnecessary_debug_formatting)] // Quote input paths so embedded newlines cannot spoof report fields.
fn write_run(
    cases: &[FieldCase],
    inputs: &[(&str, &Path, &str)],
    hdf5: Option<(&File, &Path)>,
    directory: Option<(&Path, &str, super::directory::Format)>,
    maximum: u64,
) -> Result<RunSummary, RunError> {
    let mut budget = Budget {
        payload: 0,
        maximum,
    };
    let frequencies = if let Some((file, path)) = hdf5 {
        write_header(file, cases, inputs, &mut budget)?;
        check_file(file, path, maximum)?;
        Some(file.create_group("frequencies").map_err(hdf5_error)?)
    } else {
        None
    };
    let mut report = directory
        .map(|(root, stem, _)| native_output::create(&root.join(format!("{stem}.prt"))))
        .transpose()
        .map_err(|e| RunError::Output(e.to_string()))?;
    if let Some(report) = &mut report {
        writeln!(report, "Pelagic {} KRAKEN run report (not the Fortran PRT format)\nreference: Acoustics Toolbox v2023.5\nformat: {:?}\nmax_output_bytes: {maximum}", env!("CARGO_PKG_VERSION"), directory.unwrap().2).map_err(|e| e.to_string())?;
        for (role, path, source) in inputs {
            writeln!(
                report,
                "input {role}: {:?} sha256={:x}",
                path.as_os_str(),
                Sha256::digest(source.as_bytes())
            )
            .map_err(|e| e.to_string())?;
        }
    }
    let mut summary = RunSummary {
        frequency_count: cases.len(),
        ..RunSummary::default()
    };
    // Exactly one iterator carries the existing cross-frequency seeds and stops at first error.
    for (index, (case, result)) in cases
        .iter()
        .zip(kraken::solve_frequencies(cases))
        .enumerate()
    {
        let result = result.map_err(|report| RunError::Simulation {
            frequency_index: index,
            frequency_hz: case.profiles()[0].frequency_hz,
            report,
        })?;
        if let (Some(frequencies), Some((file, path))) = (&frequencies, hdf5) {
            let group = frequencies
                .create_group(&index.to_string())
                .map_err(hdf5_error)?;
            write_frequency(&group, case, &result, &mut budget, file, path)?;
            check_file(file, path, maximum)?;
        }
        if let Some((root, stem, format)) = directory {
            let name = if cases.len() == 1 {
                stem.to_owned()
            } else {
                format!("{stem}.f{index:04}")
            };
            if format.legacy() {
                super::kraken_native::write(root, &name, case, &result, maximum)?;
            }
            if let Some(report) = &mut report {
                writeln!(report, "frequency[{index}] {} Hz; native_stem={name}; profiles={} ranges_m={:?}; modes={:?}; pressures={}", case.profiles()[0].frequency_hz, case.profiles().len(), case.ranges_m(), result.modes.iter().map(|m| m.modes.len()).collect::<Vec<_>>(), result.field.pressure.len()).map_err(|e| e.to_string())?;
                for (profile, environment) in case.profiles().iter().enumerate() {
                    writeln!(report, "  profile[{profile}] title={:?}; solver={:?}; mesh_reference_frequency_hz={}; source_geometry={:?}; mode_addition={:?}", environment.title, environment.mode_solver, environment.mesh_reference_frequency_hz.unwrap_or(environment.frequency_hz), environment.source_geometry, environment.mode_addition).map_err(|e| e.to_string())?;
                }
                report.flush().map_err(|e| e.to_string())?;
            }
            super::directory::check_quota(root, Some(maximum))?;
        }
        summary.mode_count += result
            .modes
            .iter()
            .map(|m| m.modes.len() as u64)
            .sum::<u64>();
        summary.pressure_count += result.field.pressure.len() as u64;
        if let Some((file, path)) = hdf5 {
            check_file(file, path, maximum)?;
        }
    }
    drop(frequencies);
    if let Some((file, path)) = hdf5 {
        write_scalar_attribute(file, "mode_count", &summary.mode_count)?;
        write_scalar_attribute(file, "pressure_count", &summary.pressure_count)?;
        check_file(file, path, maximum)?;
    }
    if let Some(mut report) = report {
        report.flush().map_err(|e| e.to_string())?;
    }
    Ok(summary)
}

fn read_source(path: &Path) -> Result<String, RunError> {
    let mut source = String::new();
    fs::File::open(path)
        .and_then(|file| {
            file.take(kraken::legacy::MAX_INPUT_BYTES + 1)
                .read_to_string(&mut source)
        })
        .map_err(|e| {
            RunError::Input(input_report(
                path,
                "KR0001",
                &format!("unable to read input: {e}"),
                "input",
            ))
        })?;
    Ok(source)
}

fn input_report(path: &Path, code: &'static str, message: &str, field: &str) -> DiagnosticReport {
    DiagnosticReport::one(kraken::Diagnostic::new(code, message, field, path, 1, 1))
}

struct Budget {
    payload: u64,
    maximum: u64,
}

impl Budget {
    fn reserve(&mut self, bytes: u64) -> Result<(), String> {
        self.payload = self
            .payload
            .checked_add(bytes)
            .filter(|&n| n <= self.maximum)
            .ok_or_else(|| format!("dataset payload exceeds {} bytes", self.maximum))?;
        Ok(())
    }
}

fn check_file(file: &File, path: &Path, maximum: u64) -> Result<(), String> {
    // ponytail: physical quota checked after header/profile/frequency flushes; a bounded HDF5 VFD
    // is needed for a strict in-write disk quota, not for sequential bounded results.
    file.flush().map_err(hdf5_error)?;
    check_file_size(path, maximum)
}

fn write_header(
    file: &File,
    cases: &[FieldCase],
    inputs: &[(&str, &Path, &str)],
    budget: &mut Budget,
) -> Result<(), String> {
    write_scalar_attribute(file, "schema_version", &SCHEMA_VERSION)?;
    write_string_attribute(file, "schema_name", "kraken")?;
    write_string_attribute(
        file,
        "implementation",
        concat!("Pelagic kraken ", env!("CARGO_PKG_VERSION")),
    )?;
    write_string_attribute(
        file,
        "compatibility_reference",
        "Acoustics Toolbox v2023.5 (475108519289c6fb488b58980c644ea14eccc604)",
    )?;
    write_string_attribute(file, "title", &cases[0].profiles()[0].title)?;
    write_string_attribute(
        file,
        "solver",
        match cases[0].profiles()[0].mode_solver {
            ModeSolver::Kraken => "kraken",
            ModeSolver::Krakenc => "krakenc",
        },
    )?;
    write_string_attribute(
        file,
        "coordinate_convention",
        "range origin at source; depth positive downward",
    )?;
    write_scalar_attribute(file, "frequency_count", &(cases.len() as u64))?;
    write_scalar_attribute(file, "max_output_bytes", &budget.maximum)?;
    let frequencies: Vec<_> = cases
        .iter()
        .map(|case| case.profiles()[0].frequency_hz)
        .collect();
    dataset(
        file,
        "frequency_hz",
        &frequencies,
        &[frequencies.len()],
        "Hz",
        budget,
    )?;
    let group = file.create_group("inputs").map_err(hdf5_error)?;
    for (name, path, source) in inputs {
        let input = group.create_group(name).map_err(hdf5_error)?;
        write_string_attribute(&input, "filename", &path.to_string_lossy())?;
        write_scalar_attribute(&input, "size_bytes", &(source.len() as u64))?;
        write_string_attribute(
            &input,
            "sha256",
            &format!("{:x}", Sha256::digest(source.as_bytes())),
        )?;
    }
    Ok(())
}

fn write_frequency(
    group: &Group,
    case: &FieldCase,
    result: &ProfileSimulationResult,
    budget: &mut Budget,
    file: &File,
    path: &Path,
) -> Result<(), String> {
    write_string_attribute(group, "title", &case.profiles()[0].title)?;
    write_profile_metadata(group, &case.profiles()[0], budget)?;
    write_modes(group, &result.modes[0], budget)?;
    write_field(group, &result.field, budget)?;
    write_scalar_attribute(group, "profile_count", &(case.profiles().len() as u64))?;
    write_string_attribute(
        group,
        "field_propagation",
        match case.propagation() {
            FieldPropagation::RangeIndependent => "range_independent",
            FieldPropagation::Adiabatic => "adiabatic",
            FieldPropagation::Coupled => "coupled",
        },
    )?;
    if case.profiles().len() > 1 {
        dataset(
            group,
            "profile_range_m",
            case.ranges_m(),
            &[case.profiles().len()],
            "m",
            budget,
        )?;
        let profiles = group.create_group("profiles").map_err(hdf5_error)?;
        for (index, (profile, modes)) in case.profiles().iter().zip(&result.modes).enumerate() {
            let child = profiles
                .create_group(&index.to_string())
                .map_err(hdf5_error)?;
            write_string_attribute(&child, "title", &profile.title)?;
            write_scalar_attribute(&child, "range_m", &case.ranges_m()[index])?;
            write_profile_metadata(&child, profile, budget)?;
            if index == 0 {
                group
                    .link_hard("modes", "profiles/0/modes")
                    .map_err(hdf5_error)?;
            } else {
                write_modes(&child, modes, budget)?;
            }
            check_file(file, path, budget.maximum)?;
        }
    }
    Ok(())
}

#[allow(clippy::too_many_lines)] // Keep each profile's material and interface metadata together.
fn write_profile_metadata(group: &Group, case: &Case, budget: &mut Budget) -> Result<(), String> {
    write_scalar_attribute(group, "frequency_hz", &case.frequency_hz)?;
    write_string_attribute(
        group,
        "surface_boundary",
        match &case.surface_boundary {
            kraken::SurfaceBoundary::Vacuum => "V",
            kraken::SurfaceBoundary::Rigid => "R",
            kraken::SurfaceBoundary::FluidHalfSpace
            | kraken::SurfaceBoundary::ElasticHalfSpace { .. } => "A",
            kraken::SurfaceBoundary::Reflection(_) => "F",
            kraken::SurfaceBoundary::Impedance { .. } => "P",
        },
    )?;
    write_string_attribute(
        group,
        "bottom_boundary",
        match &case.bottom_boundary {
            kraken::BottomBoundary::Vacuum => "V",
            kraken::BottomBoundary::FluidHalfSpace
            | kraken::BottomBoundary::ElasticHalfSpace { .. } => "A",
            kraken::BottomBoundary::Rigid => "R",
            kraken::BottomBoundary::Reflection(_) => "F",
            kraken::BottomBoundary::Impedance { .. } => "P",
        },
    )?;
    write_elastic_half_spaces(group, case)?;
    write_elastic_layers(group, case, budget)?;
    write_scalar_attribute(
        group,
        "mesh_reference_frequency_hz",
        &case
            .mesh_reference_frequency_hz
            .unwrap_or(case.frequency_hz),
    )?;
    write_scalar_attribute(group, "requested_mesh_points", &(case.mesh_points as u64))?;
    write_scalar_attribute(group, "max_range_m", &case.max_range_m)?;
    write_scalar_attribute(group, "field_mode_limit", &(case.mode_limit as u64))?;
    write_string_attribute(
        group,
        "source_geometry",
        match case.source_geometry {
            SourceGeometry::Line => "line",
            SourceGeometry::Point => "point",
            SourceGeometry::ScaledCylindrical => "scaled_cylindrical",
        },
    )?;
    write_string_attribute(
        group,
        "mode_addition",
        match case.mode_addition {
            ModeAddition::Coherent => "coherent",
            ModeAddition::Incoherent => "incoherent",
        },
    )?;
    write_string_attribute(
        group,
        "source_pattern",
        if case.source_pattern.is_empty() {
            "omnidirectional"
        } else {
            "tabulated"
        },
    )?;
    write_scalar_attribute(
        group,
        "source_pattern_point_count",
        &(case.source_pattern.len() as u64),
    )?;

    write_scalar_attribute(
        group,
        "finite_fluid_layer_count",
        &((1 + case.additional_fluid_layers.len()) as u64),
    )?;
    let media = group.create_group("media").map_err(hdf5_error)?;
    let mut top_depth = case.fluid_top_depth_m();
    for index in 0..=case.additional_fluid_layers.len() {
        let (bottom_depth, density, mesh_points) = if index == 0 {
            (
                case.water_depth_m,
                case.water_density_g_cm3,
                case.mesh_points,
            )
        } else {
            let layer = &case.additional_fluid_layers[index - 1];
            (layer.bottom_depth_m, layer.density_g_cm3, layer.mesh_points)
        };
        let layer = media.create_group(&index.to_string()).map_err(hdf5_error)?;
        write_scalar_attribute(&layer, "top_depth_m", &top_depth)?;
        write_scalar_attribute(&layer, "bottom_depth_m", &bottom_depth)?;
        write_scalar_attribute(&layer, "density_g_cm3", &density)?;
        write_scalar_attribute(&layer, "requested_mesh_points", &(mesh_points as u64))?;
        write_scalar_attribute(
            &layer,
            "top_roughness_m",
            &case
                .interface_roughness_m
                .get(index)
                .copied()
                .unwrap_or(0.0),
        )?;
        write_scalar_attribute(
            &layer,
            "bottom_roughness_m",
            &case
                .interface_roughness_m
                .get(index + 1)
                .copied()
                .unwrap_or(0.0),
        )?;
        top_depth = bottom_depth;
    }

    Ok(())
}

#[allow(clippy::too_many_lines)] // Keep finite-elastic metadata and its sampled profile together.
fn write_elastic_layers(group: &Group, case: &Case, budget: &mut Budget) -> Result<(), String> {
    write_scalar_attribute(
        group,
        "finite_elastic_layer_count",
        &((case.top_elastic_layers.len() + case.bottom_elastic_layers.len()) as u64),
    )?;
    if case.top_elastic_layers.is_empty() && case.bottom_elastic_layers.is_empty() {
        return Ok(());
    }
    let media = group.create_group("elastic_media").map_err(hdf5_error)?;
    for (name, layers, mut top) in [
        ("top", &case.top_elastic_layers, 0.0),
        (
            "bottom",
            &case.bottom_elastic_layers,
            case.fluid_bottom_depth_m(),
        ),
    ] {
        let side = media.create_group(name).map_err(hdf5_error)?;
        for (index, material) in layers.iter().enumerate() {
            let layer = side.create_group(&index.to_string()).map_err(hdf5_error)?;
            write_string_attribute(&layer, "material", "elastic")?;
            write_string_attribute(
                &layer,
                "attenuation_model",
                if case.mode_solver == ModeSolver::Kraken {
                    "reference_real_stiffness"
                } else {
                    "complex"
                },
            )?;
            for (attribute, value) in [
                ("top_depth_m", top),
                ("bottom_depth_m", material.bottom_depth_m),
                (
                    "compressional_sound_speed_mps",
                    material.compressional_sound_speed_mps,
                ),
                ("shear_sound_speed_mps", material.shear_sound_speed_mps),
                ("density_g_cm3", material.density_g_cm3),
                (
                    "compressional_attenuation_db_per_wavelength",
                    material.compressional_attenuation_db_per_wavelength,
                ),
                (
                    "shear_attenuation_db_per_wavelength",
                    material.shear_attenuation_db_per_wavelength,
                ),
            ] {
                write_scalar_attribute(&layer, attribute, &value)?;
            }
            write_scalar_attribute(
                &layer,
                "requested_mesh_points",
                &(material.mesh_points as u64),
            )?;
            let points = &material.material_profile;
            write_scalar_attribute(
                &layer,
                "material_profile_point_count",
                &(points.len() as u64),
            )?;
            if !points.is_empty() {
                let profile = layer.create_group("material_profile").map_err(hdf5_error)?;
                for (name, unit, values) in [
                    (
                        "depth_m",
                        "m",
                        points.iter().map(|p| p.depth_m).collect::<Vec<_>>(),
                    ),
                    (
                        "compressional_sound_speed_mps",
                        "m/s",
                        points
                            .iter()
                            .map(|p| p.compressional_sound_speed_mps)
                            .collect(),
                    ),
                    (
                        "shear_sound_speed_mps",
                        "m/s",
                        points.iter().map(|p| p.shear_sound_speed_mps).collect(),
                    ),
                    (
                        "density_g_cm3",
                        "g/cm^3",
                        points.iter().map(|p| p.density_g_cm3).collect(),
                    ),
                    (
                        "compressional_attenuation_db_per_wavelength",
                        "dB/wavelength",
                        points
                            .iter()
                            .map(|p| p.compressional_attenuation_db_per_wavelength)
                            .collect(),
                    ),
                    (
                        "shear_attenuation_db_per_wavelength",
                        "dB/wavelength",
                        points
                            .iter()
                            .map(|p| p.shear_attenuation_db_per_wavelength)
                            .collect(),
                    ),
                ] {
                    dataset(&profile, name, &values, &[points.len()], unit, budget)?;
                }
            }
            top = material.bottom_depth_m;
        }
    }
    Ok(())
}

fn write_elastic_half_spaces(group: &Group, case: &Case) -> Result<(), String> {
    for (name, boundary, cp, density, loss) in [
        (
            "surface",
            &case.surface_boundary,
            case.surface_sound_speed_mps,
            case.surface_density_g_cm3,
            case.surface_attenuation_db_per_wavelength,
        ),
        (
            "bottom",
            &case.bottom_boundary,
            case.bottom_sound_speed_mps,
            case.bottom_density_g_cm3,
            case.bottom_attenuation_db_per_wavelength,
        ),
    ] {
        if let kraken::Boundary::ElasticHalfSpace {
            shear_sound_speed_mps,
            shear_attenuation_db_per_wavelength,
        } = boundary
        {
            write_string_attribute(group, &format!("{name}_half_space_material"), "elastic")?;
            write_string_attribute(
                group,
                &format!("{name}_elastic_attenuation_model"),
                if case.mode_solver == ModeSolver::Kraken {
                    "reference_real"
                } else {
                    "complex"
                },
            )?;
            for (attribute, value) in [
                ("sound_speed_mps", cp),
                ("density_g_cm3", density),
                ("attenuation_db_per_wavelength", loss),
                ("shear_sound_speed_mps", *shear_sound_speed_mps),
                (
                    "shear_attenuation_db_per_wavelength",
                    *shear_attenuation_db_per_wavelength,
                ),
            ] {
                write_scalar_attribute(group, &format!("{name}_{attribute}"), &value)?;
            }
        }
    }
    Ok(())
}

fn write_modes(group: &Group, result: &ModeSet, budget: &mut Budget) -> Result<(), String> {
    let modes = group.create_group("modes").map_err(hdf5_error)?;
    let count = result.modes.len();
    let depths = &result.sampled_depths_m;
    write_string_attribute(&modes, "eigenfunction_axis_order", "mode,sample_depth")?;
    write_string_attribute(
        &modes,
        "normalization",
        "pinned AT density-weighted normalization; fluid/elastic boundary-admittance derivative when present; arbitrary unit phase",
    )?;
    dataset(
        &modes,
        "sample_depth_m",
        depths,
        &[depths.len()],
        "m",
        budget,
    )?;
    for (name, unit, values) in [
        (
            "horizontal_wavenumber_real",
            "rad/m",
            result
                .modes
                .iter()
                .map(|m| m.horizontal_wavenumber_rad_per_m.re)
                .collect::<Vec<_>>(),
        ),
        (
            "horizontal_wavenumber_imaginary",
            "rad/m",
            result
                .modes
                .iter()
                .map(|m| m.horizontal_wavenumber_rad_per_m.im)
                .collect(),
        ),
        (
            "phase_speed_mps",
            "m/s",
            result.modes.iter().map(|m| m.phase_speed_mps).collect(),
        ),
        (
            "group_speed_mps",
            "m/s",
            result.modes.iter().map(|m| m.group_speed_mps).collect(),
        ),
        (
            "attenuation_nepers_per_m",
            "neper/m",
            result
                .modes
                .iter()
                .map(|m| m.attenuation_nepers_per_m)
                .collect(),
        ),
    ] {
        dataset(&modes, name, &values, &[count], unit, budget)?;
    }
    for imaginary in [false, true] {
        let values: Vec<_> = result
            .modes
            .iter()
            .flat_map(|m| &m.eigenfunction)
            .map(|value| if imaginary { value.im } else { value.re })
            .collect();
        dataset(
            &modes,
            if imaginary {
                "eigenfunction_imaginary"
            } else {
                "eigenfunction_real"
            },
            &values,
            &[count, depths.len()],
            "reference_normalized",
            budget,
        )?;
    }
    Ok(())
}

#[allow(clippy::cast_possible_truncation)]
fn write_field(group: &Group, result: &PressureField, budget: &mut Budget) -> Result<(), String> {
    let field = group.create_group("field").map_err(hdf5_error)?;
    write_string_attribute(
        &field,
        "pressure_axis_order",
        "source_depth,receiver_depth,receiver_range",
    )?;
    for (name, values) in [
        ("source_depth_m", &result.source_depths_m),
        ("receiver_depth_m", &result.receiver_depths_m),
        ("receiver_range_m", &result.receiver_ranges_m),
        ("receiver_offset_m", &result.receiver_offsets_m),
    ] {
        dataset(&field, name, values, &[values.len()], "m", budget)?;
    }
    let shape = [
        result.source_depths_m.len(),
        result.receiver_depths_m.len(),
        result.receiver_ranges_m.len(),
    ];
    for imaginary in [false, true] {
        let values: Vec<_> = result
            .pressure
            .iter()
            .map(|value| (if imaginary { value.im } else { value.re }) as f32)
            .collect();
        dataset(
            &field,
            if imaginary {
                "pressure_imaginary"
            } else {
                "pressure_real"
            },
            &values,
            &shape,
            "1",
            budget,
        )?;
    }
    Ok(())
}

fn dataset<T: H5Type + Copy + Into<f64>>(
    group: &Group,
    name: &str,
    values: &[T],
    shape: &[usize],
    unit: &str,
    budget: &mut Budget,
) -> Result<(), String> {
    if values.iter().any(|&value| !value.into().is_finite()) {
        return Err(format!("non-finite output in {name}"));
    }
    budget.reserve(
        (values.len() as u64)
            .checked_mul(std::mem::size_of::<T>() as u64)
            .ok_or("dataset byte count overflow")?,
    )?;
    let data = group
        .new_dataset::<T>()
        .shape(shape)
        .create(name)
        .map_err(hdf5_error)?;
    data.write_raw(values).map_err(hdf5_error)?;
    write_string_attribute(&data, "unit", unit)
}

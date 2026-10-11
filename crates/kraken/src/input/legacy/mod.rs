use std::collections::HashMap;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

mod material;

use crate::attenuation::VolumeLoss;
use crate::{
    BottomBoundary, Case, CaseDefinition, Diagnostic, DiagnosticReport, Interpolation,
    MAX_VECTOR_LENGTH, ModeAddition, ModeSolver, SoundSpeedPoint, SourceGeometry,
    SourcePatternPoint, SurfaceBoundary,
};

/// Maximum byte length of each legacy input, including source-based loading.
pub const MAX_INPUT_BYTES: u64 = 1_048_576;

const MAX_PROFILE_POINTS: usize = MAX_VECTOR_LENGTH;
pub(crate) const MAX_FREQUENCIES: usize = 1000;
const MAX_FREQUENCY_INPUT_VALUES: usize = 5_000_000;

/// Load the supported layered-fluid subset of a KRAKEN `.env` and FIELD `.flp` pair.
///
/// # Errors
///
/// Returns structured parse, validation, and input-file diagnostics.
#[allow(clippy::missing_panics_doc)] // parser guarantees exactly one case
pub fn load_case(
    env_path: impl AsRef<Path>,
    flp_path: impl AsRef<Path>,
) -> Result<Case, DiagnosticReport> {
    load_file_cases(
        env_path.as_ref(),
        flp_path.as_ref(),
        ModeSolver::Kraken,
        true,
    )
    .map(|mut cases| cases.pop().unwrap())
}

/// Load a KRAKENC environment and supported range-independent FIELD geometry.
///
/// # Errors
///
/// Returns structured input diagnostics for unsupported configurations.
#[allow(clippy::missing_panics_doc)] // parser guarantees exactly one case
pub fn load_complex_case(
    env_path: impl AsRef<Path>,
    flp_path: impl AsRef<Path>,
) -> Result<Case, DiagnosticReport> {
    load_file_cases(
        env_path.as_ref(),
        flp_path.as_ref(),
        ModeSolver::Krakenc,
        true,
    )
    .map(|mut cases| cases.pop().unwrap())
}

/// Load one validated case per frequency, in legacy input order.
///
/// Each case can be passed to `solve`; results remain frequency-domain products.
/// Single-frequency inputs also work. Geometry/profile copies are bounded.
///
/// # Errors
///
/// Returns structured parse, mesh-scaling, and case validation diagnostics.
pub fn load_frequency_cases(
    env_path: impl AsRef<Path>,
    flp_path: impl AsRef<Path>,
    mode_solver: ModeSolver,
) -> Result<Vec<Case>, DiagnosticReport> {
    load_file_cases(env_path.as_ref(), flp_path.as_ref(), mode_solver, false)
}

fn load_file_cases(
    env_path: &Path,
    flp_path: &Path,
    mode_solver: ModeSolver,
    single_frequency: bool,
) -> Result<Vec<Case>, DiagnosticReport> {
    let env = read_file(env_path)?;
    let flp = read_file(flp_path)?;
    let [surface, brc, irc] = first_environment_table_extensions(&env, env_path, mode_solver)?;
    let bottom = brc
        .or(irc)
        .map(|extension| read_file(&env_path.with_extension(extension)))
        .transpose()?;
    let surface = surface
        .map(|extension| read_file(&env_path.with_extension(extension)))
        .transpose()?;
    let pattern = source_pattern_extension(&flp, flp_path)?
        .map(|extension| read_file(&flp_path.with_extension(extension)))
        .transpose()?;
    parse_frequency_cases_with_resources(
        &env,
        &flp,
        env_path,
        flp_path,
        mode_solver,
        single_frequency,
        [surface.as_deref(), bottom.as_deref()],
        pattern.as_deref(),
    )
}

/// Load frequency cases from exact UTF-8 input snapshots, preserving source locations.
///
/// This lets adapters record hashes of the bytes actually parsed, without rereading files.
/// The same input-size and validation limits as the file loader apply.
///
/// # Errors
///
/// Returns structured input-size, parse, mesh-scaling, and case diagnostics.
pub fn load_frequency_cases_from_sources(
    env_source: &str,
    flp_source: &str,
    env_path: &Path,
    flp_path: &Path,
    mode_solver: ModeSolver,
) -> Result<Vec<Case>, DiagnosticReport> {
    for (source, path) in [(env_source, env_path), (flp_source, flp_path)] {
        check_input_size(source, path)?;
    }
    parse_frequency_cases(
        env_source,
        flp_source,
        env_path,
        flp_path,
        mode_solver,
        false,
    )
}

#[cfg(test)]
fn parse_case(
    env_source: &str,
    flp_source: &str,
    env_path: &Path,
    flp_path: &Path,
) -> Result<Case, DiagnosticReport> {
    parse_case_with_solver(
        env_source,
        flp_source,
        env_path,
        flp_path,
        ModeSolver::Kraken,
    )
}

#[cfg(test)]
fn parse_case_with_solver(
    env_source: &str,
    flp_source: &str,
    env_path: &Path,
    flp_path: &Path,
    mode_solver: ModeSolver,
) -> Result<Case, DiagnosticReport> {
    parse_frequency_cases(
        env_source,
        flp_source,
        env_path,
        flp_path,
        mode_solver,
        true,
    )
    .map(|mut cases| cases.pop().unwrap())
}

fn parse_frequency_cases(
    env_source: &str,
    flp_source: &str,
    env_path: &Path,
    flp_path: &Path,
    mode_solver: ModeSolver,
    single_frequency: bool,
) -> Result<Vec<Case>, DiagnosticReport> {
    parse_frequency_cases_with_resources(
        env_source,
        flp_source,
        env_path,
        flp_path,
        mode_solver,
        single_frequency,
        [None, None],
        None,
    )
}

/// Return the same-stem bottom resource required by this environment.
/// # Errors
/// Returns input-size and environment parse diagnostics.
pub fn bottom_table_extension(
    source: &str,
    path: &Path,
    solver: ModeSolver,
) -> Result<Option<&'static str>, DiagnosticReport> {
    let [_, brc, irc] = first_environment_table_extensions(source, path, solver)?;
    Ok(brc.or(irc))
}

/// Return the same-stem top reflection resource consumed by this environment.
/// # Errors
/// Returns bounded-input and environment diagnostics.
pub fn surface_table_extension(
    source: &str,
    path: &Path,
    solver: ModeSolver,
) -> Result<Option<&'static str>, DiagnosticReport> {
    Ok(first_environment_table_extensions(source, path, solver)?[0])
}

/// Return the same-stem source-pattern resource consumed by this FIELD input.
/// # Errors
/// Returns bounded-input and FIELD parse diagnostics.
pub fn source_pattern_extension(
    source: &str,
    path: &Path,
) -> Result<Option<&'static str>, DiagnosticReport> {
    check_input_size(source, path)?;
    Ok(parse_field(source, path)?
        .source_pattern_requested
        .then_some("sbp"))
}

/// Parse exact environment, FIELD and optional bottom-table snapshots.
/// # Errors
/// Returns structured input-size, resource, parse and validation diagnostics.
pub fn load_frequency_cases_with_bottom_table(
    env_source: &str,
    flp_source: &str,
    env_path: &Path,
    flp_path: &Path,
    mode_solver: ModeSolver,
    bottom_table: Option<&str>,
) -> Result<Vec<Case>, DiagnosticReport> {
    load_frequency_cases_with_boundary_tables(
        env_source,
        flp_source,
        env_path,
        flp_path,
        mode_solver,
        None,
        bottom_table,
    )
}

/// Parse exact .env/.flp and optional top TRC / bottom BRC or IRC snapshots.
/// Neither resource is reread; old snapshot APIs reject a missing TRC snapshot.
/// # Errors
/// Returns bounded-input, resource, parse and validation diagnostics.
pub fn load_frequency_cases_with_boundary_tables(
    env_source: &str,
    flp_source: &str,
    env_path: &Path,
    flp_path: &Path,
    mode_solver: ModeSolver,
    surface_table: Option<&str>,
    bottom_table: Option<&str>,
) -> Result<Vec<Case>, DiagnosticReport> {
    load_frequency_cases_with_resources(
        env_source,
        flp_source,
        env_path,
        flp_path,
        mode_solver,
        surface_table,
        bottom_table,
        None,
    )
}

// Single-environment discovery deliberately leaves trailing profiles to assembly;
// FIELD discovery below scans them all. Preserve each loader's diagnostic order.
fn first_environment_table_extensions(
    source: &str,
    path: &Path,
    solver: ModeSolver,
) -> Result<[Option<&'static str>; 3], DiagnosticReport> {
    check_input_size(source, path)?;
    let environment = read_environment(&mut Reader::new(source, path)?, solver)?;
    Ok(environment_table_extensions(&environment))
}

fn environment_table_extensions(environment: &Environment) -> [Option<&'static str>; 3] {
    [
        matches!(environment.surface_boundary, SurfaceBoundary::Reflection(_)).then_some("trc"),
        matches!(environment.bottom_boundary, BottomBoundary::Reflection(_)).then_some("brc"),
        matches!(
            environment.bottom_boundary,
            BottomBoundary::Impedance { .. }
        )
        .then_some("irc"),
    ]
}

/// Discover same-stem tables across every ENV profile, in TRC/BRC/IRC order.
/// # Errors
/// Returns bounded-input and environment parse diagnostics.
pub fn field_table_extensions(
    source: &str,
    path: &Path,
    solver: ModeSolver,
) -> Result<[Option<&'static str>; 3], DiagnosticReport> {
    check_input_size(source, path)?;
    let mut reader = Reader::new(source, path)?;
    let mut extensions = [None; 3];
    while reader.index < reader.records.len() {
        let environment = read_environment(&mut reader, solver)?;
        for (slot, extension) in extensions
            .iter_mut()
            .zip(environment_table_extensions(&environment))
        {
            *slot = slot.or(extension);
        }
    }
    Ok(extensions)
}

/// Load ordered legacy environments and FIELD propagation, retaining frequency order.
/// # Errors
/// Returns bounded-input, resource, profile-count and validation diagnostics.
pub fn load_field_cases(
    env_path: impl AsRef<Path>,
    flp_path: impl AsRef<Path>,
    solver: ModeSolver,
) -> Result<Vec<crate::FieldCase>, DiagnosticReport> {
    crate::input::load_legacy(env_path.as_ref(), flp_path.as_ref(), solver)
        .map(crate::input::FieldInput::into_cases)
}

/// Parse exact snapshots into one validated profile sequence per frequency.
/// Tables are TRC/BRC/IRC snapshots; each profile consumes only its required tables.
/// Snapshots not required anywhere in the sequence are rejected.
/// # Errors
/// Returns bounded-input, resource, profile-count and validation diagnostics.
#[allow(clippy::too_many_arguments, clippy::too_many_lines, clippy::float_cmp)]
pub fn load_field_cases_with_resources(
    env_source: &str,
    flp_source: &str,
    env_path: &Path,
    flp_path: &Path,
    solver: ModeSolver,
    tables: [Option<&str>; 3],
    source_pattern: Option<&str>,
) -> Result<Vec<crate::FieldCase>, DiagnosticReport> {
    check_input_size(env_source, env_path)?;
    check_input_size(flp_source, flp_path)?;
    for (extension, source) in ["trc", "brc", "irc"].into_iter().zip(tables) {
        if let Some(source) = source {
            check_input_size(source, &env_path.with_extension(extension))?;
        }
    }
    let field = parse_field(flp_source, flp_path)?;
    let mut reader = Reader::new(env_source, env_path)?;
    let mut sequences: Vec<Vec<Case>> = Vec::new();
    let mut total = 0;
    let mut used_tables = [false; 3];
    for index in 0..field.profile_ranges_m.len() {
        let environment = read_environment(&mut reader, solver)?;
        let required = environment_table_extensions(&environment);
        for (used, extension) in used_tables.iter_mut().zip(required) {
            *used |= extension.is_some();
        }
        let [surface_table, brc, irc] =
            std::array::from_fn(|index| required[index].and(tables[index]));
        let cases = environment_cases(
            environment,
            field.clone(),
            env_path,
            flp_path,
            solver,
            false,
            [surface_table, brc.or(irc)],
            source_pattern,
        )?;
        if index == 0 {
            total += field.profile_ranges_m.len() * cases.len();
            sequences = (0..cases.len())
                .map(|_| Vec::with_capacity(field.profile_ranges_m.len()))
                .collect();
        }
        if cases.len() != sequences.len()
            || cases
                .iter()
                .zip(&sequences)
                .any(|(c, s)| s.first().is_some_and(|p| p.frequency_hz != c.frequency_hz))
        {
            return Err(one(
                "KR0201",
                "profiles must share the exact frequency vector",
                "frequencies_hz",
                env_path,
                reader.last_line,
                1,
            ));
        }
        total += cases.iter().map(crate::case::input_values).sum::<usize>();
        if total > crate::case::MAX_SEQUENCE_VALUES {
            return Err(one(
                "KR0201",
                "profile/frequency cases exceed the cumulative input storage limit",
                "profiles",
                env_path,
                reader.last_line,
                1,
            ));
        }
        for (sequence, case) in sequences.iter_mut().zip(cases) {
            sequence.push(case);
        }
    }
    reader.finish()?;
    for ((extension, source), used) in ["trc", "brc", "irc"]
        .into_iter()
        .zip(tables)
        .zip(used_tables)
    {
        if source.is_some() && !used {
            return Err(one(
                "KR0202",
                "unexpected boundary table snapshot",
                if extension == "trc" {
                    "surface_boundary"
                } else {
                    "bottom_boundary"
                },
                env_path.with_extension(extension),
                1,
                1,
            ));
        }
    }
    sequences
        .into_iter()
        .map(|profiles| {
            crate::FieldCase::new(profiles, field.profile_ranges_m.clone(), field.propagation)
                .map_err(|mut report| {
                    for d in &mut report.diagnostics {
                        d.path = flp_path.to_path_buf();
                        d.line = field.locations["profile_ranges_km"].0;
                    }
                    report
                })
        })
        .collect()
}

/// Parse exact `.env`/`.flp` and optional TRC/BRC/IRC/SBP snapshots without rereading them.
/// # Errors
/// Returns bounded-input, resource, parse and validation diagnostics.
#[allow(clippy::too_many_arguments)]
pub fn load_frequency_cases_with_resources(
    env_source: &str,
    flp_source: &str,
    env_path: &Path,
    flp_path: &Path,
    mode_solver: ModeSolver,
    surface_table: Option<&str>,
    bottom_table: Option<&str>,
    source_pattern: Option<&str>,
) -> Result<Vec<Case>, DiagnosticReport> {
    for (source, path) in [(env_source, env_path), (flp_source, flp_path)] {
        check_input_size(source, path)?;
    }
    parse_frequency_cases_with_resources(
        env_source,
        flp_source,
        env_path,
        flp_path,
        mode_solver,
        false,
        [surface_table, bottom_table],
        source_pattern,
    )
}

#[allow(clippy::float_cmp, clippy::too_many_arguments, clippy::too_many_lines)]
fn parse_frequency_cases_with_resources(
    env_source: &str,
    flp_source: &str,
    env_path: &Path,
    flp_path: &Path,
    mode_solver: ModeSolver,
    single_frequency: bool,
    tables: [Option<&str>; 2],
    source_pattern: Option<&str>,
) -> Result<Vec<Case>, DiagnosticReport> {
    let environment = parse_environment_with_solver(env_source, env_path, mode_solver)?;
    let field = parse_field(flp_source, flp_path)?;
    if field.profile_ranges_m.len() != 1 {
        return Err(one(
            "KR0202",
            "multiple profiles require load_field_cases",
            "profile_ranges_km",
            flp_path,
            field.locations["profile_ranges_km"].0,
            1,
        ));
    }
    environment_cases(
        environment,
        field,
        env_path,
        flp_path,
        mode_solver,
        single_frequency,
        tables,
        source_pattern,
    )
}

#[allow(clippy::float_cmp, clippy::too_many_arguments, clippy::too_many_lines)]
fn environment_cases(
    mut environment: Environment,
    mut field: Field,
    env_path: &Path,
    flp_path: &Path,
    mode_solver: ModeSolver,
    single_frequency: bool,
    tables: [Option<&str>; 2],
    source_pattern: Option<&str>,
) -> Result<Vec<Case>, DiagnosticReport> {
    let [surface_table, bottom_table] = tables;
    if environment.bottom_boundary.is_tabulated() {
        let extension = if matches!(environment.bottom_boundary, BottomBoundary::Reflection(_)) {
            "brc"
        } else {
            "irc"
        };
        let path = env_path.with_extension(extension);
        let source = bottom_table.ok_or_else(|| {
            one(
                "KR0001",
                "required bottom table snapshot is missing",
                "bottom_boundary",
                &path,
                1,
                1,
            )
        })?;
        check_input_size(source, &path)?;
        environment.bottom_boundary = if extension == "brc" {
            parse_reflection_table(source, &path, "bottom_boundary")?
        } else {
            parse_impedance_table(source, &path)?
        };
        crate::solver::reflection::validate_table(
            &environment.bottom_boundary,
            environment.frequency_hz,
        )
        .map_err(|message| one("KR0201", message, "bottom_boundary", &path, 1, 1))?;
    } else if bottom_table.is_some() {
        return Err(one(
            "KR0202",
            "unexpected bottom table",
            "bottom_boundary",
            env_path,
            1,
            1,
        ));
    }
    if matches!(environment.surface_boundary, SurfaceBoundary::Reflection(_)) {
        let path = env_path.with_extension("trc");
        let source = surface_table.ok_or_else(|| {
            one(
                "KR0001",
                "required surface table snapshot is missing",
                "surface_boundary",
                &path,
                1,
                1,
            )
        })?;
        check_input_size(source, &path)?;
        environment.surface_boundary = parse_reflection_table(source, &path, "surface_boundary")?;
        crate::solver::reflection::validate_table(
            &environment.surface_boundary,
            environment.frequency_hz,
        )
        .map_err(|message| one("KR0201", message, "surface_boundary", &path, 1, 1))?;
    } else if surface_table.is_some() {
        return Err(one(
            "KR0202",
            "unexpected surface table",
            "surface_boundary",
            env_path,
            1,
            1,
        ));
    }
    if single_frequency && environment.frequencies_hz.len() != 1 {
        let (line, column) = environment.locations["frequencies_hz"];
        return Err(one(
            "KR0202",
            "multiple frequencies require load_frequency_cases",
            "frequencies_hz",
            env_path,
            line,
            column,
        ));
    }
    let pattern_path = flp_path.with_extension("sbp");
    if field.source_pattern_requested {
        let source = source_pattern.ok_or_else(|| {
            one(
                "KR0001",
                "required source pattern snapshot is missing",
                "source_pattern",
                &pattern_path,
                1,
                1,
            )
        })?;
        check_input_size(source, &pattern_path)?;
        field.source_pattern = parse_source_pattern(source, &pattern_path)?;
    } else if source_pattern.is_some() {
        return Err(one(
            "KR0202",
            "unexpected source pattern",
            "source_pattern",
            &pattern_path,
            1,
            1,
        ));
    }
    // ReadSzRz stores depths in single precision. Keep samples at either fluid
    // interface on the exact f64 boundary, whether the f32 spelling rounds up or down.
    let bottom_depth = environment.materials.fluid_bottom();
    let top_depth = environment.materials.fluid_top();
    #[allow(clippy::cast_possible_truncation)]
    let boundaries = [bottom_depth, top_depth].map(|depth| (f64::from(depth as f32), depth));
    for depths in [
        &mut environment.source_depths,
        &mut environment.receiver_depths,
        &mut field.source_depths,
        &mut field.receiver_depths,
    ] {
        for depth in depths {
            for (rounded, exact) in boundaries {
                if *depth == rounded {
                    *depth = exact;
                    break;
                }
            }
        }
    }

    let mut mode_sample_depths_m = environment.source_depths;
    mode_sample_depths_m.extend(environment.receiver_depths);
    mode_sample_depths_m.sort_by(f64::total_cmp);
    mode_sample_depths_m.dedup_by(|left, right| *left == *right);

    let bottom_location = if environment.bottom_boundary.is_half_space() {
        "bottom_half_space"
    } else {
        "bottom_options"
    };
    let surface_location = if environment.surface_boundary.is_half_space() {
        "surface_half_space"
    } else {
        "top_options"
    };
    let env_locations = environment.locations;
    let field_locations = field.locations;
    // Geometry/options template; Materials supplies canonical material fields
    // only after the cumulative frequency budget has passed.
    let definition = CaseDefinition {
        title: environment.title,
        mode_solver,
        frequency_hz: environment.frequency_hz,
        mesh_reference_frequency_hz: environment.broadband.then_some(environment.frequency_hz),
        water_depth_m: 0.0,
        interpolation: environment.interpolation,
        surface_boundary: environment.surface_boundary,
        surface_sound_speed_mps: 0.0,
        surface_density_g_cm3: 0.0,
        surface_attenuation_db_per_wavelength: 0.0,
        sound_speed_profile: Vec::new(),
        water_density_g_cm3: 0.0,
        water_attenuation_db_per_wavelength: Vec::new(),
        additional_fluid_layers: Vec::new(),
        top_elastic_layers: Vec::new(),
        bottom_elastic_layers: Vec::new(),
        bottom_boundary: environment.bottom_boundary,
        bottom_sound_speed_mps: 0.0,
        bottom_density_g_cm3: 0.0,
        bottom_attenuation_db_per_wavelength: 0.0,
        source_geometry: field.source_geometry,
        mode_addition: field.mode_addition,
        source_pattern: field.source_pattern,
        mesh_points: 0,
        c_low_mps: environment.c_low,
        c_high_mps: environment.c_high,
        max_range_m: environment.max_range_m,
        mode_sample_depths_m,
        mode_limit: field.mode_limit,
        source_depths_m: field.source_depths,
        receiver_depths_m: field.receiver_depths,
        receiver_ranges_m: field.receiver_ranges_m,
        receiver_offsets_m: field.receiver_offsets_m,
    };
    let table_values = match &definition.bottom_boundary {
        BottomBoundary::Reflection(points) => points.len(),
        BottomBoundary::Impedance { points, .. } => points.len(),
        _ => 0,
    };
    let surface_values = match &definition.surface_boundary {
        SurfaceBoundary::Reflection(points) => points.len(),
        _ => 0,
    };
    let values = environment.materials.input_values()
        + table_values
        + surface_values
        + 2 * definition.source_pattern.len()
        + definition.mode_sample_depths_m.len()
        + definition.source_depths_m.len()
        + definition.receiver_depths_m.len()
        + definition.receiver_ranges_m.len()
        + definition.receiver_offsets_m.len();
    if values
        .checked_mul(environment.frequencies_hz.len())
        .is_none_or(|n| n > MAX_FREQUENCY_INPUT_VALUES)
    {
        let &(line, column) = env_locations
            .get("frequencies_hz")
            .unwrap_or(&env_locations["frequency_hz"]);
        return Err(one(
            "KR0201",
            "frequency cases exceed the input storage limit",
            "frequencies_hz",
            env_path,
            line,
            column,
        ));
    }
    environment
        .frequencies_hz
        .into_iter()
        .map(|frequency_hz| {
            let input = environment
                .materials
                .case_definition(&definition, frequency_hz);
            Case::from_definition(input)
                .and_then(|case| {
                    if case.mesh_reference_frequency_hz.is_some() {
                        case.mesh_points_at(1)?;
                    }
                    Ok(case)
                })
                .map_err(|mut report| {
                    for diagnostic in &mut report.diagnostics {
                        if diagnostic.field == "source_pattern" {
                            diagnostic.path.clone_from(&pattern_path);
                            diagnostic.line = 1;
                            diagnostic.column = 1;
                            continue;
                        }
                        let layer_record = diagnostic
                            .field
                            .rsplit_once('.')
                            .filter(|(prefix, _)| {
                                prefix.starts_with("additional_fluid_layers[")
                                    || prefix.starts_with("top_elastic_layers[")
                                    || prefix.starts_with("bottom_elastic_layers[")
                            })
                            .map(|(prefix, suffix)| {
                                format!(
                                    "{prefix}.{}",
                                    if matches!(suffix, "bottom_depth_m" | "mesh_points") {
                                        "header"
                                    } else {
                                        "sound_speed_profile"
                                    }
                                )
                            });
                        let (locations, path, record) = match diagnostic.field.as_str() {
                            "water_depth_m" | "mesh_points" => {
                                (&env_locations, env_path, "water_header")
                            }
                            "sound_speed_profile"
                            | "water_density_g_cm3"
                            | "water_attenuation_db_per_wavelength" => {
                                (&env_locations, env_path, "sound_speed_profile")
                            }
                            "surface_boundary"
                            | "surface_sound_speed_mps"
                            | "surface_density_g_cm3"
                            | "surface_attenuation_db_per_wavelength" => {
                                (&env_locations, env_path, surface_location)
                            }
                            "bottom_boundary"
                            | "bottom_sound_speed_mps"
                            | "bottom_density_g_cm3"
                            | "bottom_attenuation_db_per_wavelength" => {
                                (&env_locations, env_path, bottom_location)
                            }
                            "max_range_m" => (&env_locations, env_path, "max_range_km"),
                            "mesh_reference_frequency_hz" => {
                                (&env_locations, env_path, "frequency_hz")
                            }
                            "mode_sample_depths_m" => {
                                (&env_locations, env_path, "mode_receiver_depths_m")
                            }
                            "source_depths_m" => {
                                (&field_locations, flp_path, "field_source_depths_m")
                            }
                            "receiver_depths_m" => {
                                (&field_locations, flp_path, "field_receiver_depths_m")
                            }
                            "receiver_ranges_m" | "field_grid" => {
                                (&field_locations, flp_path, "receiver_ranges_km")
                            }
                            "receiver_offsets_m" => {
                                (&field_locations, flp_path, "receiver_offsets_m")
                            }
                            "mode_limit" => (&field_locations, flp_path, "mode_limit"),
                            other => (&env_locations, env_path, other),
                        };
                        if let Some(&(line, column)) =
                            locations.get(layer_record.as_deref().unwrap_or(record))
                        {
                            diagnostic.path = path.to_path_buf();
                            diagnostic.line = line;
                            diagnostic.column = column;
                        }
                    }
                    report
                })
        })
        .collect()
}

pub(crate) fn read_file(path: &Path) -> Result<String, DiagnosticReport> {
    let mut source = String::new();
    File::open(path)
        .and_then(|file| file.take(MAX_INPUT_BYTES + 1).read_to_string(&mut source))
        .map_err(|error| {
            one(
                "KR0001",
                format!("unable to read input: {error}"),
                "input",
                path,
                1,
                1,
            )
        })?;
    check_input_size(&source, path)?;
    Ok(source)
}

fn check_input_size(source: &str, path: &Path) -> Result<(), DiagnosticReport> {
    if source.len() as u64 > MAX_INPUT_BYTES {
        return Err(one(
            "KR0201",
            "input file exceeds 1 MiB",
            "input",
            path,
            1,
            1,
        ));
    }
    Ok(())
}

#[derive(Clone)]
struct Token {
    text: String,
    line: usize,
    column: usize,
}

struct Record {
    tokens: Vec<Token>,
    line: usize,
    slash: bool,
}

struct Reader {
    path: PathBuf,
    records: Vec<Record>,
    index: usize,
    last_line: usize,
    eof_line: usize,
    locations: HashMap<String, (usize, usize)>,
}

impl Reader {
    fn new(source: &str, path: &Path) -> Result<Self, DiagnosticReport> {
        let mut records = Vec::new();
        for (index, line_text) in source.lines().enumerate() {
            let line = index + 1;
            let (tokens, slash) = tokenize(line_text, line, path)?;
            if !tokens.is_empty() || slash {
                records.push(Record {
                    tokens,
                    line,
                    slash,
                });
            }
        }
        Ok(Self {
            path: path.to_path_buf(),
            records,
            index: 0,
            last_line: 0,
            eof_line: source.lines().count() + 1,
            locations: HashMap::new(),
        })
    }

    fn record(&mut self, field: &str) -> Result<Record, DiagnosticReport> {
        let Some(record) = self.records.get(self.index) else {
            return Err(one(
                "KR0101",
                "unexpected end of file",
                field,
                &self.path,
                self.eof_line,
                1,
            ));
        };
        self.index += 1;
        self.last_line = record.line;
        self.locations.entry(field.to_owned()).or_insert((
            record.line,
            record.tokens.first().map_or(1, |token| token.column),
        ));
        Ok(Record {
            tokens: record.tokens.clone(),
            line: record.line,
            slash: record.slash,
        })
    }

    fn text(&mut self, field: &str) -> Result<Token, DiagnosticReport> {
        let record = self.record(field)?;
        if let Some(token) = record.tokens.first() {
            Ok(token.clone())
        } else {
            Err(self.record_error(&record, field, "expected a value"))
        }
    }

    fn count(&mut self, field: &str) -> Result<usize, DiagnosticReport> {
        let token = self.text(field)?;
        let count = token.text.parse::<usize>().map_err(|_| {
            one(
                "KR0102",
                format!("expected an integer, got {:?}", token.text),
                field,
                &self.path,
                token.line,
                token.column,
            )
        })?;
        if count == 0 || count > MAX_VECTOR_LENGTH {
            return Err(one(
                "KR0201",
                format!("count must be in 1..={MAX_VECTOR_LENGTH}"),
                field,
                &self.path,
                token.line,
                token.column,
            ));
        }
        Ok(count)
    }

    fn scalar(&mut self, field: &str) -> Result<f64, DiagnosticReport> {
        let token = self.text(field)?;
        number(&token, &self.path, field)
    }

    fn vector(&mut self, count: usize, field: &str) -> Result<Vec<f64>, DiagnosticReport> {
        let mut result = Vec::with_capacity(count);
        while result.len() < count {
            let record = self.record(field)?;
            for token in record.tokens.iter().take(count - result.len()) {
                result.push(number(token, &self.path, field)?);
            }
            if record.slash {
                break;
            }
            if record.tokens.is_empty() {
                return Err(self.record_error(&record, field, "expected numeric values"));
            }
        }
        Ok(result)
    }

    fn numbers(&mut self, field: &str, count: usize) -> Result<Vec<f64>, DiagnosticReport> {
        let record = self.record(field)?;
        if record.tokens.len() != count {
            return Err(self.record_error(
                &record,
                field,
                &format!("expected exactly {count} values"),
            ));
        }
        record
            .tokens
            .iter()
            .map(|token| number(token, &self.path, field))
            .collect()
    }

    fn record_error(&self, record: &Record, field: &str, message: &str) -> DiagnosticReport {
        one(
            "KR0102",
            message,
            field,
            &self.path,
            record.line,
            record.tokens.first().map_or(1, |token| token.column),
        )
    }

    fn finish(&self) -> Result<(), DiagnosticReport> {
        if let Some(record) = self.records.get(self.index) {
            return Err(self.record_error(record, "input", "unexpected trailing input"));
        }
        Ok(())
    }
}

fn tokenize(text: &str, line: usize, path: &Path) -> Result<(Vec<Token>, bool), DiagnosticReport> {
    let chars: Vec<char> = text.chars().collect();
    let mut tokens = Vec::new();
    let mut index = 0;
    let mut slash = false;
    let mut comma_pending = false;
    while index < chars.len() {
        let ch = chars[index];
        if ch.is_whitespace() {
            index += 1;
        } else if ch == ',' {
            if comma_pending || tokens.is_empty() {
                return Err(one(
                    "KR0202",
                    "Fortran null slots are not supported",
                    "input",
                    path,
                    line,
                    index + 1,
                ));
            }
            comma_pending = true;
            index += 1;
        } else if ch == '!' {
            break;
        } else if ch == '/' {
            slash = true;
            break;
        } else if ch == '\'' || ch == '"' {
            comma_pending = false;
            let quote = ch;
            let column = index + 1;
            index += 1;
            let mut value = String::new();
            let mut closed = false;
            while index < chars.len() {
                if chars[index] == quote {
                    if chars.get(index + 1) == Some(&quote) {
                        value.push(quote);
                        index += 2;
                    } else {
                        index += 1;
                        closed = true;
                        break;
                    }
                } else {
                    value.push(chars[index]);
                    index += 1;
                }
            }
            if !closed {
                return Err(one(
                    "KR0101",
                    "unterminated quoted string",
                    "input",
                    path,
                    line,
                    column,
                ));
            }
            tokens.push(Token {
                text: value,
                line,
                column,
            });
        } else {
            comma_pending = false;
            let start = index;
            while index < chars.len()
                && !chars[index].is_whitespace()
                && !matches!(chars[index], ',' | '/' | '!')
            {
                index += 1;
            }
            tokens.push(Token {
                text: chars[start..index].iter().collect(),
                line,
                column: start + 1,
            });
        }
    }
    Ok((tokens, slash))
}

fn number(token: &Token, path: &Path, field: &str) -> Result<f64, DiagnosticReport> {
    let value = token
        .text
        .replace('D', "E")
        .replace('d', "e")
        .parse::<f64>()
        .map_err(|_| {
            one(
                "KR0102",
                format!("expected a number, got {:?}", token.text),
                field,
                path,
                token.line,
                token.column,
            )
        })?;
    if !value.is_finite() {
        return Err(one(
            "KR0201",
            "value must be finite",
            field,
            path,
            token.line,
            token.column,
        ));
    }
    Ok(value)
}

enum FiniteMedium {
    Fluid(material::Fluid),
    Elastic(material::Elastic),
}

struct Environment {
    title: String,
    frequency_hz: f64,
    frequencies_hz: Vec<f64>,
    broadband: bool,
    interpolation: Interpolation,
    surface_boundary: SurfaceBoundary,
    bottom_boundary: BottomBoundary,
    materials: material::Materials,
    c_low: f64,
    c_high: f64,
    max_range_m: f64,
    source_depths: Vec<f64>,
    receiver_depths: Vec<f64>,
    locations: HashMap<String, (usize, usize)>,
}

#[cfg(test)]
fn parse_environment(source: &str, path: &Path) -> Result<Environment, DiagnosticReport> {
    parse_environment_with_solver(source, path, ModeSolver::Kraken)
}

#[allow(clippy::float_cmp, clippy::too_many_lines)]
fn parse_environment_with_solver(
    source: &str,
    path: &Path,
    mode_solver: ModeSolver,
) -> Result<Environment, DiagnosticReport> {
    let mut reader = Reader::new(source, path)?;
    let environment = read_environment(&mut reader, mode_solver)?;
    reader.finish()?;
    Ok(environment)
}

#[allow(clippy::float_cmp, clippy::too_many_lines, clippy::needless_borrow)]
fn read_environment(
    reader: &mut Reader,
    mode_solver: ModeSolver,
) -> Result<Environment, DiagnosticReport> {
    let path = reader.path.clone();
    let path = path.as_path();
    let title = reader.text("title")?.text;
    let frequency_hz = reader.scalar("frequency_hz")?;
    let medium_count = reader.count("medium_count")?;
    if medium_count > crate::solver::layers::MAX_LAYERS {
        return Err(reader_error(
            &reader,
            "KR0201",
            "at most 500 total finite media",
            "medium_count",
        ));
    }

    let options = reader.text("top_options")?;
    let option = |index| options.text.as_bytes().get(index).copied().unwrap_or(b' ');
    if !matches!(option(0), b'N' | b'C' | b'P' | b'S' | b'A')
        || !matches!(option(1), b'V' | b'R' | b'A' | b'F')
        || (option(1) == b'F' && (mode_solver != ModeSolver::Krakenc || option(4) != b' '))
        || !matches!(option(2), b'N' | b'W' | b'M' | b'm' | b'F' | b'Q' | b'L')
        || !matches!(option(3), b' ' | b'T' | b'F' | b'B')
        || (mode_solver == ModeSolver::Kraken && option(4) != b' ')
        || (mode_solver == ModeSolver::Krakenc && !matches!(option(4), b' ' | b'.'))
        || !matches!(option(5), b' ' | b'B')
        || options
            .text
            .as_bytes()
            .iter()
            .skip(6)
            .any(|byte| !byte.is_ascii_whitespace())
    {
        return Err(one(
            "KR0202",
            "requires N/C/P/S or fixed analytic A interpolation, smooth V/R/A or KRAKENC F surface, N/W/M/m/F/Q/L attenuation, optional T/F/B volume loss and B frequencies",
            "top_options",
            path,
            options.line,
            options.column,
        ));
    }

    if medium_count > 1 && (option(0) == b'A' || option(1) == b'F') {
        return Err(reader_error(
            &reader,
            "KR0202",
            "analytic Munk and top TRC remain single-layer",
            "top_options",
        ));
    }
    let interpolation = match option(0) {
        b'N' => Interpolation::N2Linear,
        b'C' => Interpolation::CLinear,
        b'P' => Interpolation::Pchip,
        b'S' => Interpolation::Spline,
        _ => Interpolation::AnalyticMunk,
    };
    let volume_loss = match option(3) {
        b'T' => VolumeLoss::Thorp,
        b'F' => {
            let p = reader.numbers("volume_attenuation", 4)?;
            if p[0] <= -273.0 || p[1] < 0.0 || !(0.0..=14.0).contains(&p[2]) || p[3] < 0.0 {
                return Err(reader_error(
                    &reader,
                    "KR0201",
                    "require T > -273, salinity >= 0, 0 <= pH <= 14 and mean depth >= 0",
                    "volume_attenuation",
                ));
            }
            VolumeLoss::FrancoisGarrison([p[0], p[1], p[2], p[3]])
        }
        b'B' => {
            let count = reader.count("bio_layers.count")?;
            if count > 200 {
                return Err(reader_error(
                    &reader,
                    "KR0201",
                    "at most 200 biological layers",
                    "bio_layers.count",
                ));
            }
            let mut layers = Vec::with_capacity(count);
            for _ in 0..count {
                let p = reader.numbers("bio_layers", 5)?;
                if p[0] < 0.0 || p[1] < p[0] || p[2] <= 0.0 || p[3] <= 0.0 || p[4] < 0.0 {
                    return Err(reader_error(
                        &reader,
                        "KR0201",
                        "require 0 <= top <= bottom, positive resonance/Q and nonnegative a0",
                        "bio_layers",
                    ));
                }
                layers.push([p[0], p[1], p[2], p[3], p[4]]);
            }
            VolumeLoss::Biological(layers)
        }
        _ => VolumeLoss::None,
    };
    if interpolation == Interpolation::AnalyticMunk && !matches!(volume_loss, VolumeLoss::None) {
        return Err(reader_error(
            &reader,
            "KR0202",
            "analytic Munk water remains lossless; volume attenuation is not supported with A",
            "top_options",
        ));
    }
    let surface = if option(1) == b'A' {
        if option(2) == b'm' {
            return Err(reader_error(
                &reader,
                "KR0202",
                "top A with m loss has no defined reference power-law parameters",
                "top_options",
            ));
        }
        read_half_space(
            reader,
            "surface_half_space",
            0.0,
            [0.0, 1500.0, 0.0, 1.0, 0.0, 0.0],
            1,
        )?
    } else {
        [0.0; 6]
    };
    let defaults = if option(1) == b'A' {
        surface
    } else {
        [0.0, 1500.0, 0.0, 1.0, 0.0, 0.0]
    };
    let mut water = None;
    let mut inherited = defaults;
    let mut additional_layers = Vec::new();
    let mut top_elastic_layers = Vec::new();
    let mut bottom_elastic_layers = Vec::new();
    let mut top = 0.0;
    let mut profile_points = 0;
    for index in 0..medium_count {
        let (medium, last, point_count) = read_finite_layer(
            reader,
            interpolation,
            option(2),
            frequency_hz,
            index,
            top,
            inherited,
        )?;
        inherited = last;
        profile_points += point_count;
        if profile_points > MAX_PROFILE_POINTS {
            return Err(reader_error(
                &reader,
                "KR0201",
                "total finite profile points exceed the limit",
                "medium_count",
            ));
        }
        let (old_header, old_profile) = if index == 0 {
            ("water_header".to_owned(), "sound_speed_profile".to_owned())
        } else {
            (
                format!("additional_fluid_layers[{}].header", index - 1),
                format!("additional_fluid_layers[{}].sound_speed_profile", index - 1),
            )
        };
        let (header, profile) = match medium {
            FiniteMedium::Fluid(layer) => {
                if !bottom_elastic_layers.is_empty() {
                    return Err(reader_error(
                        &reader,
                        "KR0202",
                        "finite fluids must be contiguous; solids between fluids are unsupported",
                        &old_profile,
                    ));
                }
                top = layer.bottom_depth_m;
                if water.is_none() {
                    water = Some(layer);
                    ("water_header".to_owned(), "sound_speed_profile".to_owned())
                } else {
                    let prefix = format!("additional_fluid_layers[{}]", additional_layers.len());
                    additional_layers.push(layer);
                    (
                        format!("{prefix}.header"),
                        format!("{prefix}.sound_speed_profile"),
                    )
                }
            }
            FiniteMedium::Elastic(layer) => {
                top = layer.bottom_depth_m;
                let prefix = if water.is_none() {
                    let prefix = format!("top_elastic_layers[{}]", top_elastic_layers.len());
                    top_elastic_layers.push(layer);
                    prefix
                } else {
                    let prefix = format!("bottom_elastic_layers[{}]", bottom_elastic_layers.len());
                    bottom_elastic_layers.push(layer);
                    prefix
                };
                (
                    format!("{prefix}.header"),
                    format!("{prefix}.sound_speed_profile"),
                )
            }
        };
        let header_location = reader.locations[&old_header];
        let profile_location = reader.locations.get(&old_profile).copied();
        reader.locations.insert(header, header_location);
        if let Some(location) = profile_location {
            reader.locations.insert(profile, location);
        }
    }
    let water = water.ok_or_else(|| {
        reader_error(
            &reader,
            "KR0202",
            "requires at least one finite fluid layer",
            "medium_count",
        )
    })?;
    let bottom_option = reader.record("bottom_options")?;
    if bottom_option.tokens.len() != if option(2) == b'm' { 4 } else { 2 }
        || !matches!(
            bottom_option.tokens[0].text.as_str(),
            "V" | "A" | "R" | "F" | "P"
        )
        || (mode_solver == ModeSolver::Kraken
            && matches!(bottom_option.tokens[0].text.as_str(), "F" | "P"))
        || number(&bottom_option.tokens[1], path, "bottom_roughness")? != 0.0
    {
        return Err(reader.record_error(
            &bottom_option,
            "bottom_options",
            "requires smooth V/A/R bottom, or KRAKENC F/P table bottom",
        ));
    }
    if medium_count > 1 && matches!(bottom_option.tokens[0].text.as_str(), "F" | "P") {
        return Err(reader.record_error(
            &bottom_option,
            "bottom_options",
            "tabulated bottoms remain single-layer",
        ));
    }
    if matches!(bottom_option.tokens[0].text.as_str(), "F" | "P") && option(4) == b'.' {
        return Err(reader.record_error(
            &bottom_option,
            "bottom_options",
            "tabulated bottoms require blank restart option (no random restarts)",
        ));
    }
    let bottom_power_law = read_power_law(&reader, &bottom_option, 2, option(2), frequency_hz)?;
    let mut bottom_boundary = match bottom_option.tokens[0].text.as_str() {
        "V" => BottomBoundary::Vacuum,
        "R" => BottomBoundary::Rigid,
        "F" => BottomBoundary::Reflection(Vec::new()),
        "P" => BottomBoundary::Impedance {
            frequency_hz,
            points: Vec::new(),
        },
        _ => BottomBoundary::FluidHalfSpace,
    };
    let mut bottom = [0.0; 6];
    if bottom_boundary == BottomBoundary::FluidHalfSpace {
        let defaults = if interpolation == Interpolation::AnalyticMunk {
            [top, 1500.0, 0.0, 1.0, 0.0, 0.0]
        } else {
            inherited
        };
        let minimum = if interpolation == Interpolation::AnalyticMunk {
            4
        } else {
            1
        };
        bottom = read_half_space(reader, "bottom_half_space", top, defaults, minimum)?;
        if bottom[2] > 0.0 {
            bottom_boundary = crate::Boundary::ElasticHalfSpace {
                shear_sound_speed_mps: bottom[2],
                shear_attenuation_db_per_wavelength: 0.0,
            };
        }
    }
    let limits = reader.numbers("phase_speed_limits", 2)?;
    let max_range_m = reader.scalar("max_range_km")? * 1000.0;
    let source_depths = read_vector(reader, "mode_source_depths_m", true)?;
    let receiver_depths = read_vector(reader, "mode_receiver_depths_m", true)?;
    let broadband = option(5) == b'B';
    let frequencies_hz = if broadband {
        let count = reader.count("frequencies_hz.count")?;
        if count > MAX_FREQUENCIES {
            return Err(reader_error(
                &reader,
                "KR0201",
                "at most 1000 frequencies are supported",
                "frequencies_hz.count",
            ));
        }
        // Unlike ReadVector, ReadfreqVec/SubTab preserves frequency order.
        let frequencies = read_vector_values(reader, "frequencies_hz", count, false)?;
        if frequencies.iter().any(|&f| f <= 0.0) {
            return Err(reader_error(
                &reader,
                "KR0201",
                "frequencies must be positive",
                "frequencies_hz",
            ));
        }
        frequencies
    } else {
        vec![frequency_hz]
    };
    Ok(Environment {
        title,
        frequency_hz,
        frequencies_hz,
        broadband,
        interpolation,
        surface_boundary: match option(1) {
            b'V' => SurfaceBoundary::Vacuum,
            b'R' => SurfaceBoundary::Rigid,
            b'F' => SurfaceBoundary::Reflection(Vec::new()),
            _ if surface[2] > 0.0 => SurfaceBoundary::ElasticHalfSpace {
                shear_sound_speed_mps: surface[2],
                shear_attenuation_db_per_wavelength: 0.0,
            },
            _ => SurfaceBoundary::FluidHalfSpace,
        },
        bottom_boundary,
        materials: material::Materials {
            surface_half_space: material::HalfSpace {
                values: surface,
                power_law: water.power_law,
            },
            bottom_half_space: material::HalfSpace {
                values: bottom,
                power_law: bottom_power_law,
            },
            water,
            additional: additional_layers,
            top: top_elastic_layers,
            bottom: bottom_elastic_layers,
            unit: option(2),
            volume: volume_loss,
        },
        c_low: limits[0],
        c_high: limits[1],
        max_range_m,
        source_depths,
        receiver_depths,
        locations: std::mem::take(&mut reader.locations),
    })
}

#[allow(
    clippy::too_many_arguments,
    clippy::too_many_lines,
    clippy::float_cmp,
    clippy::type_complexity
)]
fn read_finite_layer(
    reader: &mut Reader,
    interpolation: Interpolation,
    unit: u8,
    frequency: f64,
    index: usize,
    top: f64,
    mut inherited: [f64; 6],
) -> Result<(FiniteMedium, [f64; 6], usize), DiagnosticReport> {
    let header_field = if index == 0 {
        "water_header".into()
    } else {
        format!("additional_fluid_layers[{}].header", index - 1)
    };
    let profile_field = if index == 0 {
        "sound_speed_profile".into()
    } else {
        format!("additional_fluid_layers[{}].sound_speed_profile", index - 1)
    };
    let header = reader.record(&header_field)?;
    if header.tokens.len() != if unit == b'm' { 5 } else { 3 } {
        return Err(reader.record_error(
            &header,
            &header_field,
            "expected mesh count, roughness, depth (plus beta and transition frequency for m)",
        ));
    }
    let mesh_points = header.tokens[0].text.parse::<usize>().map_err(|_| {
        reader.record_error(&header, &header_field, "mesh count must be an integer")
    })?;
    let bottom = number(&header.tokens[2], &reader.path, &header_field)?;
    if bottom <= top {
        return Err(reader.record_error(
            &header,
            &header_field,
            "layer interfaces must strictly increase",
        ));
    }
    if number(&header.tokens[1], &reader.path, &header_field)? != 0.0 {
        return Err(reader.record_error(
            &header,
            if index == 0 {
                "surface_roughness"
            } else {
                &header_field
            },
            "requires smooth interfaces",
        ));
    }
    let power = read_power_law(reader, &header, 3, unit, frequency)?;
    let mut points: Vec<[f64; 6]> = Vec::new();
    if interpolation != Interpolation::AnalyticMunk {
        loop {
            let record = reader.record(&profile_field)?;
            if !(1..=6).contains(&record.tokens.len()) || (record.tokens.len() < 6 && !record.slash)
            {
                return Err(reader.record_error(
                    &record,
                    &profile_field,
                    "expected 6 values, or 1..=5 followed by / to inherit trailing values",
                ));
            }
            let mut point = inherited;
            for (i, token) in record.tokens.iter().enumerate() {
                point[i] = number(token, &reader.path, &profile_field)?;
            }
            if point[0] < top
                || point[0] > bottom
                || points.last().is_some_and(|p| point[0] <= p[0])
            {
                return Err(reader.record_error(
                    &record,
                    &profile_field,
                    "depths must increase within each finite layer",
                ));
            }
            if (points.is_empty() && point[0] != top)
                || point[2] < 0.0
                || point[4] < 0.0
                || point[5] < 0.0
                || (point[2] == 0.0 && point[5] != 0.0)
                || points.first().is_some_and(|p| {
                    (p[2] == 0.0 && (point[3] != p[3] || point[2] != 0.0))
                        || (p[2] > 0.0 && point[2] == 0.0)
                })
            {
                return Err(reader.record_error(&record, &profile_field, "requires constant-density fluid (no shear loss), or positive elastic shear speed throughout the layer; nonnegative absorption"));
            }
            inherited = point;
            points.push(point);
            if points.len() > MAX_PROFILE_POINTS {
                return Err(reader.record_error(
                    &record,
                    &profile_field,
                    "too many profile points",
                ));
            }
            if point[0] == bottom {
                break;
            }
        }
        if points.len() < 2 {
            return Err(reader_error(
                reader,
                "KR0202",
                "a finite profile needs top and interface points",
                &profile_field,
            ));
        }
    }
    if points.first().is_some_and(|p| p[2] > 0.0) {
        let point_count = points.len();
        return Ok((
            FiniteMedium::Elastic(material::Elastic {
                bottom_depth_m: bottom,
                points,
                mesh_points,
                power_law: power,
            }),
            inherited,
            point_count,
        ));
    }
    Ok((
        FiniteMedium::Fluid(material::Fluid {
            bottom_depth_m: bottom,
            density_g_cm3: points.first().map_or(1.0, |p| p[3]),
            mesh_points,
            points: points
                .iter()
                .map(|p| SoundSpeedPoint {
                    depth_m: p[0],
                    sound_speed_mps: p[1],
                })
                .collect(),
            attenuation: points.iter().map(|p| p[4]).collect(),
            power_law: power,
        }),
        inherited,
        points.len(),
    ))
}

#[allow(clippy::float_cmp)]
fn read_half_space(
    reader: &mut Reader,
    field: &str,
    depth: f64,
    mut values: [f64; 6],
    minimum: usize,
) -> Result<[f64; 6], DiagnosticReport> {
    let record = reader.record(field)?;
    if !(minimum..=6).contains(&record.tokens.len()) || (record.tokens.len() < 6 && !record.slash) {
        return Err(reader.record_error(&record, field, "expected half-space values, with explicit speed/density for analytic water; trailing defaults require /"));
    }
    for (value, token) in values.iter_mut().zip(&record.tokens) {
        *value = number(token, &reader.path, field)?;
    }
    if values[0] != depth
        || values[1] <= 0.0
        || values[2] < 0.0
        || values[3] <= 0.0
        || values[4] < 0.0
        || values[5] < 0.0
        || (values[2] == 0.0 && values[5] != 0.0)
    {
        return Err(reader.record_error(&record, field, "require half-space at the boundary, positive compressional speed/density, nonnegative shear speed/loss and no shear loss for a fluid"));
    }
    Ok(values)
}

fn read_power_law(
    reader: &Reader,
    record: &Record,
    start: usize,
    unit: u8,
    reference: f64,
) -> Result<(f64, f64, f64), DiagnosticReport> {
    if unit != b'm' {
        return Ok((reference, 1.0, reference));
    }
    let beta = number(&record.tokens[start], &reader.path, "attenuation_power_law")?;
    let transition = number(
        &record.tokens[start + 1],
        &reader.path,
        "attenuation_power_law",
    )?;
    if beta < 0.0 || transition <= 0.0 {
        return Err(reader.record_error(
            record,
            "attenuation_power_law",
            "require beta >= 0 and transition frequency > 0",
        ));
    }
    Ok((reference, beta, transition))
}

#[allow(clippy::cast_possible_truncation, clippy::cast_precision_loss)]
fn read_vector(
    reader: &mut Reader,
    field: &str,
    single_precision: bool,
) -> Result<Vec<f64>, DiagnosticReport> {
    let count = reader.count(&format!("{field}.count"))?;
    let mut values = read_vector_values(reader, field, count, single_precision)?;
    // The reference ReadVector sorts each vector, including receiver offsets.
    values.sort_by(f64::total_cmp);
    Ok(values)
}

#[allow(clippy::cast_possible_truncation, clippy::cast_precision_loss)]
fn read_vector_values(
    reader: &mut Reader,
    field: &str,
    count: usize,
    single_precision: bool,
) -> Result<Vec<f64>, DiagnosticReport> {
    let mut values = reader.vector(count, field)?;
    if single_precision {
        for value in &mut values {
            *value = f64::from(*value as f32);
        }
    }
    if values.len() < count {
        if count < 3 || !(1..=2).contains(&values.len()) {
            return Err(reader_error(
                reader,
                "KR0102",
                "incomplete vector: supply all values or 1..=2 endpoints for count >= 3",
                field,
            ));
        }
        let start = values[0];
        let end = *values.last().unwrap();
        values = if single_precision {
            let step = (end as f32 - start as f32) / (count - 1) as f32;
            (0..count)
                .map(|i| f64::from(start as f32 + i as f32 * step))
                .collect()
        } else {
            let step = (end - start) / (count - 1) as f64;
            (0..count).map(|i| start + i as f64 * step).collect()
        };
    }
    if values.iter().any(|value| !value.is_finite()) {
        return Err(reader_error(
            reader,
            "KR0201",
            "vector exceeds the numeric range",
            field,
        ));
    }
    Ok(values)
}

fn parse_reflection_table(
    source: &str,
    path: &Path,
    field: &str,
) -> Result<BottomBoundary, DiagnosticReport> {
    let mut reader = Reader::new(source, path)?;
    let count = reader.count(&format!("{field}.count"))?;
    let mut points = Vec::with_capacity(count);
    for _ in 0..count {
        let values = reader.numbers(field, 3)?;
        points.push(crate::ReflectionPoint {
            angle_degrees: values[0],
            magnitude: values[1],
            phase_radians: values[2].to_radians(),
        });
    }
    reader.finish()?;
    Ok(BottomBoundary::Reflection(points))
}

fn parse_impedance_table(source: &str, path: &Path) -> Result<BottomBoundary, DiagnosticReport> {
    let mut lines = source.lines();
    let header = lines.next().unwrap_or("");
    let mut reader = Reader::new(header, path)?;
    let record = reader.record("bottom_boundary.frequency_hz")?;
    if record.tokens.len() != 2 || !matches!(header.trim_start().chars().next(), Some('\'' | '"')) {
        return Err(reader.record_error(
            &record,
            "bottom_boundary",
            "IRC header requires quoted title and frequency",
        ));
    }
    let frequency_hz = number(&record.tokens[1], path, "bottom_boundary.frequency_hz")?;
    let count_line = lines.next().unwrap_or("");
    let count = count_line
        .trim()
        .parse::<usize>()
        .ok()
        .filter(|n| (2..=MAX_VECTOR_LENGTH).contains(n))
        .ok_or_else(|| {
            one(
                "KR0102",
                "IRC requires 2..=100000 rows",
                "bottom_boundary",
                path,
                2,
                1,
            )
        })?;
    let mut points = Vec::with_capacity(count);
    for index in 0..count {
        let line = lines.next().unwrap_or("");
        let fail = || {
            one(
                "KR0102",
                "expected IRC fixed-width (5G15.7,I5) row",
                "bottom_boundary",
                path,
                index + 3,
                1,
            )
        };
        if !line.is_ascii() || line.len() < 80 || !line[80..].trim().is_empty() {
            return Err(fail());
        }
        let mut values = [0.0; 5];
        for (i, value) in values.iter_mut().enumerate() {
            let mut text = line[i * 15..(i + 1) * 15].trim().to_owned();
            // G15.7 omits E when a three-digit exponent fills the field.
            if !text.contains(['e', 'E', 'd', 'D'])
                && let Some((index, _)) = text
                    .char_indices()
                    .skip(1)
                    .find(|(_, ch)| matches!(ch, '+' | '-'))
            {
                text.insert(index, 'E');
            }
            let token = Token {
                text,
                line: index + 3,
                column: i * 15 + 1,
            };
            *value = number(&token, path, "bottom_boundary")?;
        }
        let power = line[75..80].trim().parse::<i32>().map_err(|_| fail())?;
        points.push(crate::ImpedancePoint {
            wavenumber_squared: values[0],
            f: num_complex::Complex64::new(values[1], values[2]),
            g: num_complex::Complex64::new(values[3], values[4]),
            power,
        });
    }
    if lines.any(|line| !line.trim().is_empty()) {
        return Err(one(
            "KR0102",
            "unexpected trailing IRC data",
            "bottom_boundary",
            path,
            count + 3,
            1,
        ));
    }
    Ok(BottomBoundary::Impedance {
        frequency_hz,
        points,
    })
}

#[derive(Clone)]
struct Field {
    profile_ranges_m: Vec<f64>,
    propagation: crate::FieldPropagation,
    mode_limit: usize,
    source_geometry: SourceGeometry,
    mode_addition: ModeAddition,
    source_pattern_requested: bool,
    source_pattern: Vec<SourcePatternPoint>,
    source_depths: Vec<f64>,
    receiver_depths: Vec<f64>,
    receiver_ranges_m: Vec<f64>,
    receiver_offsets_m: Vec<f64>,
    locations: HashMap<String, (usize, usize)>,
}

fn parse_source_pattern(
    source: &str,
    path: &Path,
) -> Result<Vec<SourcePatternPoint>, DiagnosticReport> {
    let mut reader = Reader::new(source, path)?;
    let count = reader.count("source_pattern.count")?;
    if count < 2 {
        return Err(reader_error(
            &reader,
            "KR0201",
            "source pattern requires at least two points",
            "source_pattern.count",
        ));
    }
    let mut points = Vec::with_capacity(count);
    for _ in 0..count {
        let values = reader.numbers("source_pattern", 2)?;
        let amplitude = 10.0_f64.powf(values[1] / 20.0);
        if !amplitude.is_finite()
            || points
                .last()
                .is_some_and(|point: &SourcePatternPoint| values[0] <= point.angle_degrees)
        {
            return Err(reader_error(
                &reader,
                "KR0201",
                "source pattern requires increasing finite angles and finite dB levels",
                "source_pattern",
            ));
        }
        points.push(SourcePatternPoint {
            angle_degrees: values[0],
            amplitude,
        });
    }
    reader.finish()?;
    Ok(points)
}

fn parse_field(source: &str, path: &Path) -> Result<Field, DiagnosticReport> {
    let mut reader = Reader::new(source, path)?;
    // A slash-only title means use the environment title in FIELD.
    reader.record("field_title")?;
    let options = reader.text("field_options")?;
    let chars: Vec<char> = options.text.chars().collect();
    let option = |index| chars.get(index).copied().unwrap_or(' ');
    if !matches!(option(0), 'X' | 'R' | 'S')
        || !matches!(option(1), ' ' | 'A' | 'C')
        || !matches!(option(2), ' ' | 'O' | '*')
        || !matches!(option(3), ' ' | 'C' | 'I')
        || chars.iter().skip(4).any(|ch| !ch.is_whitespace())
    {
        return Err(one(
            "KR0202",
            "requires line, point or scaled-cylindrical geometry; omni or tabulated pattern; and coherent or incoherent addition",
            "field_options",
            path,
            options.line,
            options.column,
        ));
    }
    let mode_limit = reader.count("mode_limit")?;
    let profiles = reader.count("profile_count")?;
    let mut profile_ranges = read_vector_values(&mut reader, "profile_ranges_km", profiles, false)?;
    profile_ranges.sort_by(f64::total_cmp);
    if profile_ranges.len() != profiles
        || profile_ranges[0] != 0.0
        || profile_ranges.windows(2).any(|pair| pair[1] <= pair[0])
        || (profiles > 1 && !matches!(option(1), 'A' | 'C'))
        || (profiles > 1 && option(1) == 'C' && option(3) == 'I')
    {
        return Err(reader_error(
            &reader,
            "KR0202",
            "requires increasing profile ranges starting at 0 km, adiabatic or coupled propagation; coupled modes cannot use incoherent addition",
            "profile_ranges_km",
        ));
    }
    let ranges_km = read_vector(&mut reader, "receiver_ranges_km", false)?;
    let source_depths = read_vector(&mut reader, "field_source_depths_m", true)?;
    let receiver_depths = read_vector(&mut reader, "field_receiver_depths_m", true)?;
    let receiver_offsets = read_vector(&mut reader, "receiver_offsets_m", false)?;
    reader.finish()?;

    Ok(Field {
        profile_ranges_m: profile_ranges
            .into_iter()
            .map(|range| range * 1000.0)
            .collect(),
        propagation: if profiles == 1 {
            crate::FieldPropagation::RangeIndependent
        } else if option(1) == 'C' {
            crate::FieldPropagation::Coupled
        } else {
            crate::FieldPropagation::Adiabatic
        },
        mode_limit,
        source_geometry: match option(0) {
            'X' => SourceGeometry::Line,
            'R' => SourceGeometry::Point,
            _ => SourceGeometry::ScaledCylindrical,
        },
        mode_addition: if option(3) == 'I' {
            ModeAddition::Incoherent
        } else {
            ModeAddition::Coherent
        },
        source_pattern_requested: option(2) == '*',
        source_pattern: Vec::new(),
        source_depths,
        receiver_depths,
        receiver_ranges_m: ranges_km.into_iter().map(|range| range * 1000.0).collect(),
        receiver_offsets_m: receiver_offsets,
        locations: reader.locations,
    })
}

fn reader_error(
    reader: &Reader,
    code: &'static str,
    message: &str,
    field: &str,
) -> DiagnosticReport {
    one(code, message, field, &reader.path, reader.last_line, 1)
}

fn one(
    code: &'static str,
    message: impl Into<String>,
    field: impl Into<String>,
    path: impl Into<PathBuf>,
    line: usize,
    column: usize,
) -> DiagnosticReport {
    DiagnosticReport::one(Diagnostic::new(code, message, field, path, line, column))
}

#[cfg(test)]
mod tests {
    use super::{parse_case, parse_environment, parse_field, parse_frequency_cases, read_file};

    #[test]
    fn irc_fixed_width_accepts_d_and_letterless_exponents_without_splitting_fields() {
        let row = |x: &str| {
            format!(
                "{x:>15}{:>15}{:>15}{:>15}{:>15}{:>5}\n",
                "-.1234567-100", "0.0", "1.0D+00", "0.0", 0
            )
        };
        let source = format!("'fixed width' 50.0\n2\n{}{}", row("1.0"), row("2.0"));
        let crate::BottomBoundary::Impedance { points, .. } =
            super::parse_impedance_table(&source, std::path::Path::new("fixed.irc")).unwrap()
        else {
            panic!()
        };
        assert_eq!(points[0].f.re.to_bits(), (-0.123_456_7e-100_f64).to_bits());
        assert_eq!(points[1].g.re.to_bits(), 1.0_f64.to_bits());
        let invalid = source.replace("1.0D+00", "NaN");
        assert!(super::parse_impedance_table(&invalid, std::path::Path::new("fixed.irc")).is_err());
    }
    use std::path::Path;

    #[test]
    fn frequency_source_snapshots_match_files_and_enforce_size_limits() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/PekerisBroadband");
        let env = root.with_extension("env");
        let flp = root.with_extension("flp");
        let env_source = read_file(&env).unwrap();
        let flp_source = read_file(&flp).unwrap();
        let parse = |source: &str| {
            super::load_frequency_cases_from_sources(
                source,
                &flp_source,
                Path::new("snapshot.env"),
                Path::new("snapshot.flp"),
                crate::ModeSolver::Kraken,
            )
        };
        assert_eq!(
            parse(&env_source).unwrap(),
            super::load_frequency_cases(&env, &flp, crate::ModeSolver::Kraken).unwrap()
        );
        let report =
            parse(&" ".repeat(usize::try_from(super::MAX_INPUT_BYTES + 1).unwrap())).unwrap_err();
        assert_eq!(report.diagnostics()[0].path, Path::new("snapshot.env"));
        assert!(report.diagnostics()[0].message.contains("1 MiB"));
        let report = super::load_frequency_cases_from_sources(
            &env_source,
            &" ".repeat(usize::try_from(super::MAX_INPUT_BYTES + 1).unwrap()),
            &env,
            &flp,
            crate::ModeSolver::Kraken,
        )
        .unwrap_err();
        assert_eq!(report.diagnostics()[0].path, flp);
    }

    #[test]
    #[allow(clippy::float_cmp)]
    fn broadband_preserves_order_and_scales_before_mesh_rounding() {
        let env = include_str!("../../../tests/fixtures/PekerisBroadband.env");
        let flp = include_str!("../../../tests/fixtures/PekerisBroadband.flp");
        let parse = |source: &str| {
            parse_frequency_cases(
                source,
                flp,
                Path::new("case.env"),
                Path::new("case.flp"),
                crate::ModeSolver::Kraken,
                false,
            )
            .unwrap()
        };
        let cases = parse(env);
        assert_eq!(
            cases.iter().map(|c| c.frequency_hz).collect::<Vec<_>>(),
            [75.0, 50.0, 62.5]
        );
        assert_eq!(cases[0].mesh_reference_frequency_hz, Some(50.0));
        for (case, expected) in
            cases
                .iter()
                .zip([[151, 303, 606], [101, 202, 404], [126, 252, 505]])
        {
            for (multiplier, n) in [1, 2, 4].into_iter().zip(expected) {
                assert_eq!(case.mesh_points_at(multiplier).unwrap(), n);
            }
        }
        // Resolve NG=0 at freq0 (66), not at the current frequency (100).
        let automatic = parse(&env.replace("101 0.0", "0 0.0"));
        assert_eq!(automatic[0].mesh_points_at(1).unwrap(), 99);
        for frequency in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            let mut input = cases[0].clone().into_definition();
            input.mesh_reference_frequency_hz = Some(frequency);
            let report = crate::Case::from_definition(input).unwrap_err();
            assert_eq!(report.diagnostics()[0].field, "mesh_reference_frequency_hz");
        }
        let ordered = parse(&env.replace("75.0 50.0 62.5 /", "100.0 50.0 /"));
        assert_eq!(
            ordered.iter().map(|c| c.frequency_hz).collect::<Vec<_>>(),
            [100.0, 75.0, 50.0]
        );
        assert_eq!(
            parse(&env.replace("75.0 50.0 62.5 /", "75.0 75.0 50.0 /"))[1].frequency_hz,
            75.0
        );
        let report =
            parse_case(env, flp, Path::new("case.env"), Path::new("case.flp")).unwrap_err();
        assert_eq!(report.diagnostics()[0].field, "frequencies_hz");
        assert_eq!(report.diagnostics()[0].line, 17);
    }

    #[test]
    fn broadband_rejects_bad_frequencies_and_bounded_storage() {
        let env = include_str!("../../../tests/fixtures/PekerisBroadband.env");
        let flp = include_str!("../../../tests/fixtures/PekerisBroadband.flp");
        for source in [
            env.replace("75.0 50.0 62.5 /", "0.0 50.0 62.5 /"),
            env.replace("75.0 50.0 62.5 /", "NaN 50.0 62.5 /"),
            env.replace("75.0 50.0 62.5 /", "-75.0 50.0 62.5 /"),
            env.replace("3\n75.0", "1001\n75.0"),
            env.replace("75.0 50.0 62.5 /", "75.0 50.0\n"),
            env.replace("50.0\n1\n", "0.0\n1\n"),
            env.replace("75.0 50.0 62.5 /", "0.01 50.0 62.5 /"),
        ] {
            assert!(
                parse_frequency_cases(
                    &source,
                    flp,
                    Path::new("case.env"),
                    Path::new("case.flp"),
                    crate::ModeSolver::Kraken,
                    false
                )
                .is_err(),
                "accepted {source}"
            );
        }
        let source = env.replace("3\n75.0 50.0 62.5 /", "100\n50.0 100.0 /");
        let field = flp.replace("3\n0.5 1.0 2.0 /", "100000\n0.5 2.0 /");
        let report = parse_frequency_cases(
            &source,
            &field,
            Path::new("case.env"),
            Path::new("case.flp"),
            crate::ModeSolver::Kraken,
            false,
        )
        .unwrap_err();
        assert!(
            report.diagnostics()[0]
                .message
                .contains("input storage limit")
        );
    }

    #[test]
    fn oversized_files_are_rejected_before_parsing() {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path =
            std::env::temp_dir().join(format!("kraken-input-{}-{stamp}.env", std::process::id()));
        let file = std::fs::File::create_new(&path).unwrap();
        file.set_len(1_048_577).unwrap();
        drop(file);
        let result = read_file(&path);
        std::fs::remove_file(&path).unwrap();
        assert_eq!(result.unwrap_err().diagnostics()[0].code, "KR0201");
    }

    #[test]
    fn subtabulated_depths_use_reference_precision_and_keep_fractional_boundary_valid() {
        let flp = include_str!("../../../tests/fixtures/Pekeris.flp")
            .replace("3\n25.0 75.0 99.0 /", "10\n0.0 100.0 /")
            .replace("3\n0.0 0.0 0.0 /", "10\n0.0 /");
        let field = parse_field(&flp, Path::new("case.flp")).unwrap();
        let expected: Vec<_> = (0..10)
            .map(|i| f64::from(f32::from(u16::try_from(i).unwrap()) * (100.0_f32 / 9.0)))
            .collect();
        assert_eq!(field.receiver_depths, expected);
        assert_eq!(field.receiver_offsets_m, vec![0.0; 10]);

        let env = "'fractional boundary'\n50\n1\n'NVN'\n100 0 0.1\n0 1500 /\n0.1 1500 /\n'A' 0\n0.1 1700 /\n1400 1700\n0\n1\n0.1 /\n2\n0 0.1 /\n";
        let flp = "/\n'X OC'\n9999\n1\n0 /\n1\n1 /\n1\n0.1 /\n2\n0 0.1 /\n2\n0 0 /\n";
        let case = parse_case(env, flp, Path::new("case.env"), Path::new("case.flp")).unwrap();
        assert_eq!(case.mode_sample_depths_m, [0.0, 0.1]);
        assert_eq!(case.source_depths_m, [0.1]);
    }

    #[test]
    fn semantic_errors_retain_file_locations() {
        let env = include_str!("../../../tests/fixtures/Pekeris.env").replace("50.0\n", "-50.0\n");
        let flp = include_str!("../../../tests/fixtures/Pekeris.flp").replace("75.0 /", "175.0 /");
        let report =
            parse_case(&env, &flp, Path::new("case.env"), Path::new("case.flp")).unwrap_err();
        for (field, path, line) in [
            ("frequency_hz", "case.env", 2),
            ("source_depths_m", "case.flp", 9),
        ] {
            let diagnostic = report
                .diagnostics()
                .iter()
                .find(|d| d.field == field)
                .unwrap();
            assert_eq!(diagnostic.path, Path::new(path));
            assert_eq!(diagnostic.line, line);
        }
    }

    #[test]
    fn legacy_vectors_follow_reference_sorting_and_reject_unsupported_syntax() {
        let flp = include_str!("../../../tests/fixtures/Pekeris.flp");
        let source = flp
            .replace("0.5 1.0 2.0 /", "2.0D0, 0.5d0, 1.0 /")
            .replace("0.0 0.0 0.0 /", "10.0 -10.0 0.0 /");
        let field = parse_field(&source, Path::new("case.flp")).unwrap();
        assert_eq!(field.receiver_ranges_m, [500.0, 1000.0, 2000.0]);
        assert_eq!(field.receiver_offsets_m, [-10.0, 0.0, 10.0]);
        for vector in ["3*0.5 /", ",0.5,1.0 /", "0.5,,1.0 /", "0.5 NaN 2.0 /"] {
            let source = flp.replace("0.5 1.0 2.0 /", vector);
            assert!(
                parse_field(&source, Path::new("case.flp")).is_err(),
                "accepted {vector}"
            );
        }
    }

    #[test]
    fn ssp_ends_at_interface_not_slash() {
        let env = include_str!("../../../tests/fixtures/Pekeris.env");
        for source in [
            env.replace(
                "0.0 1500.0 0.0 1.0 0.0 0.0\n",
                "0.0 1500.0 0.0 1.0 0.0 0.0 /\n",
            ),
            env.replace("100.0 1500.0 /", "100.0 1500.0 0.0 1.0 0.0 0.0"),
        ] {
            assert!(parse_environment(&source, Path::new("Pekeris.env")).is_ok());
        }
    }

    #[test]
    fn invalid_ssp_numbers_and_ambiguous_records_are_rejected() {
        let env = include_str!("../../../tests/fixtures/Pekeris.env");
        for point in [
            "NaN 1500.0 0.0 1.0 0.0 0.0",
            "inf 1500.0 0.0 1.0 0.0 0.0",
            "50.0,,1500.0,0.0,1.0,0.0,0.0",
            "50.0 1500.0", // Missing values need an explicit slash, not implicit inheritance.
        ] {
            let source = env.replace("100.0 1500.0 /", &format!("{point}\n100.0 1500.0 /"));
            assert!(
                parse_environment(&source, Path::new("Pekeris.env")).is_err(),
                "accepted {point}"
            );
        }
    }

    #[test]
    fn original_sduct_leaky_options_remain_unsupported() {
        let env = include_str!("../../../tests/fixtures/SductTrapped.env")
            .replace("'CVN'", "'CVW'")
            .replace("1450.0 1523.9", "1450.0 100000");
        assert_eq!(
            parse_case(
                &env,
                include_str!("../../../tests/fixtures/SductTrapped.flp"),
                Path::new("sductK.env"),
                Path::new("sductK.flp"),
            )
            .unwrap_err()
            .diagnostics()[0]
                .field,
            "phase_speed_limits"
        );
        let env = include_str!("../../../tests/fixtures/SductTrapped.env")
            .replace("1450.0 1523.9", "1450.0 100000");
        let report = parse_case(
            &env,
            include_str!("../../../tests/fixtures/SductTrapped.flp"),
            Path::new("sductK.env"),
            Path::new("sductK.flp"),
        )
        .unwrap_err();
        assert!(
            report
                .diagnostics()
                .iter()
                .any(|d| d.field == "phase_speed_limits")
        );
    }

    #[test]
    fn spline_with_no_trapped_water_is_rejected_at_legacy_boundary() {
        let env = include_str!("../../../tests/fixtures/Pekeris.env")
            .replace("'NVN'", "'SVN'")
            .replace(
                "100.0 1700.0 0.0 1.5 0.0 0.0 /",
                "100.0 1400.0 0.0 1.5 0.0 0.0 /",
            )
            .replace("1400.0 1700.0", "1300.0 1400.0");
        let report = parse_case(
            &env,
            include_str!("../../../tests/fixtures/Pekeris.flp"),
            Path::new("Pekeris.env"),
            Path::new("Pekeris.flp"),
        )
        .unwrap_err();
        assert!(report.diagnostics().iter().any(|d| {
            d.field == "bottom_sound_speed_mps" && d.path == Path::new("Pekeris.env")
        }));
    }

    #[test]
    fn rigid_bottom_has_no_half_space_record_but_allows_water_loss_units() {
        let env = include_str!("../../../tests/fixtures/PekerisHard.env");
        let unexpected = env.replace("'R' 0.0\n", "'R' 0.0\n100.0 1700.0 0.0 1.5 0.0 0.0 /\n");
        assert_eq!(
            parse_environment(&unexpected, Path::new("case.env"))
                .err()
                .unwrap()
                .diagnostics()[0]
                .field,
            "phase_speed_limits"
        );
        let water_unit = env.replace("'SVN'", "'SVW'");
        assert!(parse_environment(&water_unit, Path::new("case.env")).is_ok());
    }

    #[test]
    fn unsupported_solver_and_field_options_are_rejected() {
        let env = include_str!("../../../tests/fixtures/Pekeris.env").replace("'NVN'", "'NVX'");
        let error = parse_environment(&env, Path::new("Pekeris.env"))
            .err()
            .expect("unsupported attenuation unit should be rejected");
        assert_eq!(error.diagnostics()[0].field, "top_options");
        assert_eq!(error.diagnostics()[0].line, 4);
        for option in ["'NZN'", "'NFN'", "'NPN'"] {
            let env = include_str!("../../../tests/fixtures/Pekeris.env").replace("'NVN'", option);
            assert_eq!(
                parse_environment(&env, Path::new("Pekeris.env"))
                    .err()
                    .unwrap()
                    .diagnostics()[0]
                    .field,
                "top_options"
            );
        }
        let analytic =
            include_str!("../../../tests/fixtures/Pekeris.env").replace("'NVN'", "'AVN'");
        assert_eq!(
            parse_environment(&analytic, Path::new("Pekeris.env"))
                .err()
                .unwrap()
                .diagnostics()[0]
                .field,
            "bottom_options"
        );

        let analytic = include_str!("../../../tests/fixtures/MunkAnalytic.env");
        let extra_point = analytic.replace("'A' 0.0", "0 1500 /\n'A' 0.0");
        assert_eq!(
            parse_environment(&extra_point, Path::new("MunkAnalytic.env"))
                .err()
                .unwrap()
                .diagnostics()[0]
                .field,
            "bottom_options"
        );
        let missing_material = analytic.replace("5000.0  1600.00 0.0 1.8 0.8 /", "5000 1600 /");
        assert_eq!(
            parse_environment(&missing_material, Path::new("MunkAnalytic.env"))
                .err()
                .unwrap()
                .diagnostics()[0]
                .field,
            "bottom_half_space"
        );

        let lossy_water = include_str!("../../../tests/fixtures/MunkBottomLoss.env")
            .replace("200.0 1530.29 /", "200.0 1530.29 0.0 1.0 -0.1 /");
        assert_eq!(
            parse_environment(&lossy_water, Path::new("MunkBottomLoss.env"))
                .err()
                .unwrap()
                .diagnostics()[0]
                .field,
            "sound_speed_profile"
        );
        let unexpected_vacuum_material =
            include_str!("../../../tests/fixtures/Pekeris.env").replace("'A' 0.0", "'V' 0.0");
        assert_eq!(
            parse_environment(&unexpected_vacuum_material, Path::new("Pekeris.env"))
                .err()
                .unwrap()
                .diagnostics()[0]
                .field,
            "phase_speed_limits"
        );
        let excessive_loss =
            include_str!("../../../tests/fixtures/MunkBottomLoss.env").replace("'NVW'", "'NVN'");
        assert_eq!(
            parse_case(
                &excessive_loss,
                include_str!("../../../tests/fixtures/MunkBottomLoss.flp"),
                Path::new("MunkBottomLoss.env"),
                Path::new("MunkBottomLoss.flp")
            )
            .unwrap_err()
            .diagnostics()[0]
                .field,
            "bottom_attenuation_db_per_wavelength"
        );

        let flp = include_str!("../../../tests/fixtures/Pekeris.flp").replace("'X OC'", "'T OC'");
        let error = parse_field(&flp, Path::new("Pekeris.flp"))
            .err()
            .expect("unknown FIELD geometry should be rejected");
        assert_eq!(error.diagnostics()[0].field, "field_options");
        assert_eq!(error.diagnostics()[0].line, 2);
    }

    #[test]
    fn parses_single_profile_field_extensions_and_source_pattern() {
        let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
        let scaled = super::load_case(
            fixtures.join("FieldScaled.env"),
            fixtures.join("FieldScaled.flp"),
        )
        .unwrap();
        assert_eq!(
            scaled.source_geometry,
            crate::SourceGeometry::ScaledCylindrical
        );

        let pattern = super::load_complex_case(
            fixtures.join("FieldPattern.env"),
            fixtures.join("FieldPattern.flp"),
        )
        .unwrap();
        assert_eq!(pattern.mode_addition, crate::ModeAddition::Coherent);
        assert_eq!(pattern.source_pattern.len(), 7);
        assert!((pattern.source_pattern[2].amplitude - 10.0_f64.powf(-3.0 / 20.0)).abs() < 1e-15);
        assert_eq!(
            super::source_pattern_extension(
                include_str!("../../../tests/fixtures/FieldPattern.flp"),
                Path::new("FieldPattern.flp")
            )
            .unwrap(),
            Some("sbp")
        );

        let incoherent = super::load_complex_case(
            fixtures.join("FieldIncoherent.env"),
            fixtures.join("FieldIncoherent.flp"),
        )
        .unwrap();
        assert_eq!(incoherent.mode_addition, crate::ModeAddition::Incoherent);
        assert_eq!(incoherent.source_pattern, []);
    }
}

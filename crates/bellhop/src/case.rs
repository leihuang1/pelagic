use crate::diagnostic::{Diagnostic, DiagnosticReport, LoadOutcome, SourceLocation};
use crate::model::LegacyArrivalEncoding;
use crate::model::{
    BeamFamily, Boundary, BoundaryCondition, BoundaryMaterial, BoundaryShape, EnvironmentCase,
    RangeDependentSoundSpeed, ReceiverGrid, ReflectionCoefficientTable, RunKind, SspInterpolation,
};
use std::collections::HashMap;
use std::ops::Deref;

pub use crate::model::CaseDefinition;

/// A fully loaded and validated two-dimensional BELLHOP input case.
///
/// Construct a case with [`Case::from_definition`], the JSON adapter, or the
/// legacy adapter. Its definition is exposed read-only so its invariants remain
/// valid for the lifetime of the case.
///
/// ```compile_fail
/// fn invalidate(case: &mut bellhop::Case) {
///     case.environment.trace.launch_angles_degrees.clear();
/// }
/// ```
///
/// ```compile_fail
/// fn bypass_validation(definition: bellhop::CaseDefinition) -> bellhop::Case {
///     bellhop::Case(definition)
/// }
/// ```
#[derive(Clone, Debug, PartialEq)]
pub struct Case(CaseDefinition);

impl Case {
    /// Validates candidate data and constructs a case.
    ///
    /// # Errors
    ///
    /// Returns all semantic errors found in the definition.
    pub fn from_definition(
        definition: CaseDefinition,
    ) -> Result<LoadOutcome<Self>, DiagnosticReport> {
        Self::from_definition_with(definition, &HashMap::new(), DiagnosticReport::default())
    }

    #[must_use]
    pub fn into_definition(self) -> CaseDefinition {
        self.0
    }

    pub(crate) fn from_definition_with(
        definition: CaseDefinition,
        locations: &HashMap<&'static str, SourceLocation>,
        mut diagnostics: DiagnosticReport,
    ) -> Result<LoadOutcome<Self>, DiagnosticReport> {
        validate_environment(&definition.environment, locations, &mut diagnostics);
        validate_definition(&definition, locations, &mut diagnostics);
        if diagnostics.has_errors() {
            return Err(diagnostics);
        }
        Ok(LoadOutcome {
            value: Self(definition),
            warnings: diagnostics.diagnostics().to_vec(),
        })
    }
}

impl Deref for Case {
    type Target = CaseDefinition;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

#[allow(clippy::too_many_lines)]
pub(crate) fn validate_environment(
    environment: &EnvironmentCase,
    locations: &HashMap<&'static str, SourceLocation>,
    diagnostics: &mut DiagnosticReport,
) {
    let location = |field| location(environment, locations, field);

    if !environment.frequency_hz.is_finite() || environment.frequency_hz <= 0.0 {
        diagnostics.push(Diagnostic::error(
            "BH0201",
            "frequency must be finite and positive",
            "frequency_hz",
            location("frequency"),
        ));
    }
    let sound_speed = &environment.sound_speed;
    if !sound_speed.top_depth_m.is_finite()
        || !sound_speed.bottom_depth_m.is_finite()
        || sound_speed.bottom_depth_m <= sound_speed.top_depth_m
    {
        diagnostics.push(Diagnostic::error(
            "BH0201",
            "bottom depth must be finite and greater than top depth",
            "sound_speed.bottom_depth_m",
            location("sound_speed.bottom_depth_m"),
        ));
    }

    let positions = &environment.positions;
    require_nonempty(
        positions.source_depths_m.len(),
        "at least one source depth is required",
        "positions.source_depths_m",
        &location,
        diagnostics,
    );
    require_nonempty(
        positions.receiver_depths_m.len(),
        "at least one receiver depth is required",
        "positions.receiver_depths_m",
        &location,
        diagnostics,
    );
    require_nonempty(
        positions.receiver_ranges_m.len(),
        "at least one receiver range is required",
        "positions.receiver_ranges_m",
        &location,
        diagnostics,
    );
    require_finite_f64(
        &positions.source_depths_m,
        "positions.source_depths_m",
        &location,
        diagnostics,
    );
    require_finite_f64(
        &positions.receiver_depths_m,
        "positions.receiver_depths_m",
        &location,
        diagnostics,
    );
    require_finite_f64(
        &positions.receiver_ranges_m,
        "positions.receiver_ranges_m",
        &location,
        diagnostics,
    );

    let top = sound_speed.top_depth_m;
    let bottom = sound_speed.bottom_depth_m;
    if positions
        .source_depths_m
        .iter()
        .any(|depth| *depth < top || *depth > bottom)
    {
        diagnostics.push(Diagnostic::error(
            "BH0201",
            "source depths must lie within the water column",
            "positions.source_depths_m",
            location("positions.source_depths_m"),
        ));
    }
    if positions
        .receiver_depths_m
        .iter()
        .any(|depth| *depth < top || *depth > bottom)
    {
        diagnostics.push(Diagnostic::error(
            "BH0201",
            "receiver depths must lie within the water column",
            "positions.receiver_depths_m",
            location("positions.receiver_depths_m"),
        ));
    }
    if positions
        .receiver_ranges_m
        .windows(2)
        .any(|pair| pair[1] <= pair[0])
    {
        diagnostics.push(Diagnostic::error(
            "BH0201",
            "receiver ranges must be strictly increasing",
            "positions.receiver_ranges_m",
            location("positions.receiver_ranges_m"),
        ));
    }
    if environment.run.receiver_grid == ReceiverGrid::Irregular
        && positions.receiver_depths_m.len() != positions.receiver_ranges_m.len()
    {
        diagnostics.push(Diagnostic::error(
            "BH0201",
            "irregular receiver grids require equal depth and range counts",
            "positions",
            location("positions"),
        ));
    }

    let points = &sound_speed.points;
    if environment.top_options.interpolation != SspInterpolation::AnalyticMunk && points.len() < 2 {
        diagnostics.push(Diagnostic::error(
            "BH0201",
            "non-analytic sound-speed models require at least two points",
            "sound_speed.points",
            location("sound_speed.points"),
        ));
    }
    if points
        .windows(2)
        .any(|pair| pair[1].depth_m <= pair[0].depth_m)
    {
        diagnostics.push(Diagnostic::error(
            "BH0201",
            "sound-speed depths must be strictly increasing",
            "sound_speed.points",
            location("sound_speed.points"),
        ));
    }
    if points.iter().any(|point| {
        !point.depth_m.is_finite()
            || !point.compressional_speed_mps.is_finite()
            || point.compressional_speed_mps <= 0.0
            || !point.shear_speed_mps.is_finite()
            || !point.density_g_cm3.is_finite()
            || point.density_g_cm3 <= 0.0
            || !point.compressional_attenuation.is_finite()
            || !point.shear_attenuation.is_finite()
    }) {
        diagnostics.push(Diagnostic::error(
            "BH0201",
            "sound-speed points contain invalid physical values",
            "sound_speed.points",
            location("sound_speed.points"),
        ));
    }

    let trace = &environment.trace;
    require_nonempty(
        trace.launch_angles_degrees.len(),
        "at least one launch angle is required",
        "trace.launch_angles_degrees",
        &location,
        diagnostics,
    );
    require_finite_f64(
        &trace.launch_angles_degrees,
        "trace.launch_angles_degrees",
        &location,
        diagnostics,
    );
    if let Some(selected) = trace.selected_launch_angle
        && (selected == 0 || selected > trace.launch_angles_degrees.len())
    {
        diagnostics.push(Diagnostic::error(
            "BH0201",
            "selected_launch_angle is outside the one-based launch-angle range",
            "trace.selected_launch_angle",
            location("trace.selected_launch_angle"),
        ));
    }
    if !trace.step_m.is_finite() || trace.step_m < 0.0 {
        diagnostics.push(Diagnostic::error(
            "BH0201",
            "trace step must be finite and non-negative",
            "trace.step_m",
            location("trace.step_m"),
        ));
    }
    if !trace.max_depth_m.is_finite() || trace.max_depth_m <= 0.0 {
        diagnostics.push(Diagnostic::error(
            "BH0201",
            "trace depth limit must be finite and positive",
            "trace.max_depth_m",
            location("trace.max_depth_m"),
        ));
    }
    if !trace.max_range_m.is_finite() || trace.max_range_m <= 0.0 {
        diagnostics.push(Diagnostic::error(
            "BH0201",
            "trace range limit must be finite and positive",
            "trace.max_range_m",
            location("trace.max_range_m"),
        ));
    }
    if let Some(cerveny) = &trace.cerveny
        && (!cerveny.epsilon_multiplier.is_finite() || !cerveny.loop_range.is_finite())
    {
        diagnostics.push(Diagnostic::error(
            "BH0201",
            "Cerveny options must be finite",
            "trace.cerveny",
            location("trace.cerveny"),
        ));
    }

    validate_run_configuration(environment, &location, diagnostics);
    validate_boundary(
        &environment.top_boundary,
        "top_boundary",
        &location,
        diagnostics,
    );
    validate_boundary(
        &environment.bottom_boundary,
        "bottom_boundary",
        &location,
        diagnostics,
    );
    validate_volume_attenuation(environment, &location, diagnostics);

    if environment.run.kind == RunKind::Coherent {
        validate_coherent_beam_count(environment, &location, diagnostics);
    }
}

fn validate_definition(
    case: &CaseDefinition,
    locations: &HashMap<&'static str, SourceLocation>,
    diagnostics: &mut DiagnosticReport,
) {
    let environment = &case.environment;
    let location = |field| location(environment, locations, field);

    let quadrilateral = environment.top_options.interpolation == SspInterpolation::Quadrilateral;
    if quadrilateral != case.range_dependent_sound_speed.is_some() {
        diagnostics.push(Diagnostic::error(
            "BH0201",
            "quadrilateral interpolation requires range-dependent sound speed; other interpolation modes must omit it",
            "sound_speed.range_dependent",
            location("sound_speed.range_dependent"),
        ));
    }
    if let Some(field) = &case.range_dependent_sound_speed {
        validate_range_dependent_sound_speed(field, environment, &location, diagnostics);
    }

    if environment.top_options.has_altimetry != environment.top_boundary.has_shape_file {
        diagnostics.push(Diagnostic::error(
            "BH0201",
            "top boundary shape markers must agree",
            "top_boundary.shape",
            location("top_boundary.shape"),
        ));
    }
    if environment.top_options.has_altimetry != case.altimetry.is_some() {
        diagnostics.push(Diagnostic::error(
            "BH0201",
            "top boundary shape marker and loaded altimetry must agree",
            "top_boundary.shape",
            location("top_boundary.shape"),
        ));
    }
    if environment.bottom_boundary.has_shape_file != case.bathymetry.is_some() {
        diagnostics.push(Diagnostic::error(
            "BH0201",
            "bottom boundary shape marker and loaded bathymetry must agree",
            "bottom_boundary.shape",
            location("bottom_boundary.shape"),
        ));
    }
    validate_boundary_shape(
        case.altimetry.as_ref(),
        true,
        environment.sound_speed.top_depth_m,
        "top_boundary.shape",
        &location,
        diagnostics,
    );
    validate_boundary_shape(
        case.bathymetry.as_ref(),
        false,
        environment.sound_speed.bottom_depth_m,
        "bottom_boundary.shape",
        &location,
        diagnostics,
    );

    validate_reflection_presence(case, &location, diagnostics);
    validate_reflection_table(
        case.top_reflection.as_ref(),
        "top_boundary.condition.table",
        &location,
        diagnostics,
    );
    validate_reflection_table(
        case.bottom_reflection.as_ref(),
        "bottom_boundary.condition.table",
        &location,
        diagnostics,
    );
    validate_internal_reflection(case, &location, diagnostics);

    if environment.run.has_source_beam_pattern != case.source_beam_pattern.is_some() {
        diagnostics.push(Diagnostic::error(
            "BH0201",
            "source beam-pattern marker and loaded pattern must agree",
            "source_beam_pattern",
            location("source_beam_pattern"),
        ));
    }
    if let Some(pattern) = &case.source_beam_pattern
        && (pattern.points.len() < 2
            || pattern.points.iter().any(|point| {
                !point.angle_degrees.is_finite()
                    || !point.level_db.is_finite()
                    || !point.amplitude.is_finite()
            })
            || pattern
                .points
                .windows(2)
                .any(|points| points[1].angle_degrees <= points[0].angle_degrees))
    {
        diagnostics.push(Diagnostic::error(
            "BH0201",
            "source beam patterns require at least two finite, strictly increasing angles",
            "source_beam_pattern.points",
            location("source_beam_pattern.points"),
        ));
    }
}

fn validate_run_configuration(
    environment: &EnvironmentCase,
    location: &impl Fn(&'static str) -> SourceLocation,
    diagnostics: &mut DiagnosticReport,
) {
    let run = &environment.run;
    if (run.kind == RunKind::Rays) != run.beam_family.is_none() {
        diagnostics.push(Diagnostic::error(
            "BH0201",
            "ray runs must omit the beam family and non-ray runs require one",
            "run.beam_family",
            location("run.beam_family"),
        ));
    }
    let cerveny_family = matches!(
        run.beam_family,
        Some(BeamFamily::CervenyCartesian | BeamFamily::CervenyRayCentered)
    );
    if cerveny_family != environment.trace.cerveny.is_some() {
        diagnostics.push(Diagnostic::error(
            "BH0201",
            "Cerveny beam families require trace.cerveny and other families must omit it",
            "trace.cerveny",
            location("trace.cerveny"),
        ));
    }
    if environment.top_options.development_options
        != environment.trace.selected_launch_angle.is_some()
    {
        diagnostics.push(Diagnostic::error(
            "BH0201",
            "development options and selected launch angle must agree",
            "trace.selected_launch_angle",
            location("trace.selected_launch_angle"),
        ));
    }
    let valid_arrival_encoding = match run.kind {
        RunKind::Arrivals => matches!(
            run.arrival_encoding,
            Some(LegacyArrivalEncoding::Ascii | LegacyArrivalEncoding::Binary)
        ),
        _ => run.arrival_encoding.is_none(),
    };
    if !valid_arrival_encoding {
        diagnostics.push(Diagnostic::error(
            "BH0201",
            "only arrival runs may specify an arrival encoding",
            "run.arrival_encoding",
            location("run.arrival_encoding"),
        ));
    }
}

fn validate_boundary(
    boundary: &Boundary,
    field: &'static str,
    location: &impl Fn(&'static str) -> SourceLocation,
    diagnostics: &mut DiagnosticReport,
) {
    if !boundary.roughness_m.is_finite() || boundary.roughness_m < 0.0 {
        diagnostics.push(Diagnostic::error(
            "BH0201",
            "boundary roughness must be finite and non-negative",
            format!("{field}.roughness_m"),
            location(field),
        ));
    }
    match &boundary.condition {
        BoundaryCondition::AcoustoElastic(material) => {
            if !valid_material(
                material.compressional_speed_mps,
                material.shear_speed_mps,
                material.density_g_cm3,
                material.compressional_attenuation,
                material.shear_attenuation,
            ) {
                diagnostics.push(Diagnostic::error(
                    "BH0201",
                    "half-space properties contain invalid physical values",
                    format!("{field}.condition.half_space"),
                    location(field),
                ));
            }
        }
        BoundaryCondition::GrainSize { depth_m, phi }
            if !depth_m.is_finite() || !phi.is_finite() =>
        {
            diagnostics.push(Diagnostic::error(
                "BH0201",
                "grain-size properties must be finite",
                format!("{field}.condition.grain_size"),
                location(field),
            ));
        }
        _ => {}
    }
}

fn validate_volume_attenuation(
    environment: &EnvironmentCase,
    location: &impl Fn(&'static str) -> SourceLocation,
    diagnostics: &mut DiagnosticReport,
) {
    use crate::model::VolumeAttenuation;

    let valid = match &environment.top_options.volume_attenuation {
        VolumeAttenuation::None | VolumeAttenuation::Thorp => true,
        VolumeAttenuation::FrancoisGarrison {
            temperature_c,
            salinity_psu,
            ph,
            mean_depth_m,
        } => [*temperature_c, *salinity_psu, *ph, *mean_depth_m]
            .iter()
            .all(|value| value.is_finite()),
        VolumeAttenuation::Biological { layers } => layers.iter().all(|layer| {
            layer.top_depth_m.is_finite()
                && layer.bottom_depth_m.is_finite()
                && layer.bottom_depth_m >= layer.top_depth_m
                && layer.resonance_frequency_hz.is_finite()
                && layer.resonance_frequency_hz > 0.0
                && layer.quality_factor.is_finite()
                && layer.quality_factor > 0.0
                && layer.attenuation.is_finite()
        }),
    };
    if !valid {
        diagnostics.push(Diagnostic::error(
            "BH0201",
            "volume attenuation contains invalid physical values",
            "sound_speed.volume_attenuation",
            location("sound_speed.volume_attenuation"),
        ));
    }
}

fn validate_range_dependent_sound_speed(
    field: &RangeDependentSoundSpeed,
    environment: &EnvironmentCase,
    location: &impl Fn(&'static str) -> SourceLocation,
    diagnostics: &mut DiagnosticReport,
) {
    if field.ranges_m.len() < 2 || field.depths_m.len() < 2 {
        diagnostics.push(Diagnostic::error(
            "BH0201",
            "range-dependent sound speed requires at least two ranges and depths",
            "sound_speed.range_dependent",
            location("sound_speed.range_dependent"),
        ));
    }
    if field.ranges_m.iter().any(|value| !value.is_finite())
        || field.depths_m.iter().any(|value| !value.is_finite())
        || field.ranges_m.windows(2).any(|v| v[1] <= v[0])
        || field.depths_m.windows(2).any(|v| v[1] <= v[0])
    {
        diagnostics.push(Diagnostic::error(
            "BH0201",
            "range-dependent axes must be finite and strictly increasing",
            "sound_speed.range_dependent",
            location("sound_speed.range_dependent"),
        ));
    }
    if field.speeds_mps.len() != field.depths_m.len()
        || field
            .speeds_mps
            .iter()
            .any(|row| row.len() != field.ranges_m.len())
    {
        diagnostics.push(Diagnostic::error(
            "BH0201",
            "range-dependent speed matrix dimensions must match depths × ranges",
            "sound_speed.range_dependent.speeds_mps",
            location("sound_speed.range_dependent.speeds_mps"),
        ));
    }
    if field
        .speeds_mps
        .iter()
        .flatten()
        .any(|speed| !speed.is_finite() || *speed <= 0.0)
    {
        diagnostics.push(Diagnostic::error(
            "BH0201",
            "range-dependent sound speeds must be finite and positive",
            "sound_speed.range_dependent.speeds_mps",
            location("sound_speed.range_dependent.speeds_mps"),
        ));
    }
    let profile_depths: Vec<f64> = environment
        .sound_speed
        .points
        .iter()
        .map(|point| point.depth_m)
        .collect();
    if field.depths_m != profile_depths {
        diagnostics.push(Diagnostic::error(
            "BH0201",
            "range-dependent depths must match the base sound-speed profile",
            "sound_speed.range_dependent.depths_m",
            location("sound_speed.range_dependent.depths_m"),
        ));
    }
}

fn validate_boundary_shape(
    shape: Option<&BoundaryShape>,
    top: bool,
    profile_depth_m: f64,
    field: &'static str,
    location: &impl Fn(&'static str) -> SourceLocation,
    diagnostics: &mut DiagnosticReport,
) {
    let Some(shape) = shape else {
        return;
    };
    let invalid_geometry = shape.points.is_empty()
        || shape.points.iter().any(|point| {
            !point.range_m.is_finite()
                || !point.depth_m.is_finite()
                || if top {
                    point.depth_m < profile_depth_m
                } else {
                    point.depth_m > profile_depth_m
                }
        })
        || shape
            .points
            .windows(2)
            .any(|points| points[1].range_m <= points[0].range_m);
    if invalid_geometry {
        diagnostics.push(Diagnostic::error(
            "BH0201",
            "boundary shapes require finite valid depths and strictly increasing ranges",
            field,
            location(field),
        ));
    }
    if shape
        .points
        .iter()
        .filter_map(|point| point.material.as_ref())
        .any(|material| !valid_boundary_material(material))
    {
        diagnostics.push(Diagnostic::error(
            "BH0201",
            "boundary material properties contain invalid physical values",
            field,
            location(field),
        ));
    }
}

fn validate_reflection_presence(
    case: &CaseDefinition,
    location: &impl Fn(&'static str) -> SourceLocation,
    diagnostics: &mut DiagnosticReport,
) {
    let top_condition = &case.environment.top_boundary.condition;
    if matches!(
        top_condition,
        BoundaryCondition::PrecalculatedReflectionCoefficient
    ) {
        diagnostics.push(Diagnostic::error(
            "BH0202",
            "the Acoustics Toolbox .irc format defines a bottom impedance and cannot be used for the top boundary",
            "top_boundary.condition",
            location("top_boundary.condition"),
        ));
    }
    if matches!(top_condition, BoundaryCondition::ReflectionCoefficientFile)
        != case.top_reflection.is_some()
    {
        diagnostics.push(Diagnostic::error(
            "BH0201",
            "top reflection condition and loaded table must agree",
            "top_boundary.condition.table",
            location("top_boundary.condition.table"),
        ));
    }

    let bottom_condition = &case.environment.bottom_boundary.condition;
    if matches!(
        bottom_condition,
        BoundaryCondition::ReflectionCoefficientFile
    ) != case.bottom_reflection.is_some()
    {
        diagnostics.push(Diagnostic::error(
            "BH0201",
            "bottom reflection condition and loaded table must agree",
            "bottom_boundary.condition.table",
            location("bottom_boundary.condition.table"),
        ));
    }
    if matches!(
        bottom_condition,
        BoundaryCondition::PrecalculatedReflectionCoefficient
    ) != case.internal_reflection.is_some()
    {
        diagnostics.push(Diagnostic::error(
            "BH0201",
            "bottom internal-reflection condition and loaded table must agree",
            "bottom_boundary.condition.table",
            location("bottom_boundary.condition.table"),
        ));
    }
}

fn validate_reflection_table(
    table: Option<&ReflectionCoefficientTable>,
    field: &'static str,
    location: &impl Fn(&'static str) -> SourceLocation,
    diagnostics: &mut DiagnosticReport,
) {
    if let Some(table) = table
        && (table.points.len() < 2
            || table.points.iter().any(|point| {
                !point.angle_degrees.is_finite()
                    || !point.magnitude.is_finite()
                    || point.magnitude < 0.0
                    || !point.phase_radians.is_finite()
            })
            || table
                .points
                .windows(2)
                .any(|points| points[1].angle_degrees <= points[0].angle_degrees))
    {
        diagnostics.push(Diagnostic::error(
            "BH0201",
            "reflection tables require at least two finite, strictly increasing angles",
            field,
            location(field),
        ));
    }
}

fn validate_internal_reflection(
    case: &CaseDefinition,
    location: &impl Fn(&'static str) -> SourceLocation,
    diagnostics: &mut DiagnosticReport,
) {
    let Some(table) = &case.internal_reflection else {
        return;
    };
    if !table.frequency_hz.is_finite()
        || table.frequency_hz <= 0.0
        || table.points.len() < 2
        || table.points.iter().any(|point| {
            !point.horizontal_wavenumber_squared.is_finite()
                || point.horizontal_wavenumber_squared < 0.0
                || !point.f.re.is_finite()
                || !point.f.im.is_finite()
                || !point.g.re.is_finite()
                || !point.g.im.is_finite()
        })
        || table.points.windows(2).any(|points| {
            points[1].horizontal_wavenumber_squared <= points[0].horizontal_wavenumber_squared
        })
    {
        diagnostics.push(Diagnostic::error(
            "BH0201",
            "internal reflection tables require at least two valid, strictly increasing squared wavenumbers",
            "bottom_boundary.condition.table",
            location("bottom_boundary.condition.table"),
        ));
    }
    let frequency = case.environment.frequency_hz;
    if (table.frequency_hz - frequency).abs() > 1.0e-9 * frequency.abs().max(1.0) {
        diagnostics.push(Diagnostic::warning(
            "BH1004",
            format!(
                "internal reflection-table frequency {} Hz differs from environment frequency {frequency} Hz",
                table.frequency_hz
            ),
            "bottom_boundary.condition.table.frequency_hz",
            location("internal_reflection.frequency"),
        ));
    }
}

fn valid_boundary_material(material: &BoundaryMaterial) -> bool {
    valid_material(
        material.compressional_speed_mps,
        material.shear_speed_mps,
        material.density_g_cm3,
        material.compressional_attenuation,
        material.shear_attenuation,
    )
}

fn valid_material(
    compressional_speed_mps: f64,
    shear_speed_mps: f64,
    density_g_cm3: f64,
    compressional_attenuation: f64,
    shear_attenuation: f64,
) -> bool {
    compressional_speed_mps.is_finite()
        && compressional_speed_mps > 0.0
        && shear_speed_mps.is_finite()
        && shear_speed_mps >= 0.0
        && density_g_cm3.is_finite()
        && density_g_cm3 > 0.0
        && compressional_attenuation.is_finite()
        && shear_attenuation.is_finite()
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn validate_coherent_beam_count(
    case: &EnvironmentCase,
    location: &impl Fn(&'static str) -> SourceLocation,
    diagnostics: &mut DiagnosticReport,
) {
    let source_speed = case
        .sound_speed
        .points
        .first()
        .map_or(1500.0, |point| point.compressional_speed_mps);
    let maximum_range = case
        .positions
        .receiver_ranges_m
        .last()
        .copied()
        .unwrap_or(0.0);
    if maximum_range <= 0.0 || case.trace.launch_angles_degrees.len() <= 1 {
        return;
    }
    let optimal_spacing = (source_speed / (6.0 * case.frequency_hz * maximum_range)).sqrt();
    let angular_span = (case
        .trace
        .launch_angles_degrees
        .last()
        .copied()
        .unwrap_or(0.0)
        - case
            .trace
            .launch_angles_degrees
            .first()
            .copied()
            .unwrap_or(0.0))
    .to_radians();
    let recommended = 2 + (angular_span / optimal_spacing) as usize;
    if case.trace.launch_angles_degrees.len() < recommended {
        diagnostics.push(Diagnostic::warning(
            "BH1003",
            format!(
                "coherent run may use too few beams: {} configured, approximately {recommended} recommended",
                case.trace.launch_angles_degrees.len()
            ),
            "trace.launch_angles_degrees",
            location("trace.launch_angles_degrees"),
        ));
    }
}

fn require_nonempty(
    len: usize,
    message: &'static str,
    field: &'static str,
    location: &impl Fn(&'static str) -> SourceLocation,
    diagnostics: &mut DiagnosticReport,
) {
    if len == 0 {
        diagnostics.push(Diagnostic::error("BH0201", message, field, location(field)));
    }
}

fn require_finite_f64(
    values: &[f64],
    field: &'static str,
    location: &impl Fn(&'static str) -> SourceLocation,
    diagnostics: &mut DiagnosticReport,
) {
    if values.iter().any(|value| !value.is_finite()) {
        diagnostics.push(Diagnostic::error(
            "BH0201",
            "all values must be finite",
            field,
            location(field),
        ));
    }
}

fn location(
    environment: &EnvironmentCase,
    locations: &HashMap<&'static str, SourceLocation>,
    field: &'static str,
) -> SourceLocation {
    locations
        .get(field)
        .cloned()
        .unwrap_or_else(|| SourceLocation::file(&environment.source_path))
}

#[cfg(test)]
mod tests {
    use super::Case;

    const CASE: &[u8] = include_bytes!("../../../examples/field-g.json");

    #[test]
    fn f64_depth_on_fractional_boundary_is_valid() {
        for boundary in [0.1, 100.1] {
            let loaded = crate::json::load_case_document(CASE).unwrap();
            let mut definition = loaded.value.into_definition();
            let mut top = definition.environment.sound_speed.points[0].clone();
            let mut bottom = definition.environment.sound_speed.points[1].clone();
            top.depth_m = 0.0;
            bottom.depth_m = boundary;
            definition.environment.sound_speed.top_depth_m = 0.0;
            definition.environment.sound_speed.bottom_depth_m = boundary;
            definition.environment.sound_speed.points = vec![top, bottom];
            definition.environment.positions.source_depths_m = vec![boundary];
            definition.environment.positions.receiver_depths_m = vec![boundary];

            Case::from_definition(definition).unwrap();
        }
    }

    #[test]
    fn invalid_definition_cannot_become_a_case() {
        let loaded = crate::json::load_case_document(CASE).unwrap();
        let mut definition = loaded.value.into_definition();
        definition.environment.trace.launch_angles_degrees.clear();
        definition.environment.top_options.interpolation =
            crate::model::SspInterpolation::Quadrilateral;
        definition.environment.run.receiver_grid = crate::model::ReceiverGrid::Irregular;

        let report = Case::from_definition(definition).unwrap_err();
        let fields: Vec<_> = report
            .diagnostics()
            .iter()
            .filter_map(|diagnostic| diagnostic.field.as_deref())
            .collect();

        assert!(fields.contains(&"trace.launch_angles_degrees"));
        assert!(fields.contains(&"sound_speed.range_dependent"));
        assert!(fields.contains(&"positions"));
    }
}

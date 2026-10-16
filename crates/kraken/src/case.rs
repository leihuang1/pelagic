use std::ops::Deref;
use std::path::Path;

use crate::solver::error as field_error;
use crate::solver::{elastic, layers, reflection};
use crate::{
    BottomBoundary, CaseDefinition, Diagnostic, DiagnosticReport, Interpolation, ModeSolver,
    SurfaceBoundary,
};
use crate::{FieldPropagation, ModeAddition, SourceGeometry};

pub(crate) const MAX_FIELD_SAMPLES: usize = 1_000_000;
pub(crate) const MAX_VECTOR_LENGTH: usize = 100_000;
pub(crate) const MAX_MESH_POINTS: usize = 1_000_000;
pub(crate) const MAX_MODE_LIMIT: usize = 20_000;

/// A validated case. Its definition is read-only after construction.
///
/// ```compile_fail
/// fn change_frequency(mut case: kraken::Case) {
///     case.frequency_hz = 100.0;
/// }
/// ```
#[derive(Clone, Debug, PartialEq)]
pub struct Case(CaseDefinition);

impl Case {
    /// Validate a candidate fluid-profile case.
    ///
    /// # Errors
    ///
    /// Returns every invalid field in the case definition.
    #[allow(clippy::too_many_lines, clippy::float_cmp)]
    pub fn from_definition(definition: CaseDefinition) -> Result<Self, DiagnosticReport> {
        let mut diagnostics = DiagnosticReport::default();

        for (field, value) in [
            ("frequency_hz", definition.frequency_hz),
            ("water_depth_m", definition.water_depth_m),
            ("water_density_g_cm3", definition.water_density_g_cm3),
            (
                "surface_sound_speed_mps",
                definition.surface_sound_speed_mps,
            ),
            ("surface_density_g_cm3", definition.surface_density_g_cm3),
            (
                "surface_attenuation_db_per_wavelength",
                definition.surface_attenuation_db_per_wavelength,
            ),
            ("bottom_sound_speed_mps", definition.bottom_sound_speed_mps),
            ("bottom_density_g_cm3", definition.bottom_density_g_cm3),
            (
                "bottom_attenuation_db_per_wavelength",
                definition.bottom_attenuation_db_per_wavelength,
            ),
            ("c_low_mps", definition.c_low_mps),
            ("c_high_mps", definition.c_high_mps),
            ("max_range_m", definition.max_range_m),
        ] {
            if !value.is_finite() {
                diagnostics.push(error(field, "value must be finite"));
            }
        }
        if definition.frequency_hz <= 0.0 {
            diagnostics.push(error("frequency_hz", "frequency must be positive"));
        }
        if definition
            .mesh_reference_frequency_hz
            .is_some_and(|frequency| !frequency.is_finite() || frequency <= 0.0)
        {
            diagnostics.push(error(
                "mesh_reference_frequency_hz",
                "mesh reference frequency must be finite and positive",
            ));
        }
        if definition.water_depth_m <= 0.0 {
            diagnostics.push(error("water_depth_m", "water depth must be positive"));
        }
        let minimum_speed = layers::validate(&definition, &mut diagnostics);
        let analytic = definition.interpolation == Interpolation::AnalyticMunk;
        if analytic {
            if !definition.additional_fluid_layers.is_empty() {
                diagnostics.push(error(
                    "additional_fluid_layers",
                    "analytic Munk remains single-layer",
                ));
            }
            if definition.water_depth_m != 5000.0 {
                diagnostics.push(error(
                    "water_depth_m",
                    "analytic Munk profile requires 5000 m",
                ));
            }
            if definition.water_density_g_cm3 != 1.0 {
                diagnostics.push(error(
                    "water_density_g_cm3",
                    "analytic Munk profile requires density 1",
                ));
            }
        }
        if let Some(minimum_speed) = minimum_speed {
            match minimum_speed {
                Ok(minimum_speed) => {
                    if definition.mode_solver == ModeSolver::Kraken
                        && definition.surface_boundary == SurfaceBoundary::FluidHalfSpace
                        && definition.surface_sound_speed_mps > 0.0
                        && definition.surface_sound_speed_mps <= minimum_speed
                    {
                        diagnostics.push(error("surface_sound_speed_mps", "trapped modes require a surface half-space faster than the minimum water sound speed"));
                    }
                    if definition.mode_solver == ModeSolver::Kraken
                        && definition.bottom_boundary == BottomBoundary::FluidHalfSpace
                        && definition.bottom_sound_speed_mps > 0.0
                        && definition.bottom_sound_speed_mps <= minimum_speed
                    {
                        diagnostics.push(error(
                            "bottom_sound_speed_mps",
                            "trapped modes require a bottom faster than the minimum water sound speed",
                        ));
                    }
                }
                Err(report) => diagnostics.push(error(
                    &report.diagnostics()[0].field,
                    &report.diagnostics()[0].message,
                )),
            }
        }
        if let Err(message) = reflection::validate(&definition) {
            diagnostics.push(error("bottom_boundary", message));
        }
        if let Err(message) = reflection::validate_surface(&definition) {
            diagnostics.push(error("surface_boundary", message));
        }
        elastic::validate(&definition, &mut diagnostics);
        if !definition.interface_roughness_m.is_empty()
            && (definition.interface_roughness_m.len()
                != definition.additional_fluid_layers.len() + 2
                || definition
                    .interface_roughness_m
                    .iter()
                    .any(|s| !s.is_finite() || *s < 0.0))
        {
            diagnostics.push(error("interface_roughness_m", "require one finite nonnegative RMS roughness per fluid interface (layer count + 1)"));
        }
        if definition.interface_roughness_m.iter().any(|&s| s != 0.0)
            && (elastic::has_layers(&definition)
                || elastic::has_half_space(&definition)
                || definition.surface_boundary.is_tabulated()
                || definition.bottom_boundary.is_tabulated())
        {
            diagnostics.push(error("interface_roughness_m", "roughness currently requires fluid layers and V/R/fluid-A boundaries without solids or tables"));
        }
        if definition.surface_boundary.is_half_space() {
            if definition.surface_sound_speed_mps <= 0.0 || definition.surface_density_g_cm3 <= 0.0
            {
                diagnostics.push(error(
                    "surface_boundary",
                    "surface half-space requires positive sound speed and density",
                ));
            }
            if !(0.0..=8.685_889_6 * 2.0 * std::f64::consts::PI)
                .contains(&definition.surface_attenuation_db_per_wavelength)
            {
                diagnostics.push(error(
                    "surface_attenuation_db_per_wavelength",
                    "surface loss requires 0 <= Im(c) <= Re(c)",
                ));
            }
            if definition.mode_solver == ModeSolver::Kraken
                && definition.surface_boundary == SurfaceBoundary::FluidHalfSpace
                && definition.c_high_mps > definition.surface_sound_speed_mps
            {
                diagnostics.push(error(
                    "phase_speed_limits",
                    "KRAKEN does not support leaky modes above the surface half-space speed",
                ));
            }
        } else {
            for (field, value) in [
                (
                    "surface_sound_speed_mps",
                    definition.surface_sound_speed_mps,
                ),
                ("surface_density_g_cm3", definition.surface_density_g_cm3),
                (
                    "surface_attenuation_db_per_wavelength",
                    definition.surface_attenuation_db_per_wavelength,
                ),
            ] {
                if value != 0.0 {
                    diagnostics.push(error(
                        field,
                        "non-half-space surface has no half-space material",
                    ));
                }
            }
        }
        if !definition.bottom_boundary.is_half_space() {
            for (field, value) in [
                ("bottom_sound_speed_mps", definition.bottom_sound_speed_mps),
                ("bottom_density_g_cm3", definition.bottom_density_g_cm3),
                (
                    "bottom_attenuation_db_per_wavelength",
                    definition.bottom_attenuation_db_per_wavelength,
                ),
            ] {
                if value != 0.0 {
                    diagnostics.push(error(
                        field,
                        "non-half-space bottom has no half-space material",
                    ));
                }
            }
        } else if definition.bottom_attenuation_db_per_wavelength < 0.0
            || definition.bottom_attenuation_db_per_wavelength
                > 8.685_889_6 * 2.0 * std::f64::consts::PI
        {
            diagnostics.push(error(
                "bottom_attenuation_db_per_wavelength",
                "bottom attenuation must yield a complex speed with imaginary part no larger than real part",
            ));
        }
        if definition.mode_solver == ModeSolver::Kraken
            && definition.bottom_boundary == BottomBoundary::FluidHalfSpace
            && definition.c_high_mps > definition.bottom_sound_speed_mps
        {
            diagnostics.push(error(
                "phase_speed_limits",
                "leaky modes above the bottom sound speed are not supported yet",
            ));
        }
        if definition.water_density_g_cm3 <= 0.0 {
            diagnostics.push(error("water_density_g_cm3", "density must be positive"));
        }
        if definition.bottom_boundary.is_half_space() {
            if definition.bottom_sound_speed_mps <= 0.0 {
                diagnostics.push(error(
                    "bottom_sound_speed_mps",
                    "fluid bottom sound speed must be positive",
                ));
            }
            if definition.bottom_density_g_cm3 <= 0.0 {
                diagnostics.push(error("bottom_density_g_cm3", "density must be positive"));
            }
        }
        if definition.mesh_points != 0 && !(10..=MAX_MESH_POINTS).contains(&definition.mesh_points)
        {
            diagnostics.push(error(
                "mesh_points",
                format!("mesh points must be 0 (automatic) or in 10..={MAX_MESH_POINTS}"),
            ));
        }
        if definition.c_low_mps < 0.0 || definition.c_high_mps <= definition.c_low_mps {
            diagnostics.push(error(
                "phase_speed_limits",
                "require 0 <= c_low < c_high; zero selects the physical minimum",
            ));
        }
        if definition.max_range_m < 0.0 {
            diagnostics.push(error("max_range_m", "maximum range must be non-negative"));
        }
        if definition.mode_limit == 0 || definition.mode_limit > MAX_MODE_LIMIT {
            diagnostics.push(error(
                "mode_limit",
                format!("mode limit must be in 1..={MAX_MODE_LIMIT}"),
            ));
        }
        if !definition.source_pattern.is_empty()
            && (!(2..=MAX_VECTOR_LENGTH).contains(&definition.source_pattern.len())
                || definition.source_pattern.iter().any(|point| {
                    !point.angle_degrees.is_finite()
                        || !point.amplitude.is_finite()
                        || point.amplitude < 0.0
                })
                || definition
                    .source_pattern
                    .windows(2)
                    .any(|pair| pair[1].angle_degrees <= pair[0].angle_degrees))
        {
            diagnostics.push(error(
                "source_pattern",
                "source pattern requires 2..=100000 increasing finite angles and finite nonnegative amplitudes",
            ));
        }
        if definition.mode_sample_depths_m.is_empty()
            || definition.mode_sample_depths_m.len() > MAX_VECTOR_LENGTH
            || definition.mode_sample_depths_m.iter().any(|depth| {
                !depth.is_finite()
                    || *depth < definition.fluid_top_depth_m()
                    || *depth > definition.fluid_bottom_depth_m()
            })
            || definition
                .mode_sample_depths_m
                .windows(2)
                .any(|pair| pair[1] <= pair[0])
        {
            diagnostics.push(error(
                "mode_sample_depths_m",
                "mode depths must be finite, increasing, and inside the water column",
            ));
        }
        for (field, values) in [
            ("source_depths_m", &definition.source_depths_m),
            ("receiver_depths_m", &definition.receiver_depths_m),
            ("receiver_ranges_m", &definition.receiver_ranges_m),
        ] {
            if values.is_empty() || values.len() > MAX_VECTOR_LENGTH {
                diagnostics.push(error(field, "vector length is outside the supported range"));
            }
            if values.iter().any(|value| !value.is_finite()) {
                diagnostics.push(error(field, "values must be finite"));
            }
            if field != "receiver_ranges_m"
                && values.iter().any(|depth| {
                    *depth < definition.fluid_top_depth_m()
                        || *depth > definition.fluid_bottom_depth_m()
                })
            {
                diagnostics.push(error(field, "depths must lie in water"));
            }
        }
        if definition
            .receiver_ranges_m
            .iter()
            .any(|range| *range < 0.0)
            || definition
                .receiver_ranges_m
                .windows(2)
                .any(|pair| pair[1] <= pair[0])
        {
            diagnostics.push(error(
                "receiver_ranges_m",
                "receiver ranges must be non-negative and strictly increasing",
            ));
        }
        if definition.receiver_offsets_m.len() != definition.receiver_depths_m.len()
            || definition
                .receiver_offsets_m
                .iter()
                .any(|offset| !offset.is_finite())
        {
            diagnostics.push(error(
                "receiver_offsets_m",
                "receiver offsets must be finite and match receiver-depth count",
            ));
        }
        if definition.source_geometry == SourceGeometry::Point
            && definition.receiver_ranges_m.first().is_some_and(|&range| {
                definition
                    .receiver_offsets_m
                    .iter()
                    .any(|&offset| !(range + offset).is_finite() || range + offset < 0.0)
            })
        {
            diagnostics.push(error(
                "receiver_offsets_m",
                "point-source effective ranges must be finite and non-negative",
            ));
        }
        if let (Some(first), Some(last)) = (
            definition.mode_sample_depths_m.first(),
            definition.mode_sample_depths_m.last(),
        ) && definition
            .source_depths_m
            .iter()
            .chain(&definition.receiver_depths_m)
            .any(|depth| {
                let extension = if definition.mode_sample_depths_m.len() >= 2 {
                    1500.0 / definition.frequency_hz
                } else {
                    0.0
                };
                *depth < first - extension || *depth > last + extension
            })
        {
            diagnostics.push(error(
                "mode_sample_depths_m",
                "FIELD depths must lie within one reference wavelength of the mode-sample interval",
            ));
        }
        let field_samples = definition
            .source_depths_m
            .len()
            .checked_mul(definition.receiver_depths_m.len())
            .and_then(|count| count.checked_mul(definition.receiver_ranges_m.len()));
        if field_samples.is_none_or(|count| count > MAX_FIELD_SAMPLES) {
            diagnostics.push(error(
                "field_grid",
                format!("field grid may not exceed {MAX_FIELD_SAMPLES} samples"),
            ));
        }

        if diagnostics.diagnostics.is_empty() {
            Ok(Self(definition))
        } else {
            Err(diagnostics)
        }
    }

    #[must_use]
    pub fn into_definition(self) -> CaseDefinition {
        self.0
    }

    #[allow(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss
    )]
    pub(crate) fn mesh_points_at(&self, multiplier: usize) -> Result<usize, DiagnosticReport> {
        layers::mesh_intervals(self, multiplier).map(|intervals| intervals[0])
    }
}

pub(crate) fn error(field: impl Into<String>, message: impl Into<String>) -> Diagnostic {
    Diagnostic::new("KR0201", message, field, Path::new("<case>"), 1, 1)
}

impl Deref for Case {
    type Target = CaseDefinition;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

/// One frequency, with ordered range-independent modal environments.
#[derive(Clone, Debug, PartialEq)]
pub struct FieldCase {
    pub(crate) profiles: Vec<Case>,
    pub(crate) ranges_m: Vec<f64>,
    pub(crate) propagation: FieldPropagation,
}

impl FieldCase {
    /// Validate profile ranges, shared FIELD geometry and coupling requirements.
    /// # Errors
    /// Returns structured diagnostics for inconsistent or oversized sequences.
    #[allow(
        clippy::float_cmp,
        clippy::too_many_lines,
        clippy::cast_possible_truncation
    )]
    pub fn new(
        profiles: Vec<Case>,
        ranges_m: Vec<f64>,
        propagation: FieldPropagation,
    ) -> Result<Self, DiagnosticReport> {
        if profiles.is_empty()
            || profiles.len() > crate::MAX_VECTOR_LENGTH
            || profiles.len() != ranges_m.len()
            || ranges_m[0] != 0.0
            || ranges_m.iter().any(|r| !r.is_finite())
            || ranges_m.windows(2).any(|p| p[1] <= p[0])
            || (profiles.len() == 1) != (propagation == FieldPropagation::RangeIndependent)
        {
            return Err(field_error(
                "KR0201",
                "profile ranges must increase from zero and match the propagation type and profile count",
                "profile_ranges_m",
            ));
        }
        let first = &profiles[0];
        for case in &profiles {
            if case.frequency_hz != first.frequency_hz
                || case.mode_solver != first.mode_solver
                || case.source_geometry != first.source_geometry
                || case.mode_addition != first.mode_addition
                || case.source_pattern != first.source_pattern
                || case.mode_limit != first.mode_limit
                || case.source_depths_m != first.source_depths_m
                || case.receiver_depths_m != first.receiver_depths_m
                || case.receiver_ranges_m != first.receiver_ranges_m
                || case.receiver_offsets_m != first.receiver_offsets_m
            {
                return Err(field_error(
                    "KR0201",
                    "profiles must share frequency, solver and FIELD geometry",
                    "profiles",
                ));
            }
            if propagation == FieldPropagation::Coupled {
                if case.mode_addition == ModeAddition::Incoherent {
                    return Err(field_error(
                        "KR0202",
                        "coupled modes do not support incoherent addition",
                        "mode_addition",
                    ));
                }
                if !case.top_elastic_layers.is_empty()
                    || !case.bottom_elastic_layers.is_empty()
                    || matches!(
                        case.surface_boundary,
                        crate::Boundary::ElasticHalfSpace { .. }
                    )
                    || matches!(
                        case.bottom_boundary,
                        crate::Boundary::ElasticHalfSpace { .. }
                    )
                    || case.surface_boundary.is_tabulated()
                    || case.bottom_boundary.is_tabulated()
                {
                    return Err(field_error(
                        "KR0202",
                        "coupling currently requires smooth fluid profiles",
                        "profiles",
                    ));
                }
                if case.mode_sample_depths_m[0] != case.fluid_top_depth_m()
                    || case.mode_sample_depths_m.last().copied()
                        != Some(case.fluid_bottom_depth_m())
                {
                    return Err(field_error(
                        "KR0201",
                        "coupling requires modes sampled across the full fluid interval",
                        "mode_sample_depths_m",
                    ));
                }
                // ponytail: require PLeft-supported f32 stencils; retabulate for coarser coupling grids.
                let z = &case.mode_sample_depths_m;
                let mut previous_upper = None;
                if crate::solver::layers::iter(case).skip(1).any(|layer| {
                    let depth = layer.top as f32;
                    let upper = z.partition_point(|&z| (z as f32) < depth);
                    let on_grid = z.get(upper).is_some_and(|&z| z as f32 == depth);
                    // Endpoints have no interface correction; interior stencils handle one interface.
                    let unsupported = upper == 0
                        || upper == z.len()
                        || (!on_grid && (upper == 1 || upper + 1 == z.len()))
                        || previous_upper.is_some_and(|previous| {
                            previous == upper || (!on_grid && previous + 1 == upper)
                        });
                    previous_upper = Some(upper);
                    unsupported
                }) {
                    return Err(field_error(
                        "KR0201",
                        "coupling requires a modal grid resolving every fluid-interface quadrature stencil",
                        "mode_sample_depths_m",
                    ));
                }
            }
        }
        if profiles.iter().map(input_values).sum::<usize>() + ranges_m.len() > MAX_SEQUENCE_VALUES {
            return Err(field_error(
                "KR0201",
                "profile sequence exceeds the input storage limit",
                "profiles",
            ));
        }
        Ok(Self {
            profiles,
            ranges_m,
            propagation,
        })
    }

    #[must_use]
    pub fn profiles(&self) -> &[Case] {
        &self.profiles
    }
    #[must_use]
    pub fn ranges_m(&self) -> &[f64] {
        &self.ranges_m
    }
    #[must_use]
    pub fn propagation(&self) -> FieldPropagation {
        self.propagation
    }
}

pub(crate) const MAX_SEQUENCE_VALUES: usize = 5_000_000;
pub(crate) fn input_values(case: &Case) -> usize {
    case.sound_speed_profile.len()
        + case.water_attenuation_db_per_wavelength.len()
        + case.interface_roughness_m.len()
        + case.mode_sample_depths_m.len()
        + case.source_depths_m.len()
        + case.receiver_depths_m.len()
        + case.receiver_ranges_m.len()
        + case.receiver_offsets_m.len()
        + 2 * case.source_pattern.len()
        + case
            .additional_fluid_layers
            .iter()
            .map(|l| 1 + l.sound_speed_profile.len() + l.attenuation_db_per_wavelength.len())
            .sum::<usize>()
        + case
            .top_elastic_layers
            .iter()
            .chain(&case.bottom_elastic_layers)
            .map(|layer| 7 + 6 * layer.material_profile.len())
            .sum::<usize>()
        + [&case.surface_boundary, &case.bottom_boundary]
            .iter()
            .map(|b| match b {
                crate::Boundary::Reflection(p) => p.len(),
                crate::Boundary::Impedance { points, .. } => points.len(),
                _ => 0,
            })
            .sum::<usize>()
}

pub(crate) mod complex_modes;
pub(crate) mod elastic;
mod field;
pub(crate) mod layers;
mod modes;
#[cfg(test)]
pub(crate) mod pekeris;
pub(crate) mod profile;
pub(crate) mod refinement;
pub(crate) mod reflection;
mod scattering;

pub use field::{solve_field, solve_frequencies};

/// First-mesh fluid interval counts for native MOD metadata; uses the solver's mesh policy.
/// # Errors
/// Returns the existing mesh diagnostic when the first mesh is inadmissible.
pub fn base_mesh_intervals(case: &Case) -> Result<Vec<usize>, DiagnosticReport> {
    layers::mesh_intervals(case, 1)
}

use crate::{
    Case, Diagnostic, DiagnosticReport, MAX_FIELD_SAMPLES, MAX_MODE_LIMIT, ModeAddition, ModeSet,
    ModeSolver, PressureField, SimulationResult, SourceGeometry,
};
use num_complex::{Complex32, Complex64};
use std::path::Path;

// ponytail: 550m contributions cover BroadBand/MunkK's 513m per frequency;
// precompute modal phases only if measured throughput requires it.
pub(crate) const MAX_FIELD_WORK: usize = 550_000_000;

pub(super) fn solve_with_limit(
    case: &Case,
    root_limit: usize,
) -> Result<SimulationResult, DiagnosticReport> {
    crate::cancellation::check()?;
    let modes = solve_modes(case, root_limit)?;
    let field = synthesize_field(case, &modes)?;
    Ok(SimulationResult { modes, field })
}

pub(crate) fn solve_modes(case: &Case, root_limit: usize) -> Result<ModeSet, DiagnosticReport> {
    match case.mode_solver {
        crate::ModeSolver::Kraken => modes::solve(case, root_limit),
        crate::ModeSolver::Krakenc => crate::solver::complex_modes::solve(case),
    }
}

#[allow(clippy::too_many_lines)]
fn synthesize_field(case: &Case, mode_set: &ModeSet) -> Result<PressureField, DiagnosticReport> {
    let mode_count = mode_set.modes.len().min(case.mode_limit);
    let sample_count = case
        .source_depths_m
        .len()
        .checked_mul(case.receiver_depths_m.len())
        .and_then(|count| count.checked_mul(case.receiver_ranges_m.len()))
        .ok_or_else(|| error("KR0302", "field grid size overflow", "field_grid"))?;
    if sample_count > MAX_FIELD_SAMPLES
        || sample_count
            .checked_mul(mode_count)
            .is_none_or(|work| work > MAX_FIELD_WORK)
    {
        return Err(error(
            "KR0302",
            format!("field calculation exceeds the {MAX_FIELD_WORK} modal-operation limit"),
            "field_grid",
        ));
    }
    // EvaluateMod.f90: single-precision modal arithmetic and separate range/offset phases.
    #[allow(clippy::approx_constant)]
    let field_pi = 3.141_592_6_f32;
    let factor = Complex32::new(0.0, 1.0)
        * (2.0 * field_pi).sqrt()
        * Complex32::from_polar(1.0, field_pi * 0.25);
    let mut pressure = Vec::with_capacity(sample_count);
    for (source_index, &source_depth) in case.source_depths_m.iter().enumerate() {
        let source_shapes: Vec<_> = mode_set
            .modes
            .iter()
            .take(mode_count)
            .map(|mode| {
                let shape = sample_shape(
                    &mode_set.sampled_depths_m,
                    &mode.eigenfunction,
                    source_depth,
                );
                // Pinned FIELD applies its one source-pattern table only to the
                // first source-depth block.
                if source_index == 0 && !case.source_pattern.is_empty() {
                    double(
                        single(shape)
                            * source_pattern_scale(
                                case,
                                single(mode.horizontal_wavenumber_rad_per_m),
                            ),
                    )
                } else {
                    shape
                }
            })
            .collect();
        for (receiver_index, &receiver_depth) in case.receiver_depths_m.iter().enumerate() {
            let receiver_offset = case.receiver_offsets_m[receiver_index];
            let receiver_shapes: Vec<_> = mode_set
                .modes
                .iter()
                .take(mode_count)
                .map(|mode| {
                    sample_shape(
                        &mode_set.sampled_depths_m,
                        &mode.eigenfunction,
                        receiver_depth,
                    )
                })
                .collect();
            for &range in &case.receiver_ranges_m {
                crate::cancellation::check()?;
                let mut value = Complex32::new(0.0, 0.0);
                for ((mode, source_shape), receiver_shape) in mode_set
                    .modes
                    .iter()
                    .take(mode_count)
                    .zip(&source_shapes)
                    .zip(&receiver_shapes)
                {
                    let k = single(mode.horizontal_wavenumber_rad_per_m);
                    let amplitude = factor * single(*source_shape)
                        / match case.source_geometry {
                            SourceGeometry::Line => k,
                            SourceGeometry::Point | SourceGeometry::ScaledCylindrical => k.sqrt(),
                        };
                    let mut ik = Complex32::new(0.0, -1.0) * k;
                    if case.mode_addition == ModeAddition::Incoherent {
                        ik.im = 0.0;
                    }
                    let ik = double(ik);
                    let offset_shape = single(
                        double(amplitude * single(*receiver_shape)) * (ik * receiver_offset).exp(),
                    );
                    let contribution = offset_shape * single((ik * range).exp());
                    value += if case.mode_addition == ModeAddition::Incoherent {
                        contribution * contribution
                    } else {
                        contribution
                    };
                }
                if case.mode_addition == ModeAddition::Incoherent {
                    value = value.sqrt();
                }
                if case.source_geometry == SourceGeometry::Point && range + receiver_offset > 0.0 {
                    #[allow(clippy::cast_possible_truncation)]
                    let spreading = (range + receiver_offset).sqrt() as f32;
                    value /= spreading;
                }
                if !value.re.is_finite() || !value.im.is_finite() {
                    return Err(error(
                        "KR0302",
                        "field pressure is not finite",
                        "field_grid",
                    ));
                }
                pressure.push(double(value));
            }
        }
    }
    Ok(PressureField {
        source_depths_m: case.source_depths_m.clone(),
        receiver_depths_m: case.receiver_depths_m.clone(),
        receiver_ranges_m: case.receiver_ranges_m.clone(),
        receiver_offsets_m: case.receiver_offsets_m.clone(),
        pressure,
    })
}

#[allow(clippy::cast_possible_truncation)]
pub(crate) fn source_pattern_scale(case: &Case, k: Complex32) -> f32 {
    let omega = 2.0 * std::f64::consts::PI * case.frequency_hz;
    let kz2 = ((omega.powi(2) / 1500.0_f64.powi(2) - double(k * k).re) as f32).max(0.0);
    let angle = (f64::from(kz2).sqrt() / f64::from(k.re))
        .atan()
        .to_degrees();
    let upper = case
        .source_pattern
        .partition_point(|point| point.angle_degrees < angle)
        .clamp(1, case.source_pattern.len() - 1);
    let [left, right] = [case.source_pattern[upper - 1], case.source_pattern[upper]];
    let weight = (angle - left.angle_degrees) / (right.angle_degrees - left.angle_degrees);
    ((1.0 - weight) * left.amplitude + weight * right.amplitude) as f32
}

pub(crate) fn double(value: Complex32) -> Complex64 {
    Complex64::new(f64::from(value.re), f64::from(value.im))
}

#[allow(clippy::cast_possible_truncation)]
pub(crate) fn single(value: Complex64) -> Complex32 {
    Complex32::new(value.re as f32, value.im as f32)
}

fn sample_shape(depths: &[f64], values: &[Complex64], depth: f64) -> Complex64 {
    if depths.len() == 1 {
        return values[0];
    }
    if depth < depths[0] || depth > depths[depths.len() - 1] {
        let i = if depth < depths[0] {
            0
        } else {
            depths.len() - 2
        };
        // ReadModes/Weight_sngl extrapolates complex32 samples, including at the
        // surface; do not insert a synthetic zero or clamp to the first sample.
        #[allow(clippy::cast_possible_truncation)]
        let weight = (depth as f32 - depths[i] as f32) / (depths[i + 1] as f32 - depths[i] as f32);
        return double(single(values[i]) + weight * (single(values[i + 1]) - single(values[i])));
    }
    let index = depths.partition_point(|sample| *sample < depth).max(1);
    let weight = (depth - depths[index - 1]) / (depths[index] - depths[index - 1]);
    values[index - 1] + (values[index] - values[index - 1]) * weight
}

pub(super) fn error(
    code: &'static str,
    message: impl Into<String>,
    field: &str,
) -> DiagnosticReport {
    DiagnosticReport::one(Diagnostic::new(
        code,
        message,
        field,
        Path::new("<case>"),
        1,
        1,
    ))
}

#[cfg(test)]
#[test]
fn endpoint_shape_extension_uses_nearest_complex32_pair_without_clamping() {
    let depths = [1.0, 2.0, 98.0, 99.0];
    let values = [
        Complex64::new(1.0, 2.0),
        Complex64::new(3.0, 5.0),
        Complex64::new(4.0, -1.0),
        Complex64::new(2.0, 2.0),
    ];
    assert_eq!(
        sample_shape(&depths, &values, 0.0),
        Complex64::new(-1.0, -1.0)
    );
    assert_eq!(
        sample_shape(&depths, &values, 100.0),
        Complex64::new(0.0, 5.0)
    );
    assert_eq!(sample_shape(&depths, &values, 99.0), values[3]);
    assert_eq!(sample_shape(&[1.0], &values[..1], 1.0), values[0]);
}

#[cfg(test)]
#[test]
fn field_work_is_bounded_before_modal_products() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/Pekeris");
    let mut input =
        crate::legacy::load_case(root.with_extension("env"), root.with_extension("flp"))
            .unwrap()
            .into_definition();
    input.source_depths_m = vec![75.0; 100_000];
    input.receiver_depths_m = vec![75.0; 5];
    input.receiver_offsets_m = vec![0.0; 5];
    input.receiver_ranges_m = vec![500.0, 1000.0];
    let case = Case::from_definition(input).unwrap();
    let mode = crate::NormalMode {
        horizontal_wavenumber_rad_per_m: Complex64::new(1.0, 0.0),
        phase_speed_mps: 1.0,
        group_speed_mps: 1.0,
        attenuation_nepers_per_m: 0.0,
        eigenfunction: vec![],
    };
    let modes = ModeSet {
        frequency_hz: case.frequency_hz,
        sampled_depths_m: case.mode_sample_depths_m.clone(),
        modes: vec![mode; 551],
    };
    assert_eq!(
        synthesize_field(&case, &modes).unwrap_err().diagnostics()[0].field,
        "field_grid"
    );
}

/// Compute normal modes and a supported range-independent FIELD.
///
/// # Errors
///
/// Returns a diagnostic if the phase-speed range contains no supported modes or
/// the field exceeds the bounded modal-work limit.
pub fn solve(case: &Case) -> Result<SimulationResult, DiagnosticReport> {
    solve_with_limit(case, MAX_MODE_LIMIT)
}

/// Compute complex normal modes without synthesizing a FIELD.
///
/// # Errors
///
/// Returns a diagnostic when the fluid complex-mode subset cannot be solved.
pub fn solve_complex_modes(case: &Case) -> Result<ModeSet, DiagnosticReport> {
    if case.mode_solver != ModeSolver::Krakenc {
        return Err(DiagnosticReport::one(crate::case::error(
            "mode_solver",
            "requires KRAKENC input",
        )));
    }
    complex_modes::solve(case)
}

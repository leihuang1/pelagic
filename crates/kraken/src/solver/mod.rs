pub(crate) mod complex_modes;
pub(crate) mod elastic;
mod field;
pub(crate) mod layers;
mod modes;
mod prepared;
pub use prepared::{PreparedModes, prepare_modes};
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

// ponytail: 550m contributions cover BroadBand/MunkK's 513m per frequency.
pub(crate) const MAX_FIELD_WORK: usize = 550_000_000;
// Range phases are tabulated for blocks of ranges with at most this many modal terms.
const PHASE_TABLE_VALUES: usize = 1 << 20;
// Ranges whose modal sums share a loop, one running total per range, so the
// compiler can pack them. Each range still adds its modes in order.
const FIELD_RANGE_LANES: usize = 16;
// Fewer ranges than half a lane group would mostly sum padding.
const MIN_LANE_RANGES: usize = FIELD_RANGE_LANES / 2;

struct RangePhaseLanes {
    re: [f32; FIELD_RANGE_LANES],
    im: [f32; FIELD_RANGE_LANES],
}

enum FieldPhases {
    Scalar(Vec<Vec<Complex32>>),
    RangeLanes(Vec<Vec<RangePhaseLanes>>),
}

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

/// Whether [`solve_modes`] carries the mode count forward as its root limit, so that
/// a sequence of such solves runs in order. KRAKENC takes no root limit. The rule is
/// conservative: an elastic top without finite layers replaces the carried count
/// before reading it, so such sequences could solve side by side.
pub(crate) fn carries_mode_count(case: &Case) -> bool {
    match case.mode_solver {
        crate::ModeSolver::Kraken => modes::elastic_search(case),
        crate::ModeSolver::Krakenc => false,
    }
}

fn synthesize_field(case: &Case, mode_set: &ModeSet) -> Result<PressureField, DiagnosticReport> {
    tabulate_field(case, mode_set, PHASE_TABLE_VALUES)
}

fn tabulate_field(
    case: &Case,
    mode_set: &ModeSet,
    table_values: usize,
) -> Result<PressureField, DiagnosticReport> {
    tabulate_field_query(
        case,
        &case.receiver_ranges_m,
        &case.receiver_offsets_m,
        mode_set,
        table_values,
    )
}

/// FIELD at these ranges and offsets, with every other setting taken from `case`.
/// Range-phase and source-amplitude tables hold at most about `table_values`
/// modal terms each; the table size never changes the result.
#[allow(clippy::too_many_lines)]
fn tabulate_field_query(
    case: &Case,
    receiver_ranges_m: &[f64],
    receiver_offsets_m: &[f64],
    mode_set: &ModeSet,
    table_values: usize,
) -> Result<PressureField, DiagnosticReport> {
    let mode_count = mode_set.modes.len().min(case.mode_limit);
    let sample_count = case
        .source_depths_m
        .len()
        .checked_mul(case.receiver_depths_m.len())
        .and_then(|count| count.checked_mul(receiver_ranges_m.len()))
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
    let modes = &mode_set.modes[..mode_count];
    let incoherent = case.mode_addition == ModeAddition::Incoherent;
    // Each modal term below is evaluated with the same operations as a direct
    // sum; only factors shared by many receivers are computed once and reused.
    let wavenumbers: Vec<_> = modes
        .iter()
        .map(|mode| {
            let k = single(mode.horizontal_wavenumber_rad_per_m);
            let mut ik = Complex32::new(0.0, -1.0) * k;
            if incoherent {
                ik.im = 0.0;
            }
            (k, double(ik))
        })
        .collect();
    let amplitudes_at = |source_index: usize, source_depth: f64| -> Vec<Complex32> {
        modes
            .iter()
            .zip(&wavenumbers)
            .map(|(mode, &(k, _))| {
                let shape = sample_shape(
                    &mode_set.sampled_depths_m,
                    &mode.eigenfunction,
                    source_depth,
                );
                // Pinned FIELD applies its one source-pattern table only to the
                // first source-depth block.
                let source_shape = if source_index == 0 && !case.source_pattern.is_empty() {
                    double(
                        single(shape)
                            * source_pattern_scale(
                                case,
                                single(mode.horizontal_wavenumber_rad_per_m),
                            ),
                    )
                } else {
                    shape
                };
                factor * single(source_shape)
                    / match case.source_geometry {
                        SourceGeometry::Line => k,
                        SourceGeometry::Point | SourceGeometry::ScaledCylindrical => k.sqrt(),
                    }
            })
            .collect()
    };
    let receiver_count = case.receiver_depths_m.len();
    let range_count = receiver_ranges_m.len();
    let range_capacity = table_values / mode_count.max(1);
    let range_lanes = range_count >= MIN_LANE_RANGES && range_capacity >= FIELD_RANGE_LANES;
    // Round capacity, not the range count, so padding never exceeds the table budget.
    let block_len = if range_lanes {
        range_capacity / FIELD_RANGE_LANES * FIELD_RANGE_LANES
    } else {
        range_capacity
    }
    .clamp(1, range_count.max(1));
    let source_batch = range_capacity.max(1);
    let mut pressure = vec![Complex64::new(0.0, 0.0); sample_count];
    for (block_index, ranges) in receiver_ranges_m.chunks(block_len).enumerate() {
        let phases = if range_lanes && ranges.len() >= MIN_LANE_RANGES {
            FieldPhases::RangeLanes(crate::parallelism::map(
                ranges.len().div_ceil(FIELD_RANGE_LANES),
                ranges.len().saturating_mul(mode_count),
                |index| {
                    let first = index * FIELD_RANGE_LANES;
                    let lane_ranges = &ranges[first..ranges.len().min(first + FIELD_RANGE_LANES)];
                    wavenumbers
                        .iter()
                        .map(|&(_, ik)| {
                            let mut phases = RangePhaseLanes {
                                re: [0.0; FIELD_RANGE_LANES],
                                im: [0.0; FIELD_RANGE_LANES],
                            };
                            for (lane, &range) in lane_ranges.iter().enumerate() {
                                let phase = single((ik * range).exp());
                                phases.re[lane] = phase.re;
                                phases.im[lane] = phase.im;
                            }
                            phases
                        })
                        .collect()
                },
            ))
        } else {
            FieldPhases::Scalar(crate::parallelism::map(
                ranges.len(),
                ranges.len().saturating_mul(mode_count),
                |index| {
                    wavenumbers
                        .iter()
                        .map(|&(_, ik)| single((ik * ranges[index]).exp()))
                        .collect()
                },
            ))
        };
        let start = block_index * block_len;
        for (batch_index, sources) in case.source_depths_m.chunks(source_batch).enumerate() {
            let first_source = batch_index * source_batch;
            let amplitudes: Vec<_> = sources
                .iter()
                .enumerate()
                .map(|(offset, &source_depth)| amplitudes_at(first_source + offset, source_depth))
                .collect();
            let row_count = sources.len() * receiver_count;
            let estimated_work = row_count
                .saturating_mul(mode_count.max(1))
                .saturating_mul(ranges.len() + 1);
            let rows = crate::parallelism::map(row_count, estimated_work, |row| {
                crate::cancellation::check()?;
                let receiver_index = row % receiver_count;
                let receiver_depth = case.receiver_depths_m[receiver_index];
                let receiver_offset = receiver_offsets_m[receiver_index];
                let offset_shapes: Vec<_> = modes
                    .iter()
                    .zip(&amplitudes[row / receiver_count])
                    .zip(&wavenumbers)
                    .map(|((mode, amplitude), &(_, ik))| {
                        let receiver_shape = sample_shape(
                            &mode_set.sampled_depths_m,
                            &mode.eigenfunction,
                            receiver_depth,
                        );
                        single(
                            double(amplitude * single(receiver_shape))
                                * (ik * receiver_offset).exp(),
                        )
                    })
                    .collect();
                let finish = |mut value: Complex32, range: f64| {
                    if incoherent {
                        value = value.sqrt();
                    }
                    if case.source_geometry == SourceGeometry::Point
                        && range + receiver_offset > 0.0
                    {
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
                    Ok(double(value))
                };
                match &phases {
                    FieldPhases::Scalar(phases) => ranges
                        .iter()
                        .zip(phases)
                        .map(|(&range, phases)| {
                            let mut value = Complex32::new(0.0, 0.0);
                            for (offset_shape, phase) in offset_shapes.iter().zip(phases) {
                                let contribution = offset_shape * phase;
                                value += if incoherent {
                                    contribution * contribution
                                } else {
                                    contribution
                                };
                            }
                            finish(value, range)
                        })
                        .collect::<Result<Vec<_>, _>>(),
                    FieldPhases::RangeLanes(phases) => {
                        let mut values = Vec::with_capacity(ranges.len());
                        for (lane_ranges, phases) in ranges.chunks(FIELD_RANGE_LANES).zip(phases) {
                            let mut re = [0.0; FIELD_RANGE_LANES];
                            let mut im = [0.0; FIELD_RANGE_LANES];
                            for (offset_shape, phase) in offset_shapes.iter().zip(phases) {
                                // Padded lanes are discarded; live lanes keep their modal order.
                                #[allow(clippy::needless_range_loop)]
                                for lane in 0..FIELD_RANGE_LANES {
                                    let contribution = offset_shape
                                        * Complex32::new(phase.re[lane], phase.im[lane]);
                                    let contribution = if incoherent {
                                        contribution * contribution
                                    } else {
                                        contribution
                                    };
                                    re[lane] += contribution.re;
                                    im[lane] += contribution.im;
                                }
                            }
                            for (lane, &range) in lane_ranges.iter().enumerate() {
                                values.push(finish(Complex32::new(re[lane], im[lane]), range)?);
                            }
                        }
                        Ok(values)
                    }
                }
            });
            let first_row = first_source * receiver_count;
            for (row, values) in rows.into_iter().enumerate() {
                pressure[(first_row + row) * range_count + start..][..ranges.len()]
                    .copy_from_slice(&values?);
            }
        }
    }
    Ok(PressureField {
        source_depths_m: case.source_depths_m.clone(),
        receiver_depths_m: case.receiver_depths_m.clone(),
        receiver_ranges_m: receiver_ranges_m.to_vec(),
        receiver_offsets_m: receiver_offsets_m.to_vec(),
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

#[cfg(test)]
#[test]
fn table_size_never_changes_the_field() {
    for (stem, solver, sources) in [
        (
            "FieldPattern",
            crate::ModeSolver::Krakenc,
            vec![75.0, 30.0, 60.0, 75.0, 10.0],
        ),
        ("Pekeris", crate::ModeSolver::Kraken, vec![75.0, 20.0, 50.0]),
    ] {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(stem);
        let mut input = crate::legacy::load_field_cases(
            root.with_extension("env"),
            root.with_extension("flp"),
            solver,
        )
        .unwrap()
        .remove(0)
        .profiles
        .remove(0)
        .into_definition();
        input.source_depths_m = sources;
        let case = Case::from_definition(input).unwrap();
        let modes = solve_modes(&case, MAX_MODE_LIMIT).unwrap();
        let expected = format!("{:?}", synthesize_field(&case, &modes));
        // One value per table puts each source and range in a block of its own.
        for table_values in [1, 2, modes.modes.len(), 3 * modes.modes.len() + 1] {
            assert_eq!(
                format!("{:?}", tabulate_field(&case, &modes, table_values)),
                expected,
                "{stem} with {table_values}-value tables"
            );
        }
    }
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

#[cfg(test)]
mod field_range_lane_tests {
    use super::*;

    fn field_case(range_count: i32) -> (Case, ModeSet) {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/FieldPattern");
        let mut input = crate::legacy::load_field_cases(
            root.with_extension("env"),
            root.with_extension("flp"),
            ModeSolver::Krakenc,
        )
        .unwrap()
        .remove(0)
        .profiles
        .remove(0)
        .into_definition();
        assert_ne!(input.source_pattern, []);
        input.source_depths_m = vec![75.0, 30.0, 75.0, 10.0, 20.0, 40.0, 50.0, 60.0, 75.0];
        input.receiver_depths_m = vec![0.0, 35.0, 100.0];
        input.receiver_offsets_m = vec![-25.0, 0.0, 40.0];
        input.receiver_ranges_m = (0..range_count)
            .map(|i| 25.0 + 125.0 * f64::from(i))
            .collect();
        input.mode_sample_depths_m = vec![1.0, 50.0, 99.0];
        let case = Case::from_definition(input).unwrap();
        let modes = ModeSet {
            frequency_hz: case.frequency_hz,
            sampled_depths_m: case.mode_sample_depths_m.clone(),
            modes: (0..11)
                .map(|i| {
                    let i = f64::from(i);
                    crate::NormalMode {
                        horizontal_wavenumber_rad_per_m: Complex64::new(
                            0.04 + 0.007 * i,
                            -0.000_01 * (i + 1.0),
                        ),
                        phase_speed_mps: 1500.0,
                        group_speed_mps: 1500.0,
                        attenuation_nepers_per_m: 0.000_01 * (i + 1.0),
                        eigenfunction: vec![
                            Complex64::new(-0.12 * (i + 1.0), 0.07),
                            Complex64::new(0.09, 0.014 * i),
                            Complex64::new(-0.022 * (i + 1.0), -0.031),
                        ],
                    }
                })
                .collect(),
        };
        (case, modes)
    }

    fn pressure_bytes(field: &PressureField) -> Vec<u8> {
        field
            .pressure
            .iter()
            .flat_map(|value| {
                value
                    .re
                    .to_le_bytes()
                    .into_iter()
                    .chain(value.im.to_le_bytes())
            })
            .collect()
    }

    #[test]
    fn tabulate_field_range_lanes_match_scalar_bytes() {
        for range_count in [1, 3, 7, 8, 9, 15, 16, 17, 23, 24, 31, 32, 33, 129] {
            let (base, modes) = field_case(range_count);
            for geometry in [
                SourceGeometry::Line,
                SourceGeometry::Point,
                SourceGeometry::ScaledCylindrical,
            ] {
                for addition in [ModeAddition::Coherent, ModeAddition::Incoherent] {
                    let mut input = base.clone().into_definition();
                    input.source_geometry = geometry;
                    input.mode_addition = addition;
                    let case = Case::from_definition(input).unwrap();
                    // A one-value budget forces the unchanged scalar modal sum.
                    let expected = tabulate_field(&case, &modes, 1).unwrap();
                    let mode_count = modes.modes.len();
                    for table_values in [
                        0,
                        2,
                        mode_count,
                        7 * mode_count + 1,
                        8 * mode_count,
                        9 * mode_count + 1,
                        15 * mode_count + 1,
                        16 * mode_count,
                        16 * mode_count + 1,
                        17 * mode_count + 1,
                        32 * mode_count + 1,
                        PHASE_TABLE_VALUES,
                    ] {
                        let actual = tabulate_field(&case, &modes, table_values).unwrap();
                        assert_eq!(
                            pressure_bytes(&actual),
                            pressure_bytes(&expected),
                            "{range_count} ranges, {geometry:?}, {addition:?}, {table_values} values"
                        );
                    }
                    if range_count == 129 {
                        let actual = crate::parallelism::with_threads(
                            std::num::NonZeroUsize::new(3).unwrap(),
                            || tabulate_field(&case, &modes, PHASE_TABLE_VALUES).unwrap(),
                        );
                        assert_eq!(pressure_bytes(&actual), pressure_bytes(&expected));
                    }
                }
            }
        }
    }

    #[test]
    fn tabulate_field_range_lanes_preserve_errors_and_cancellation() {
        let (case, mut modes) = field_case(17);
        let token = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
        for table_values in [1, PHASE_TABLE_VALUES] {
            let report = crate::cancellation::with_token(token.clone(), || {
                crate::parallelism::with_threads(std::num::NonZeroUsize::new(3).unwrap(), || {
                    tabulate_field(&case, &modes, table_values).unwrap_err()
                })
            });
            assert_eq!(report.diagnostics()[0].code, "KR0304");
        }
        modes.modes[0].eigenfunction[0].re = f64::NAN;
        for table_values in [1, PHASE_TABLE_VALUES] {
            let report = tabulate_field(&case, &modes, table_values).unwrap_err();
            assert_eq!(report.diagnostics()[0].code, "KR0302");
            assert_eq!(
                report.diagnostics()[0].message,
                "field pressure is not finite"
            );
        }
    }
}

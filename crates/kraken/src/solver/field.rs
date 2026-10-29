//! Range-dependent 2D FIELD propagation.
use crate::case::MAX_SEQUENCE_VALUES;
use crate::solver::{double, elastic::complex_speed, error, single, source_pattern_scale};
use crate::{
    Case, DiagnosticReport, FieldCase, FieldPropagation, ModeAddition, ModeSet, PressureField,
    ProfileSimulationResult, SourceGeometry,
};
use num_complex::{Complex32, Complex64};

// Range factors, mode amplitudes and finished rows are held for blocks of at
// most this many values (1 MiB of complex doubles), so memory stays near a
// direct march; larger tables gain little.
const TABLE_VALUES: usize = 1 << 16;

/// Compute every profile's modes and synthesize one range-dependent field.
/// # Errors
/// Returns modal, cumulative storage/work or non-finite pressure diagnostics.
pub fn solve_field(case: &FieldCase) -> Result<ProfileSimulationResult, DiagnosticReport> {
    let mut root_limit = crate::MAX_MODE_LIMIT;
    solve_field_with_limit(case, &mut root_limit)
}

/// Solve ordered frequency blocks, stopping after the first failed block.
/// Blocks must share one mode solver; a mismatch yields `KR0201` without solving it.
/// Real finite-elastic KRAKEN retains Solve2's search bound across meshes and
/// frequencies; separate calls to `solve_field` start separate reference runs.
pub fn solve_frequencies(
    cases: &[FieldCase],
) -> impl Iterator<Item = Result<ProfileSimulationResult, DiagnosticReport>> + '_ {
    cases
        .iter()
        .scan(Some(crate::MAX_MODE_LIMIT), |limit, case| {
            let root_limit = limit.as_mut()?;
            let result = if case.profiles[0].mode_solver == cases[0].profiles[0].mode_solver {
                solve_field_with_limit(case, root_limit)
            } else {
                Err(error(
                    "KR0201",
                    "frequency blocks must share the same mode solver",
                    "mode_solver",
                ))
            };
            if result.is_err() {
                *limit = None;
            }
            Some(result)
        })
}

#[allow(clippy::too_many_lines)] // FieldCase construction guarantees a nonempty sequence
fn solve_field_with_limit(
    case: &FieldCase,
    root_limit: &mut usize,
) -> Result<ProfileSimulationResult, DiagnosticReport> {
    crate::cancellation::check()?;
    if case.propagation == FieldPropagation::RangeIndependent {
        let result = crate::solver::solve_with_limit(&case.profiles[0], *root_limit)?;
        *root_limit = result.modes.modes.len();
        return Ok(ProfileSimulationResult {
            modes: vec![result.modes],
            field: result.field,
        });
    }
    let mut modes = Vec::with_capacity(case.profiles.len());
    let mut shapes = 0;
    for (index, profile) in case.profiles.iter().enumerate() {
        let set = crate::solver::solve_modes(profile, *root_limit).map_err(|mut report| {
            for d in &mut report.diagnostics {
                d.field = format!("profiles[{index}].{}", d.field);
            }
            report
        })?;
        *root_limit = set.modes.len();
        shapes += set.modes.len() * set.sampled_depths_m.len();
        if shapes > MAX_SEQUENCE_VALUES {
            return Err(error(
                "KR0302",
                "profile mode shapes exceed the cumulative storage limit",
                "profiles",
            ));
        }
        modes.push(set);
    }
    let first = &case.profiles[0];
    let samples =
        first.source_depths_m.len() * first.receiver_depths_m.len() * first.receiver_ranges_m.len();
    let largest = modes.iter().map(|m| m.modes.len()).max().unwrap_or(0);
    let coupling_work = if case.propagation == FieldPropagation::Coupled {
        modes
            .windows(2)
            .zip(case.profiles.windows(2))
            .map(|(m, p)| {
                let left = m[0].modes.len();
                let right = m[1].modes.len();
                let grid = m[0].sampled_depths_m.len() + m[1].sampled_depths_m.len();
                // Pressure tabulation, retabulation/projection, two half-space tails,
                // and receiver interpolation; no dense coupling matrix is stored.
                left.saturating_mul(m[0].sampled_depths_m.len())
                    .saturating_add((left + right).saturating_mul(grid))
                    .saturating_add(2_usize.saturating_mul(left).saturating_mul(right))
                    .saturating_add(right.saturating_mul(p[1].receiver_depths_m.len()))
            })
            .fold(0_usize, usize::saturating_add)
            .saturating_mul(first.source_depths_m.len())
    } else {
        0
    };
    if samples > crate::MAX_FIELD_SAMPLES
        || samples
            .saturating_mul(largest)
            .saturating_add(coupling_work)
            > crate::solver::MAX_FIELD_WORK
    {
        return Err(error(
            "KR0302",
            "profile FIELD exceeds the cumulative modal-operation limit",
            "field_grid",
        ));
    }
    let pressure = synthesize(case, &modes, samples, TABLE_VALUES)?;
    Ok(ProfileSimulationResult {
        modes,
        field: PressureField {
            source_depths_m: first.source_depths_m.clone(),
            receiver_depths_m: first.receiver_depths_m.clone(),
            receiver_ranges_m: first.receiver_ranges_m.clone(),
            receiver_offsets_m: first.receiver_offsets_m.clone(),
            pressure,
        },
    })
}

/// The pressure of every source block, holding at most about `table_values`
/// per-range modal terms or pressures at once; the size never changes the result.
fn synthesize(
    case: &FieldCase,
    modes: &[ModeSet],
    samples: usize,
    table_values: usize,
) -> Result<Vec<Complex64>, DiagnosticReport> {
    let first = &case.profiles[0];
    let mut pressure = vec![Complex64::new(0.0, 0.0); samples];
    for (source, &depth) in first.source_depths_m.iter().enumerate() {
        let count = modes[0].modes.len().min(first.mode_limit);
        let excitation: Vec<_> = modes[0]
            .modes
            .iter()
            .take(count)
            .map(|mode| {
                let shape = sample(&modes[0], mode, depth);
                if source == 0 && !first.source_pattern.is_empty() {
                    shape
                        * source_pattern_scale(first, single(mode.horizontal_wavenumber_rad_per_m))
                } else {
                    shape
                }
            })
            .collect();
        let block = &mut pressure[source * samples / first.source_depths_m.len()
            ..(source + 1) * samples / first.source_depths_m.len()];
        match case.propagation {
            FieldPropagation::Adiabatic => {
                adiabatic(case, modes, &excitation, block, table_values)?;
            }
            FieldPropagation::Coupled => coupled(case, modes, &excitation, block, table_values)?,
            FieldPropagation::RangeIndependent => unreachable!(),
        }
    }
    if pressure
        .iter()
        .any(|p| !p.re.is_finite() || !p.im.is_finite())
    {
        return Err(error(
            "KR0302",
            "field pressure is not finite",
            "field_grid",
        ));
    }
    Ok(pressure)
}

#[allow(clippy::approx_constant)]
fn factor() -> Complex32 {
    let pi = 3.141_592_6_f32;
    Complex32::new(0.0, 1.0) * (2.0 * pi).sqrt() * Complex32::from_polar(1.0, pi * 0.25)
}

fn sample(set: &ModeSet, mode: &crate::NormalMode, depth: f64) -> Complex32 {
    sample_at(mode, bracket(set, depth))
}

/// The upper sample and single-precision weight for `depth`; `None` when the
/// set has one sample. Shared by every mode of the set.
#[allow(clippy::cast_possible_truncation)]
fn bracket(set: &ModeSet, depth: f64) -> Option<(usize, f32)> {
    let z = &set.sampled_depths_m;
    if z.len() == 1 {
        return None;
    }
    let upper = z.partition_point(|d| *d < depth).clamp(1, z.len() - 1);
    let w = (depth as f32 - z[upper - 1] as f32) / (z[upper] as f32 - z[upper - 1] as f32);
    Some((upper, w))
}

fn sample_at(mode: &crate::NormalMode, bracket: Option<(usize, f32)>) -> Complex32 {
    let Some((upper, w)) = bracket else {
        return single(mode.eigenfunction[0]);
    };
    let left = single(mode.eigenfunction[upper - 1]);
    left + w * (single(mode.eigenfunction[upper]) - left)
}

#[allow(clippy::cast_possible_truncation)]
fn interpolate(left: Complex32, right: Complex32, w: f64) -> Complex32 {
    single(double(left) + w * double(right - left))
}

/// Fill a block of `len` ranges from `start`, one receiver depth per work item
/// and at most about `table_values` pressures held at once. Each row samples
/// its depth once per profile, then evaluates every range of the block.
fn fill_rows(
    pressure: &mut [Complex64],
    depths: &[f64],
    range_count: usize,
    (start, len): (usize, usize),
    table_values: usize,
    row: impl Fn(f64) -> Vec<Complex64> + Sync,
) -> Result<(), DiagnosticReport> {
    let batch = (table_values / len).max(1);
    for (batch_index, depths) in depths.chunks(batch).enumerate() {
        let rows = crate::parallelism::map(depths.len(), |index| {
            crate::cancellation::check()?;
            Ok(row(depths[index]))
        });
        for (offset, values) in rows.into_iter().enumerate() {
            let iz = batch_index * batch + offset;
            pressure[iz * range_count + start..][..len].copy_from_slice(&values?);
        }
    }
    Ok(())
}

/// One receiver range of the adiabatic march: the profiles it lies between,
/// its weight, and a term per mode: the phase integral, then the range factor.
struct AdiabaticRange {
    segment: usize,
    right: usize,
    w: f64,
    range: f64,
    terms: Vec<Complex64>,
}

impl AdiabaticRange {
    /// The range with each mode's phase integral replaced by its range factor.
    #[allow(clippy::float_cmp)]
    fn with_factors(&self, first: &Case, modes: &[ModeSet], constants: &[Complex64]) -> Self {
        let incoherent = first.mode_addition == ModeAddition::Incoherent;
        let factors = self
            .terms
            .iter()
            .enumerate()
            .map(|(m, integral)| {
                let k = interpolate(
                    single(modes[self.segment].modes[m].horizontal_wavenumber_rad_per_m),
                    single(modes[self.right].modes[m].horizontal_wavenumber_rad_per_m),
                    self.w,
                );
                let phase =
                    Complex64::new(integral.im, if incoherent { 0.0 } else { -integral.re });
                let h = constants[m]
                    * if incoherent {
                        phase.exp()
                    } else {
                        double(single(phase.exp()))
                    };
                match first.source_geometry {
                    SourceGeometry::Line => h / double(k),
                    SourceGeometry::Point if self.range == 0.0 => Complex64::new(0.0, 0.0),
                    SourceGeometry::Point => h / (double(k) * self.range).sqrt(),
                    SourceGeometry::ScaledCylindrical => h / double(k.sqrt()),
                }
            })
            .collect();
        Self {
            terms: factors,
            ..*self
        }
    }
}

/// One receiver depth's pressure at each range of a block, from its range factors.
fn adiabatic_row(
    steps: &[AdiabaticRange],
    modes: &[ModeSet],
    depth: f64,
    incoherent: bool,
) -> Vec<Complex64> {
    // Each mode's shape at this depth in both profiles, as interpolate() takes them.
    let mut profiles = None;
    let (mut base, mut slope) = (Vec::new(), Vec::new());
    steps
        .iter()
        .map(|step| {
            if profiles != Some((step.segment, step.right)) {
                profiles = Some((step.segment, step.right));
                let left_at = bracket(&modes[step.segment], depth);
                let right_at = bracket(&modes[step.right], depth);
                (base, slope) = modes[step.segment]
                    .modes
                    .iter()
                    .zip(&modes[step.right].modes)
                    .map(|(left, right)| {
                        let left = sample_at(left, left_at);
                        (double(left), double(sample_at(right, right_at) - left))
                    })
                    .unzip();
            }
            let contributions = base
                .iter()
                .zip(&slope)
                .zip(&step.terms)
                .map(|((&base, &slope), term)| double(single(base + step.w * slope)) * term);
            let value = if incoherent {
                Complex64::new(contributions.map(|p| p.norm_sqr()).sum::<f64>().sqrt(), 0.0)
            } else {
                contributions.sum()
            };
            double(single(value))
        })
        .collect()
}

fn adiabatic(
    case: &FieldCase,
    modes: &[ModeSet],
    excitation: &[Complex32],
    pressure: &mut [Complex64],
    table_values: usize,
) -> Result<(), DiagnosticReport> {
    let first = &case.profiles[0];
    let incoherent = first.mode_addition == ModeAddition::Incoherent;
    let mut count = excitation.len().min(modes[1].modes.len());
    let constants: Vec<_> = excitation.iter().map(|s| double(factor() * s)).collect();
    let mut integral = vec![Complex64::new(0.0, 0.0); excitation.len()];
    let mut segment = 0;
    let mut previous = 0.0_f64;
    let range_count = first.receiver_ranges_m.len();
    let block_len = (table_values / excitation.len().max(1)).clamp(1, range_count.max(1));
    for (block_index, ranges) in first.receiver_ranges_m.chunks(block_len).enumerate() {
        // The phase integral runs on from range to range, so the march is sequential.
        let mut steps = Vec::with_capacity(ranges.len());
        for &range in ranges {
            crate::cancellation::check()?;
            let mut left = previous.max(case.ranges_m[segment]);
            while segment + 1 < modes.len() && range > case.ranges_m[segment + 1] {
                let boundary = case.ranges_m[segment + 1];
                integrate(
                    modes,
                    &case.ranges_m,
                    segment,
                    left,
                    boundary,
                    &mut integral[..count],
                );
                left = boundary;
                segment += 1;
                if segment + 1 < modes.len() {
                    count = count.min(modes[segment + 1].modes.len());
                }
            }
            integrate(
                modes,
                &case.ranges_m,
                segment,
                left,
                range,
                &mut integral[..count],
            );
            let right = (segment + 1).min(modes.len() - 1);
            let w = if right == segment {
                0.0
            } else {
                (range - case.ranges_m[segment]) / (case.ranges_m[right] - case.ranges_m[segment])
            };
            steps.push(AdiabaticRange {
                segment,
                right,
                w,
                range,
                terms: integral[..count].to_vec(),
            });
            previous = range;
        }
        let steps = crate::parallelism::map(steps.len(), |index| {
            steps[index].with_factors(first, modes, &constants)
        });
        let block = (block_index * block_len, ranges.len());
        fill_rows(
            pressure,
            &first.receiver_depths_m,
            range_count,
            block,
            table_values,
            |depth| adiabatic_row(&steps, modes, depth, incoherent),
        )?;
    }
    Ok(())
}

fn integrate(
    modes: &[ModeSet],
    ranges: &[f64],
    segment: usize,
    left: f64,
    right: f64,
    integral: &mut [Complex64],
) {
    let next = (segment + 1).min(modes.len() - 1);
    let w = if next == segment {
        0.0
    } else {
        (right.midpoint(left) - ranges[segment]) / (ranges[next] - ranges[segment])
    };
    for (m, sum) in integral.iter_mut().enumerate() {
        let k = interpolate(
            single(modes[segment].modes[m].horizontal_wavenumber_rad_per_m),
            single(modes[next].modes[m].horizontal_wavenumber_rad_per_m),
            w,
        );
        *sum += double(k) * (right - left);
    }
}

/// One receiver range of the coupled march: its profile and mode amplitudes.
struct CoupledRange {
    segment: usize,
    range: f64,
    amplitudes: Vec<Complex32>,
}

#[allow(clippy::float_cmp)]
fn coupled(
    case: &FieldCase,
    modes: &[ModeSet],
    excitation: &[Complex32],
    pressure: &mut [Complex64],
    table_values: usize,
) -> Result<(), DiagnosticReport> {
    let first = &case.profiles[0];
    let mut a: Vec<_> = excitation
        .iter()
        .zip(&modes[0].modes)
        .map(|(s, m)| {
            let k = single(m.horizontal_wavenumber_rad_per_m);
            if first.source_geometry == SourceGeometry::Line {
                factor() / Complex32::new(0.0, 1.0) * s / k
            } else {
                factor() * s / k.sqrt()
            }
        })
        .collect();
    let mut segment = 0;
    let mut previous = 0.0;
    let range_count = first.receiver_ranges_m.len();
    let largest = modes.iter().map(|m| m.modes.len()).max().unwrap_or(0);
    let block_len = (table_values / largest.max(1)).clamp(1, range_count.max(1));
    for (block_index, ranges) in first.receiver_ranges_m.chunks(block_len).enumerate() {
        // Each range advances the previous one's amplitudes, so the march is sequential.
        let mut steps = Vec::with_capacity(ranges.len());
        for &range in ranges {
            crate::cancellation::check()?;
            while segment + 1 < modes.len()
                && range > case.ranges_m[segment].midpoint(case.ranges_m[segment + 1])
            {
                let boundary = case.ranges_m[segment].midpoint(case.ranges_m[segment + 1]);
                advance(&mut a, &modes[segment], boundary - previous);
                a = project(
                    &case.profiles[segment],
                    &case.profiles[segment + 1],
                    &modes[segment],
                    &modes[segment + 1],
                    &a,
                )?;
                previous = boundary;
                segment += 1;
            }
            advance(&mut a, &modes[segment], range - previous);
            steps.push(CoupledRange {
                segment,
                range,
                amplitudes: a[..a.len().min(modes[segment].modes.len())].to_vec(),
            });
            previous = range;
        }
        let block = (block_index * block_len, ranges.len());
        fill_rows(
            pressure,
            &first.receiver_depths_m,
            range_count,
            block,
            table_values,
            |depth| {
                let mut profile = None;
                let mut shapes = Vec::new();
                steps
                    .iter()
                    .map(|step| {
                        if profile != Some(step.segment) {
                            profile = Some(step.segment);
                            let at = bracket(&modes[step.segment], depth);
                            shapes = modes[step.segment]
                                .modes
                                .iter()
                                .map(|mode| sample_at(mode, at))
                                .collect();
                        }
                        let sum: Complex32 = step
                            .amplitudes
                            .iter()
                            .zip(&shapes)
                            .map(|(a, shape)| a * shape)
                            .sum();
                        let value = if first.source_geometry == SourceGeometry::Point
                            && step.range != 0.0
                        {
                            single(double(sum) / step.range.sqrt())
                        } else {
                            sum
                        };
                        double(value)
                    })
                    .collect()
            },
        )?;
    }
    Ok(())
}

fn advance(a: &mut [Complex32], modes: &ModeSet, distance: f64) {
    for (a, m) in a.iter_mut().zip(&modes.modes) {
        let ik = Complex32::new(0.0, -1.0) * single(m.horizontal_wavenumber_rad_per_m);
        *a = single(double(*a) * (double(ik) * distance).exp());
    }
}

#[allow(clippy::approx_constant, clippy::cast_possible_truncation)]
fn gamma(case: &Case, mode: &crate::NormalMode, top: bool) -> Complex32 {
    let boundary = if top {
        &case.surface_boundary
    } else {
        &case.bottom_boundary
    };
    if !boundary.is_half_space() {
        return Complex32::new(0.0, 0.0);
    }
    let (cp, loss) = if top {
        (
            case.surface_sound_speed_mps,
            case.surface_attenuation_db_per_wavelength,
        )
    } else {
        (
            case.bottom_sound_speed_mps,
            case.bottom_attenuation_db_per_wavelength,
        )
    };
    let cp = single(complex_speed(cp, loss));
    let kb = single(
        (Complex64::new(2.0 * f64::from(3.141_592_6_f32) * case.frequency_hz, 0.0) / double(cp))
            .powi(2),
    );
    let k = single(mode.horizontal_wavenumber_rad_per_m);
    single(crate::solver::complex_modes::pekeris_root(double(
        k * k - kb,
    )))
}

#[allow(
    clippy::cast_possible_truncation,
    clippy::too_many_lines,
    clippy::manual_midpoint
)] // preserve PLeft's f32 density-quadrature grouping
fn project(
    left: &Case,
    right: &Case,
    lm: &ModeSet,
    rm: &ModeSet,
    a: &[Complex32],
) -> Result<Vec<Complex32>, DiagnosticReport> {
    let mut z: Vec<_> = rm.sampled_depths_m.iter().map(|&z| z as f32).collect();
    for &depth in &lm.sampled_depths_m {
        if depth as f32 > *z.last().unwrap_or(&0.0) {
            z.push(depth as f32);
        }
    }
    let pl: Vec<Complex32> = (0..lm.sampled_depths_m.len())
        .map(|iz| {
            a.iter()
                .zip(&lm.modes)
                .map(|(a, m)| a * single(m.eigenfunction[iz]))
                .sum()
        })
        .collect();
    let lz: Vec<_> = lm.sampled_depths_m.iter().map(|&z| z as f32).collect();
    let bottom = right.fluid_bottom_depth_m() as f32;
    let top = right.fluid_top_depth_m() as f32;
    let mut medium = 0;
    let layers: Vec<_> = crate::solver::layers::iter(right).collect();
    let mut density = layers[0].density as f32;
    let mut p = Vec::with_capacity(z.len());
    for (iz, &depth) in z.iter().enumerate() {
        crate::cancellation::check()?;
        if depth < top {
            density = right.surface_density_g_cm3 as f32;
        } else if depth > bottom {
            density = right.bottom_density_g_cm3 as f32;
        } else if medium + 1 < layers.len() && depth > layers[medium].bottom as f32 {
            medium += 1;
            density = layers[medium].density as f32;
        }
        let below = layers[medium].bottom as f32;
        let density_below = layers
            .get(medium + 1)
            .map_or(right.bottom_density_g_cm3 as f32, |l| l.density as f32);
        let value = if (depth > left.fluid_bottom_depth_m() as f32
            && !right.bottom_boundary.is_half_space())
            || (depth < left.fluid_top_depth_m() as f32 && !right.surface_boundary.is_half_space())
        {
            Complex32::new(0.0, 0.0)
        } else if depth > left.fluid_bottom_depth_m() as f32 {
            a.iter()
                .zip(&lm.modes)
                .map(|(a, m)| {
                    a * single(*m.eigenfunction.last().unwrap())
                        * (-gamma(left, m, false) * (depth - left.fluid_bottom_depth_m() as f32))
                            .exp()
                })
                .sum()
        } else if depth < left.fluid_top_depth_m() as f32 {
            a.iter()
                .zip(&lm.modes)
                .map(|(a, m)| {
                    a * single(m.eigenfunction[0])
                        * (-gamma(left, m, true) * (left.fluid_top_depth_m() as f32 - depth)).exp()
                })
                .sum()
        } else {
            let upper = lz.partition_point(|&d| d < depth).clamp(1, lz.len() - 1);
            pl[upper - 1]
                + (depth - lz[upper - 1]) / (lz[upper] - lz[upper - 1])
                    * (pl[upper] - pl[upper - 1])
        };
        let h = if iz == 0 {
            0.5 * (z[1] - z[0]) / density
        } else if iz + 1 == z.len() {
            0.5 * (depth - z[iz - 1]) / density
        } else if z[iz - 1] < below && z[iz + 1] > below {
            0.5 * (z[iz + 1] / density_below - z[iz - 1] / density - below / density_below
                + below / density)
        } else {
            0.5 * (z[iz + 1] - z[iz - 1]) / density
        };
        p.push(h * value);
    }
    rm.modes
        .iter()
        .map(|mode| {
            crate::cancellation::check()?;
            let mut sum: Complex32 = p
                .iter()
                .enumerate()
                .map(|(iz, p)| {
                    let phi = if iz < mode.eigenfunction.len() {
                        single(mode.eigenfunction[iz])
                    } else if right.bottom_boundary.is_half_space() {
                        single(*mode.eigenfunction.last().unwrap())
                            * (-gamma(right, mode, false) * (z[iz] - bottom)).exp()
                    } else {
                        Complex32::new(0.0, 0.0)
                    };
                    p * phi
                })
                .sum();
            for (top_side, boundary, density, depth) in [
                (
                    true,
                    &right.surface_boundary,
                    right.surface_density_g_cm3,
                    right.fluid_top_depth_m(),
                ),
                (
                    false,
                    &right.bottom_boundary,
                    right.bottom_density_g_cm3,
                    right.fluid_bottom_depth_m(),
                ),
            ] {
                if boundary.is_half_space() {
                    let edge = if top_side { 0 } else { z.len() - 1 };
                    let phi_r = single(if top_side {
                        mode.eigenfunction[0]
                    } else {
                        *mode.eigenfunction.last().unwrap()
                    });
                    let gr = gamma(right, mode, top_side);
                    let fr = phi_r / density as f32 * (-gr * (z[edge] - depth as f32)).exp();
                    let dl = if top_side {
                        left.fluid_top_depth_m()
                    } else {
                        left.fluid_bottom_depth_m()
                    } as f32;
                    let tail: Complex32 = a
                        .iter()
                        .zip(&lm.modes)
                        .map(|(a, m)| {
                            let gl = gamma(left, m, top_side);
                            let phi_l = single(if top_side {
                                m.eigenfunction[0]
                            } else {
                                *m.eigenfunction.last().unwrap()
                            });
                            a * phi_l * (-gl * (z[edge] - dl)).exp() / (gl + gr)
                        })
                        .sum();
                    sum += fr * tail;
                }
            }
            Ok(sum)
        })
        .collect()
}

#[cfg(test)]
#[test]
fn table_size_never_changes_the_range_dependent_field() {
    let fixtures = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    for (stem, addition, geometry) in [
        ("ProfilesAd", ModeAddition::Coherent, SourceGeometry::Point),
        ("ProfilesAd", ModeAddition::Incoherent, SourceGeometry::Line),
        (
            "ProfilesAd",
            ModeAddition::Coherent,
            SourceGeometry::ScaledCylindrical,
        ),
        ("ProfilesCm", ModeAddition::Coherent, SourceGeometry::Point),
        ("ProfilesCm", ModeAddition::Coherent, SourceGeometry::Line),
    ] {
        let root = fixtures.join(stem);
        let field = crate::legacy::load_field_cases(
            root.with_extension("env"),
            root.with_extension("flp"),
            crate::ModeSolver::Kraken,
        )
        .unwrap()
        .remove(0);
        let profiles = field
            .profiles
            .iter()
            .map(|profile| {
                let mut input = profile.clone().into_definition();
                input.source_depths_m = vec![75.0, 25.0, 50.0];
                input.mode_addition = addition;
                input.source_geometry = geometry;
                Case::from_definition(input).unwrap()
            })
            .collect();
        let case = FieldCase::new(profiles, field.ranges_m, field.propagation).unwrap();
        let expected = solve_field(&case).unwrap();
        let largest = expected.modes.iter().map(|m| m.modes.len()).max().unwrap();
        // One value per table puts each range and each receiver depth in a block of its own.
        for table_values in [1, 2, largest, 3 * largest + 1] {
            let pressure = synthesize(
                &case,
                &expected.modes,
                expected.field.pressure.len(),
                table_values,
            );
            assert_eq!(
                format!("{pressure:?}"),
                format!("{:?}", Ok::<_, DiagnosticReport>(&expected.field.pressure)),
                "{stem} {addition:?} {geometry:?} with {table_values}-value tables"
            );
        }
    }
}

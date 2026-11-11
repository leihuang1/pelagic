//! Range-dependent 2D FIELD propagation.
use super::{FIELD_RANGE_LANES, MIN_LANE_RANGES};
use crate::case::MAX_SEQUENCE_VALUES;
use crate::solver::{double, elastic::complex_speed, error, single, source_pattern_scale};
use crate::{
    Case, DiagnosticReport, FieldCase, FieldPropagation, ModeAddition, ModeSet, PressureField,
    ProfileSimulationResult, SourceGeometry,
};
use num_complex::{Complex32, Complex64};
use std::num::NonZeroUsize;

// Range factors, mode amplitudes and finished rows are held for blocks of at
// most this many values (1 MiB of complex doubles), so memory stays near a
// direct march; larger tables gain little.
const TABLE_VALUES: usize = 1 << 16;

// Below this many threads, one solve's own parallelism beats solving side by side,
// where the slowest solve holds up its whole window. Measured, not fundamental.
const SOLVE_WINDOW_THREADS: usize = 8;
// Solves per thread in one window: a surplus lets faster cores take more of them.
const SOLVES_PER_THREAD: usize = 4;
// Finished results one window may hold, judging each by twice the largest so far.
// A later result can be larger, so this limits speculation, not memory.
const SOLVE_AHEAD_BYTES: usize = 128 * 1024 * 1024;

fn mode_set_bytes(set: &ModeSet) -> usize {
    size_of::<ModeSet>()
        + set.sampled_depths_m.len() * size_of::<f64>()
        + set.modes.len()
            * (size_of::<crate::NormalMode>() + set.sampled_depths_m.len() * size_of::<Complex64>())
}

fn field_result_bytes(result: &ProfileSimulationResult) -> usize {
    result.modes.iter().map(mode_set_bytes).sum::<usize>()
        + result.field.pressure.len() * size_of::<Complex64>()
}

fn independent_profiles(case: &FieldCase) -> bool {
    !case.profiles.iter().any(crate::solver::carries_mode_count)
}

// With nothing solved yet, one solve gets every thread: its size is unknown, and
// the first result then arrives as soon as it can.
fn solve_window_len(remaining: usize, independent: bool, largest: Option<usize>) -> usize {
    let threads = crate::parallelism::threads().min(crate::parallelism::MAX_WORKERS);
    let admitted = match largest {
        Some(bytes) if independent && threads >= SOLVE_WINDOW_THREADS => (threads
            * SOLVES_PER_THREAD)
            .min((SOLVE_AHEAD_BYTES / bytes.saturating_mul(2).max(1)).max(1)),
        _ => 1,
    };
    remaining.min(admitted)
}

/// Divide this scope's thread budget between `count` solves, in order. With more
/// solves than threads, each has one thread and the workers take them as they free up.
fn solve_window<T: Send>(count: usize, work: impl Fn(usize) -> T + Sync) -> Vec<T> {
    let threads = crate::parallelism::threads().min(crate::parallelism::MAX_WORKERS);
    // Whole solves are never tiny, and each one gets its share even alone.
    crate::parallelism::map(count, usize::MAX, |index| {
        let share = threads / count + usize::from(index < threads % count);
        let share = NonZeroUsize::new(share).unwrap_or(NonZeroUsize::MIN);
        crate::parallelism::with_threads(share, || work(index))
    })
}

/// Compute every profile's modes and synthesize one range-dependent field.
/// # Errors
/// Returns modal, cumulative storage/work or non-finite pressure diagnostics.
pub fn solve_field(case: &FieldCase) -> Result<ProfileSimulationResult, DiagnosticReport> {
    let mut root_limit = crate::MAX_MODE_LIMIT;
    solve_field_with_limit(case, &mut root_limit)
}

/// Solve ordered frequency blocks, stopping after the first failed block.
/// Blocks must share one mode solver; a mismatch yields `KR0201` without solving it.
/// With enough threads, independent blocks after the first solve together on demand,
/// sharing the budget, so a failure or cancellation can discard later blocks already
/// solved.
/// Real finite-elastic KRAKEN stays sequential, retaining Solve2's search bound, and
/// so, conservatively, does elastic-top KRAKEN; separate calls to `solve_field` start
/// separate reference runs.
#[allow(clippy::missing_panics_doc)] // FieldCase construction guarantees a nonempty sequence
pub fn solve_frequencies(
    cases: &[FieldCase],
) -> impl Iterator<Item = Result<ProfileSimulationResult, DiagnosticReport>> + '_ {
    let independent = cases.iter().all(independent_profiles);
    let mut root_limit = crate::MAX_MODE_LIMIT;
    let mut largest_bytes = None;
    let mut next = 0;
    let mut pending = Vec::new().into_iter();
    let mut stopped = false;
    std::iter::from_fn(move || {
        if stopped || next == cases.len() {
            return None;
        }
        if cases[next].profiles[0].mode_solver == cases[0].profiles[0].mode_solver
            && let Err(report) = crate::cancellation::check()
        {
            stopped = true;
            pending = Vec::new().into_iter();
            return Some(Err(report));
        }
        if pending.len() == 0 {
            let count = solve_window_len(cases.len() - next, independent, largest_bytes);
            pending = solve_window(count, |offset| {
                let case = &cases[next + offset];
                if case.profiles[0].mode_solver != cases[0].profiles[0].mode_solver {
                    return Err(error(
                        "KR0201",
                        "frequency blocks must share the same mode solver",
                        "mode_solver",
                    ));
                }
                let mut limit = root_limit;
                solve_field_with_limit(case, &mut limit)
            })
            .into_iter();
        }
        next += 1;
        let result = pending.next()?.and_then(|result| {
            // Cancellation can also arrive while the window was solving.
            crate::cancellation::check()?;
            root_limit = result.modes.last().unwrap().modes.len();
            largest_bytes = largest_bytes.max(Some(field_result_bytes(&result)));
            Ok(result)
        });
        if result.is_err() {
            stopped = true;
            pending = Vec::new().into_iter();
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
    let independent = independent_profiles(case);
    let mut modes = Vec::with_capacity(case.profiles.len());
    let mut shapes = 0;
    let mut largest_bytes = None;
    while modes.len() < case.profiles.len() {
        let start = modes.len();
        let window = solve_window_len(case.profiles.len() - start, independent, largest_bytes);
        let sets = solve_window(window, |offset| {
            crate::solver::solve_modes(&case.profiles[start + offset], *root_limit)
        });
        for (offset, set) in sets.into_iter().enumerate() {
            let set = set.map_err(|mut report| {
                let index = start + offset;
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
            largest_bytes = largest_bytes.max(Some(mode_set_bytes(&set)));
            modes.push(set);
        }
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
fn fill_rows<R: IntoIterator<Item = Complex64>>(
    pressure: &mut [Complex64],
    depths: &[f64],
    range_count: usize,
    (start, len): (usize, usize),
    table_values: usize,
    mode_count: usize,
    row: impl Fn(f64) -> R + Sync,
) -> Result<(), DiagnosticReport> {
    let batch = (table_values / len).max(1);
    // Two profile samples and one modal sum at every range.
    let row_work = len.saturating_mul(mode_count.max(1)).saturating_mul(3);
    for (batch_index, depths) in depths.chunks(batch).enumerate() {
        if crate::parallelism::threads() == 1 {
            let mut result = Ok(());
            for (offset, &depth) in depths.iter().enumerate() {
                if let Err(report) = crate::cancellation::check() {
                    // Like the map, check the whole batch and report its first error.
                    result = result.and(Err(report));
                    continue;
                }
                let iz = batch_index * batch + offset;
                let output = &mut pressure[iz * range_count + start..][..len];
                for (value, pressure) in row(depth).into_iter().zip(output) {
                    *pressure = value;
                }
            }
            result?;
            continue;
        }
        let rows = crate::parallelism::map(
            depths.len(),
            depths.len().saturating_mul(row_work),
            |index| {
                crate::cancellation::check()?;
                Ok(row(depths[index]).into_iter().collect::<Vec<_>>())
            },
        );
        for (offset, values) in rows.into_iter().enumerate() {
            let iz = batch_index * batch + offset;
            pressure[iz * range_count + start..][..len].copy_from_slice(&values?);
        }
    }
    Ok(())
}

/// One same-profile group; padding is charged to the table, not the output.
struct RangeGroup {
    start: usize,
    len: usize,
    segment: usize,
    lanes: bool,
}

fn range_blocks<'a>(
    ranges: &'a [f64],
    boundaries: &'a [f64],
    capacity: usize,
) -> impl Iterator<Item = Vec<RangeGroup>> + 'a {
    let mut start = 0;
    std::iter::from_fn(move || {
        if start == ranges.len() {
            return None;
        }
        let mut remaining = capacity.max(1);
        let mut groups = Vec::new();
        while start < ranges.len() && remaining > 0 {
            let segment = boundaries.partition_point(|&boundary| ranges[start] > boundary);
            let len = boundaries
                .get(segment)
                .map_or(ranges.len() - start, |&boundary| {
                    ranges[start..].partition_point(|&range| range <= boundary)
                })
                .min(FIELD_RANGE_LANES)
                .min(remaining);
            let lanes = len >= MIN_LANE_RANGES && remaining >= FIELD_RANGE_LANES;
            groups.push(RangeGroup {
                start,
                len,
                segment,
                lanes,
            });
            remaining -= if lanes { FIELD_RANGE_LANES } else { len };
            start += len;
        }
        Some(groups)
    })
}

/// Coupled amplitudes have one layout, never a scalar table plus its transpose.
enum RangeValues {
    Scalar(Vec<Complex32>),
    Lanes(Vec<super::RangePhaseLanes>),
}

impl RangeValues {
    fn new(group: &RangeGroup, modes: usize) -> Self {
        if group.lanes {
            Self::Lanes(
                (0..modes)
                    .map(|_| super::RangePhaseLanes {
                        re: [0.0; FIELD_RANGE_LANES],
                        im: [0.0; FIELD_RANGE_LANES],
                    })
                    .collect(),
            )
        } else {
            Self::Scalar(Vec::with_capacity(group.len * modes))
        }
    }

    fn push_range(&mut self, lane: usize, values: impl Iterator<Item = Complex32>) {
        match self {
            Self::Scalar(table) => table.extend(values),
            Self::Lanes(table) => {
                for (mode, value) in table.iter_mut().zip(values) {
                    mode.re[lane] = value.re;
                    mode.im[lane] = value.im;
                }
            }
        }
    }
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
fn adiabatic_row<'a>(
    steps: &'a [AdiabaticRange],
    modes: &'a [ModeSet],
    depth: f64,
    incoherent: bool,
) -> impl Iterator<Item = Complex64> + 'a {
    // Each mode's shape at this depth in both profiles, as interpolate() takes them.
    let mut profiles = None;
    let (mut base, mut slope) = (Vec::new(), Vec::new());
    steps.iter().map(move |step| {
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
    let largest = modes.iter().map(|m| m.modes.len()).max().unwrap_or(0);
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
        let steps = crate::parallelism::map(
            steps.len(),
            steps.len().saturating_mul(excitation.len()),
            |index| steps[index].with_factors(first, modes, &constants),
        );
        let block = (block_index * block_len, ranges.len());
        fill_rows(
            pressure,
            &first.receiver_depths_m,
            range_count,
            block,
            table_values,
            largest,
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

/// One receiver depth's coupled pressure, retaining f32 modal addition order.
#[allow(clippy::float_cmp)]
fn coupled_row(
    groups: &[RangeGroup],
    amplitudes: &[RangeValues],
    modes: &[ModeSet],
    first: &Case,
    depth: f64,
) -> Vec<Complex64> {
    let mut profile = None;
    let mut shapes = Vec::new();
    let mut values = Vec::with_capacity(groups.iter().map(|g| g.len).sum());
    let finish = |sum: Complex32, range: f64| {
        double(
            if first.source_geometry == SourceGeometry::Point && range != 0.0 {
                single(double(sum) / range.sqrt())
            } else {
                sum
            },
        )
    };
    for (group, amplitudes) in groups.iter().zip(amplitudes) {
        if profile != Some(group.segment) {
            profile = Some(group.segment);
            let set = &modes[group.segment];
            let at = bracket(set, depth);
            shapes = set.modes.iter().map(|mode| sample_at(mode, at)).collect();
        }
        let ranges = &first.receiver_ranges_m[group.start..][..group.len];
        match amplitudes {
            RangeValues::Scalar(table) => {
                let count = table.len() / group.len;
                for (i, &range) in ranges.iter().enumerate() {
                    let terms = &table[i * count..(i + 1) * count];
                    let sum = terms.iter().zip(&shapes).map(|(a, shape)| a * shape).sum();
                    values.push(finish(sum, range));
                }
            }
            RangeValues::Lanes(table) => {
                let mut re = [0.0; FIELD_RANGE_LANES];
                let mut im = [0.0; FIELD_RANGE_LANES];
                for (a, shape) in table.iter().zip(&shapes) {
                    for lane in 0..FIELD_RANGE_LANES {
                        let contribution = Complex32::new(a.re[lane], a.im[lane]) * shape;
                        re[lane] += contribution.re;
                        im[lane] += contribution.im;
                    }
                }
                for (lane, &range) in ranges.iter().enumerate() {
                    values.push(finish(Complex32::new(re[lane], im[lane]), range));
                }
            }
        }
    }
    values
}

#[allow(clippy::too_many_lines)]
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
    let boundaries: Vec<_> = case
        .ranges_m
        .windows(2)
        .map(|r| r[0].midpoint(r[1]))
        .collect();
    let capacity = (table_values.min(super::PHASE_TABLE_VALUES) / largest.max(1)).max(1);
    for groups in range_blocks(&first.receiver_ranges_m, &boundaries, capacity) {
        // Write each marched amplitude directly into its final scalar/lane layout.
        let mut amplitudes = Vec::with_capacity(groups.len());
        for group in &groups {
            let ranges = &first.receiver_ranges_m[group.start..][..group.len];
            crate::cancellation::check()?;
            while segment < group.segment {
                let boundary = boundaries[segment];
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
            let count = a.len().min(modes[segment].modes.len());
            let mut table = RangeValues::new(group, count);
            for (lane, &range) in ranges.iter().enumerate() {
                crate::cancellation::check()?;
                advance(&mut a, &modes[segment], range - previous);
                table.push_range(lane, a[..count].iter().copied());
                previous = range;
            }
            amplitudes.push(table);
        }
        let block = (groups[0].start, groups.iter().map(|g| g.len).sum());
        fill_rows(
            pressure,
            &first.receiver_depths_m,
            range_count,
            block,
            table_values,
            largest,
            |depth| coupled_row(&groups, &amplitudes, modes, first, depth),
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

#[cfg(test)]
mod range_lane_tests {
    use super::*;

    fn bits(values: &[Complex64]) -> Vec<(u64, u64)> {
        values
            .iter()
            .map(|p| (p.re.to_bits(), p.im.to_bits()))
            .collect()
    }

    #[test]
    fn direct_and_collected_rows_keep_pressure_bits_and_block_offsets() {
        let depths = [1.0, 2.0, 3.0, 4.0, 5.0];
        let mut expected = vec![Complex64::new(-1.0, -0.0); 7 * depths.len()];
        for (iz, depth) in depths.into_iter().enumerate() {
            for ir in 0..3 {
                expected[iz * 7 + 2 + ir] =
                    Complex64::new(depth, f64::from(u32::try_from(ir).unwrap()));
            }
        }
        for threads in [1, 4] {
            for budget in [1, 7, TABLE_VALUES] {
                let mut pressure = vec![Complex64::new(-1.0, -0.0); expected.len()];
                crate::parallelism::with_threads(NonZeroUsize::new(threads).unwrap(), || {
                    fill_rows(
                        &mut pressure,
                        &depths,
                        7,
                        (2, 3),
                        budget,
                        usize::MAX,
                        |depth| (0..3).map(move |ir| Complex64::new(depth, f64::from(ir))),
                    )
                    .unwrap();
                });
                assert_eq!(bits(&pressure), bits(&expected));
            }
        }
    }

    #[test]
    fn direct_and_collected_rows_keep_cancellation_before_the_next_row() {
        use std::sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        };
        for threads in [1, 4] {
            let token = Arc::new(AtomicBool::new(false));
            let mut pressure = vec![Complex64::new(-1.0, 0.0); 6];
            let report = crate::cancellation::with_token(token.clone(), || {
                crate::parallelism::with_threads(NonZeroUsize::new(threads).unwrap(), || {
                    // Tiny work stays inline even with a parallel budget.
                    fill_rows(
                        &mut pressure,
                        &[1.0, 2.0, 3.0],
                        2,
                        (0, 2),
                        TABLE_VALUES,
                        1,
                        |depth| {
                            token.store(true, Ordering::Relaxed);
                            [Complex64::new(depth, 0.0); 2]
                        },
                    )
                })
            })
            .unwrap_err();
            assert_eq!(report.diagnostics()[0].code, "KR0304");
            let mut expected = vec![Complex64::new(-1.0, 0.0); 6];
            expected[..2].fill(Complex64::new(1.0, 0.0));
            assert_eq!(bits(&pressure), bits(&expected));
            assert!(!crate::cancellation::cancelled());
        }
    }

    #[test]
    fn groups_charge_padding_and_keep_short_segments_scalar() {
        let boundaries = [6.0, 14.0, 23.0, 39.0, 56.0];
        let ranges: Vec<_> = (0..74).map(f64::from).collect();
        let groups = range_blocks(&ranges, &boundaries, 128).next().unwrap();
        assert_eq!(
            groups.iter().map(|g| (g.len, g.lanes)).collect::<Vec<_>>(),
            [
                (7, false),
                (8, true),
                (9, true),
                (16, true),
                (16, true),
                (1, false),
                (16, true),
                (1, false)
            ]
        );
        for capacity in [1, 2, 7, 8, 15, 16, 17, 23, 31, 32, 33, 128] {
            let mut next = 0;
            for groups in range_blocks(&ranges, &boundaries, capacity) {
                let charged: usize = groups
                    .iter()
                    .map(|g| if g.lanes { FIELD_RANGE_LANES } else { g.len })
                    .sum();
                assert!(charged <= capacity);
                for group in groups {
                    assert_eq!(group.start, next);
                    assert!(group.len > 0 && group.len <= FIELD_RANGE_LANES);
                    assert!(!group.lanes || group.len >= MIN_LANE_RANGES);
                    for &range in &ranges[group.start..][..group.len] {
                        assert_eq!(group.segment, boundaries.partition_point(|&b| range > b));
                    }
                    next += group.len;
                }
            }
            assert_eq!(next, ranges.len());
        }
    }

    #[test]
    fn scalar_and_lane_fields_are_bitwise_identical() {
        let fixtures = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
        for stem in ["ProfilesAd", "ProfilesCm"] {
            let root = fixtures.join(stem);
            let field = crate::legacy::load_field_cases(
                root.with_extension("env"),
                root.with_extension("flp"),
                crate::ModeSolver::Kraken,
            )
            .unwrap()
            .remove(0);
            let solved = solve_field(&field).unwrap();
            let boundaries = if field.propagation == FieldPropagation::Adiabatic {
                field.ranges_m[1..].to_vec()
            } else {
                field
                    .ranges_m
                    .windows(2)
                    .map(|r| r[0].midpoint(r[1]))
                    .collect()
            };
            for len in 1..=17 {
                let mut ranges = Vec::new();
                let mut left = 0.0;
                for segment in 0..field.profiles.len() {
                    let right = boundaries.get(segment).copied().unwrap_or(left + 1000.0);
                    for i in 1..=len {
                        ranges.push(if segment == 0 && i == 1 {
                            0.0
                        } else if i == len {
                            right
                        } else {
                            left + (right - left) * f64::from(i) / f64::from(len)
                        });
                    }
                    left = right;
                }
                for (addition, geometry) in [
                    (ModeAddition::Coherent, SourceGeometry::Point),
                    (ModeAddition::Coherent, SourceGeometry::Line),
                    (ModeAddition::Coherent, SourceGeometry::ScaledCylindrical),
                    (ModeAddition::Incoherent, SourceGeometry::Point),
                    (ModeAddition::Incoherent, SourceGeometry::Line),
                ] {
                    if field.propagation == FieldPropagation::Coupled
                        && addition == ModeAddition::Incoherent
                    {
                        continue;
                    }
                    let profiles = field
                        .profiles
                        .iter()
                        .map(|profile| {
                            let mut input = profile.clone().into_definition();
                            input.receiver_ranges_m.clone_from(&ranges);
                            input.mode_addition = addition;
                            input.source_geometry = geometry;
                            Case::from_definition(input).unwrap()
                        })
                        .collect();
                    let case = FieldCase::new(profiles, field.ranges_m.clone(), field.propagation)
                        .unwrap();
                    for count in [0, 1, usize::MAX] {
                        let mut modes = solved.modes.clone();
                        for set in &mut modes {
                            set.modes.truncate(count);
                        }
                        let largest = modes.iter().map(|m| m.modes.len()).max().unwrap().max(1);
                        let samples = ranges.len() * case.profiles[0].receiver_depths_m.len();
                        let scalar = synthesize(&case, &modes, samples, 1).unwrap();
                        for budget in [
                            7 * largest,
                            8 * largest,
                            15 * largest,
                            16 * largest,
                            17 * largest,
                            31 * largest,
                            32 * largest + 1,
                            TABLE_VALUES,
                            super::super::PHASE_TABLE_VALUES + 1,
                        ] {
                            let lanes = synthesize(&case, &modes, samples, budget).unwrap();
                            assert_eq!(
                                bits(&scalar),
                                bits(&lanes),
                                "{stem} {addition:?} {geometry:?} {len} ranges/segment, {count} modes, {budget} values"
                            );
                        }
                    }
                }
            }
        }
    }

    /// The coupled field as `57f55a3` computed it, one receiver range at a time,
    /// crossing every profile midpoint below a range first. It shares only the
    /// unchanged helpers with `coupled`, so it checks the grouped march itself.
    #[allow(clippy::float_cmp)]
    fn sequential_coupled(case: &FieldCase, modes: &[ModeSet], samples: usize) -> Vec<Complex64> {
        let first = &case.profiles[0];
        let ranges = &first.receiver_ranges_m;
        let mut pressure = vec![Complex64::new(0.0, 0.0); samples];
        for (source, &source_depth) in first.source_depths_m.iter().enumerate() {
            let count = modes[0].modes.len().min(first.mode_limit);
            let excitation: Vec<_> = modes[0]
                .modes
                .iter()
                .take(count)
                .map(|mode| {
                    let shape = sample(&modes[0], mode, source_depth);
                    if source == 0 && !first.source_pattern.is_empty() {
                        shape
                            * source_pattern_scale(
                                first,
                                single(mode.horizontal_wavenumber_rad_per_m),
                            )
                    } else {
                        shape
                    }
                })
                .collect();
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
            let (mut segment, mut previous) = (0, 0.0);
            let mut steps = Vec::new();
            for &range in ranges {
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
                    )
                    .unwrap();
                    previous = boundary;
                    segment += 1;
                }
                advance(&mut a, &modes[segment], range - previous);
                let kept = a.len().min(modes[segment].modes.len());
                steps.push((segment, range, a[..kept].to_vec()));
                previous = range;
            }
            let block = source * samples / first.source_depths_m.len();
            for (iz, &depth) in first.receiver_depths_m.iter().enumerate() {
                for (ir, (segment, range, amplitudes)) in steps.iter().enumerate() {
                    let at = bracket(&modes[*segment], depth);
                    let sum: Complex32 = amplitudes
                        .iter()
                        .zip(&modes[*segment].modes)
                        .map(|(a, mode)| a * sample_at(mode, at))
                        .sum();
                    let value = if first.source_geometry == SourceGeometry::Point && *range != 0.0 {
                        single(double(sum) / range.sqrt())
                    } else {
                        sum
                    };
                    pressure[block + iz * ranges.len() + ir] = double(value);
                }
            }
        }
        pressure
    }

    #[test]
    fn coupled_field_matches_the_sequential_march_when_receivers_skip_profiles() {
        let root =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/ProfilesCm");
        let field = crate::legacy::load_field_cases(
            root.with_extension("env"),
            root.with_extension("flp"),
            crate::ModeSolver::Kraken,
        )
        .unwrap()
        .remove(0);
        assert_eq!(field.propagation, FieldPropagation::Coupled);
        let solved = solve_field(&field).unwrap();
        let mid: Vec<f64> = field
            .ranges_m
            .windows(2)
            .map(|r| r[0].midpoint(r[1]))
            .collect();
        assert!(mid.len() >= 3, "the grids below skip at least two profiles");
        let last = mid[mid.len() - 1];
        let spread = |from: f64, to: f64, n: u32| -> Vec<f64> {
            (1..=n)
                .map(|i| from + (to - from) * f64::from(i) / f64::from(n))
                .collect()
        };
        let grids = [
            // One receiver beyond every midpoint: the march crosses them all first.
            vec![last + 100.0],
            // Exact midpoints stay with the profile on their left.
            mid.clone(),
            // Scalar ranges in the first profile, then 16 lanes and a scalar tail in the last.
            [
                vec![0.0, mid[0] / 2.0, mid[0]],
                spread(last, last + 2000.0, 20),
            ]
            .concat(),
            // A first receiver past one midpoint, then a group of nine in the last profile.
            [vec![mid[0].midpoint(mid[1])], spread(last, last + 900.0, 9)].concat(),
        ];
        for ranges in &grids {
            for geometry in [
                SourceGeometry::Point,
                SourceGeometry::Line,
                SourceGeometry::ScaledCylindrical,
            ] {
                let profiles = field
                    .profiles
                    .iter()
                    .map(|profile| {
                        let mut input = profile.clone().into_definition();
                        input.receiver_ranges_m.clone_from(ranges);
                        input.source_geometry = geometry;
                        Case::from_definition(input).unwrap()
                    })
                    .collect();
                let case =
                    FieldCase::new(profiles, field.ranges_m.clone(), field.propagation).unwrap();
                let first = &case.profiles[0];
                let samples =
                    ranges.len() * first.receiver_depths_m.len() * first.source_depths_m.len();
                for count in [0, 1, usize::MAX] {
                    let mut modes = solved.modes.clone();
                    for set in &mut modes {
                        set.modes.truncate(count);
                    }
                    let largest = modes.iter().map(|m| m.modes.len()).max().unwrap().max(1);
                    let expected = bits(&sequential_coupled(&case, &modes, samples));
                    for budget in [1, 7 * largest, 16 * largest, TABLE_VALUES] {
                        let grouped = synthesize(&case, &modes, samples, budget).unwrap();
                        assert_eq!(
                            bits(&grouped),
                            expected,
                            "{ranges:?} {geometry:?}, {count} modes, {budget} values"
                        );
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod solve_window_tests {
    use super::*;
    use std::sync::{Arc, atomic::AtomicBool};

    const SMALL: Option<usize> = Some(1);

    #[test]
    fn windows_scale_with_threads_and_few_threads_solve_one_at_a_time() {
        for (budget, expected) in [(1, 1), (2, 1), (7, 1), (8, 32), (14, 56), (32, 128)] {
            crate::parallelism::with_threads(NonZeroUsize::new(budget).unwrap(), || {
                assert_eq!(solve_window_len(0, true, SMALL), 0);
                assert_eq!(solve_window_len(1000, true, SMALL), expected);
                assert_eq!(solve_window_len(1000, false, SMALL), 1);
                assert_eq!(solve_window_len(3, true, SMALL), expected.min(3));
            });
        }
        crate::parallelism::with_threads(NonZeroUsize::MAX, || {
            assert_eq!(
                solve_window_len(usize::MAX, true, SMALL),
                crate::parallelism::MAX_WORKERS * SOLVES_PER_THREAD
            );
        });
    }

    #[test]
    fn nested_solves_inherit_only_their_share() {
        for budget in [3, 5, 14, 32] {
            crate::parallelism::with_threads(NonZeroUsize::new(budget).unwrap(), || {
                let token = Arc::new(AtomicBool::new(true));
                let shares = crate::cancellation::with_token(token, || {
                    solve_window(3, |index| {
                        let share = crate::parallelism::threads();
                        assert_eq!(share, budget / 3 + usize::from(index < budget % 3));
                        assert!(crate::cancellation::cancelled());
                        let nested = solve_window(share.min(2), |_| {
                            assert!(crate::cancellation::cancelled());
                            let profile_share = crate::parallelism::threads();
                            crate::solver::refinement::shape_map(3, usize::MAX, |_| {
                                assert!(crate::cancellation::cancelled());
                                assert_eq!(crate::parallelism::threads(), 1);
                                let caller = std::thread::current().id();
                                let lanes = crate::parallelism::map(2, usize::MAX, |_| {
                                    assert_eq!(std::thread::current().id(), caller);
                                    assert_eq!(crate::parallelism::threads(), 1);
                                    crate::cancellation::cancelled()
                                });
                                assert_eq!(lanes, [true, true]);
                                Ok(())
                            })
                            .unwrap();
                            assert_eq!(crate::parallelism::threads(), profile_share);
                            profile_share
                        });
                        assert_eq!(nested.iter().sum::<usize>(), share);
                        assert_eq!(crate::parallelism::threads(), share);
                        share
                    })
                });
                assert_eq!(shares.iter().sum::<usize>(), budget);
                assert_eq!(crate::parallelism::threads(), budget);
            });
            assert_eq!(crate::parallelism::threads(), 1);
            assert!(!crate::cancellation::cancelled());
        }
    }

    #[test]
    fn more_solves_than_threads_each_get_one_thread_and_keep_their_order() {
        crate::parallelism::with_threads(NonZeroUsize::new(3).unwrap(), || {
            let solves = solve_window(10, |index| (index, crate::parallelism::threads()));
            assert_eq!(solves, (0..10).map(|index| (index, 1)).collect::<Vec<_>>());
            assert_eq!(crate::parallelism::threads(), 3);
        });
    }

    #[test]
    fn a_first_solve_gets_every_thread_because_its_size_is_unknown() {
        crate::parallelism::with_threads(NonZeroUsize::new(14).unwrap(), || {
            assert_eq!(solve_window_len(100, true, None), 1);
            assert_eq!(solve_window(1, |_| crate::parallelism::threads()), [14]);
        });
    }

    #[test]
    fn excessive_thread_requests_are_capped_before_subdivision() {
        crate::parallelism::with_threads(NonZeroUsize::MAX, || {
            let shares = solve_window(8, |_| crate::parallelism::threads());
            assert_eq!(
                shares.iter().sum::<usize>(),
                crate::parallelism::MAX_WORKERS
            );
            assert_eq!(crate::parallelism::threads(), usize::MAX);
        });
    }

    #[test]
    fn large_results_reduce_how_many_solve_together() {
        crate::parallelism::with_threads(NonZeroUsize::new(14).unwrap(), || {
            for (bytes, expected) in [
                (SOLVE_AHEAD_BYTES / 16, 8),
                (SOLVE_AHEAD_BYTES / 16 + 1, 7),
                (SOLVE_AHEAD_BYTES / 4, 2),
                (SOLVE_AHEAD_BYTES / 2, 1),
                (usize::MAX, 1),
            ] {
                assert_eq!(solve_window_len(100, true, Some(bytes)), expected);
                assert_eq!(solve_window_len(100, false, Some(bytes)), 1);
                assert_eq!(solve_window_len(0, true, Some(bytes)), 0);
            }
        });
    }

    #[test]
    fn result_sizes_count_every_mode_above_the_field_cap() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/Pekeris");
        let mut input =
            crate::legacy::load_case(root.with_extension("env"), root.with_extension("flp"))
                .unwrap()
                .into_definition();
        input.mode_limit = 1;
        let case = Case::from_definition(input).unwrap();
        let set = crate::solver::solve_modes(&case, crate::MAX_MODE_LIMIT).unwrap();
        assert!(set.modes.len() > case.mode_limit);
        let shapes = set
            .modes
            .iter()
            .map(|mode| mode.eigenfunction.len())
            .sum::<usize>();
        assert!(mode_set_bytes(&set) >= shapes * size_of::<Complex64>());
    }

    #[test]
    fn only_real_elastic_layers_or_top_require_sequential_windows() {
        let fixtures = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
        for (stem, real_independent) in [
            ("Pekeris", true),
            ("ElasticHalfBottomN", true),
            ("FiniteElasticPower", false),
            ("ElasticHalfTopBroadband", false),
        ] {
            for solver in [crate::ModeSolver::Kraken, crate::ModeSolver::Krakenc] {
                let root = fixtures.join(stem);
                let cases = crate::legacy::load_field_cases(
                    root.with_extension("env"),
                    root.with_extension("flp"),
                    solver,
                )
                .unwrap();
                assert_eq!(
                    cases.iter().all(independent_profiles),
                    real_independent || solver == crate::ModeSolver::Krakenc,
                    "{stem} {solver:?}"
                );
            }
        }
    }
}

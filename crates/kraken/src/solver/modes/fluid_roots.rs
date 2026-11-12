//! Fluid-only isolation and safeguarded Illinois refinement.
//!
//! Invariant: C(x) is nonincreasing, and [a, b] owns precisely the targets
//! C(b)..C(a). A single-root interval must also bracket the continuous fluid
//! shooting function. Elastic impedance poles do not satisfy this invariant.
//! Failed isolation, sign, finiteness or final count checks leave the affected
//! targets to the original full-interval bisection, never to a guessed root.
use super::{DiagnosticReport, Mesh, ROOT_LANES, ROOT_STEPS, Refinement};

const RELATIVE_WIDTH: f64 = 1e-13;
const REFINE_STEPS: usize = 160;
const SHOOT_LANES: usize = 16;
// Keep enough independent chunks for multi-threaded profiles with only 62–63 modes.
const REFINE_CHUNK: usize = 16;

#[derive(Clone, Copy, Debug)]
struct Interval {
    a: f64,
    b: f64,
    ca: usize,
    cb: usize,
}

impl Interval {
    fn split(self, x: f64, count: usize) -> Option<[Self; 2]> {
        (self.a < x && x < self.b && self.cb <= count && count <= self.ca).then_some([
            Self {
                b: x,
                cb: count,
                ..self
            },
            Self {
                a: x,
                ca: count,
                ..self
            },
        ])
    }
}

#[derive(Clone, Copy)]
struct Value {
    value: f64,
    power: i32,
}

impl Value {
    fn opposite(self, other: Self) -> bool {
        self.value.is_finite()
            && other.value.is_finite()
            && self.value != 0.0
            && other.value != 0.0
            && self.value.is_sign_positive() != other.value.is_sign_positive()
    }

    fn fraction(self, other: Self) -> f64 {
        let power = self.power.max(other.power);
        let a = self.value * 2_f64.powi((self.power - power).max(-1074));
        let b = other.value * 2_f64.powi((other.power - power).max(-1074));
        a / (a - b)
    }
}

struct RootState {
    a: f64,
    b: f64,
    fa: Value,
    fb: Value,
    last: i8,
    iteration: usize,
    answer: Option<f64>,
}

impl RootState {
    fn trial(&mut self) -> Option<f64> {
        // The old loop never checked width after its last shooting evaluation.
        if self.iteration == REFINE_STEPS {
            return None;
        }
        let tolerance = self.b.abs().max(self.a.abs()) * RELATIVE_WIDTH;
        if self.b - self.a <= tolerance || self.a.next_up() >= self.b {
            self.answer = Some(self.a.midpoint(self.b));
            return None;
        }
        let mut x = self.a + (self.b - self.a) * self.fa.fraction(self.fb);
        if self.iteration % 8 == 7 || !x.is_finite() || x <= self.a || x >= self.b {
            x = self.a.midpoint(self.b);
        }
        self.iteration += 1;
        Some(x)
    }

    fn accept(&mut self, x: f64, f: Value) -> bool {
        if !f.value.is_finite() {
            return false;
        }
        if f.value == 0.0 {
            self.answer = Some(x);
            return false;
        }
        if f.value.is_sign_positive() == self.fa.value.is_sign_positive() {
            self.a = x;
            self.fa = f;
            if self.last == 1 {
                self.fb.value *= 0.5;
            }
            self.last = 1;
        } else {
            self.b = x;
            self.fb = f;
            if self.last == -1 {
                self.fa.value *= 0.5;
            }
            self.last = -1;
        }
        true
    }
}

// Cold and by value: inline/reference rescaling scalarized the hot loop on x86-64.
#[cold]
#[inline(never)]
fn rescale<const K: usize, const O: usize>(
    mut previous: [f64; K],
    mut current: [f64; K],
    mut older: [f64; O],
    mut power: [i32; K],
) -> ([f64; K], [f64; K], [f64; O], [i32; K]) {
    for k in 0..K {
        if current[k].abs() > 1e100 {
            current[k] *= 2_f64.powi(-256);
            previous[k] *= 2_f64.powi(-256);
            if k < O {
                older[k] *= 2_f64.powi(-256);
            }
            power[k] += 256;
        }
    }
    (previous, current, older, power)
}

// Keep independent multiply/subtract chains visible to the vectorizer.
#[inline(never)]
fn shoot_interior<const K: usize>(
    rows: &[f64],
    shift: [f64; K],
    previous_out: &mut [f64; K],
    current_out: &mut [f64; K],
    power_out: &mut [i32; K],
) -> [f64; K] {
    let mut previous = *previous_out;
    let mut current = *current_out;
    let mut power = *power_out;
    let Some((&last, rows)) = rows.split_first() else {
        return [0.0; K];
    };
    for &b in rows.iter().rev() {
        let mut diagonal = [0.0; K];
        let mut next = [0.0; K];
        for k in 0..K {
            diagonal[k] = shift[k] - b;
        }
        for k in 0..K {
            next[k] = diagonal[k] * current[k];
        }
        for k in 0..K {
            next[k] -= previous[k];
        }
        previous = current;
        current = next;
        // One combined test per row, where a short-circuiting `any` branched per lane.
        if current
            .iter()
            .fold(false, |need, v| need | (v.abs() > 1e100))
        {
            (previous, current, _, power) = rescale(previous, current, [], power);
        }
    }
    // Only the last row needs the two-rows-back value for the boundary derivative.
    let mut older = previous;
    let mut next = [0.0; K];
    for k in 0..K {
        next[k] = (shift[k] - last) * current[k] - previous[k];
    }
    previous = current;
    current = next;
    if current
        .iter()
        .fold(false, |need, v| need | (v.abs() > 1e100))
    {
        (previous, current, older, power) = rescale(previous, current, older, power);
    }
    *previous_out = previous;
    *current_out = current;
    *power_out = power;
    older
}

impl Mesh<'_> {
    // The fluid part of solid_dispersion, without deflation. Positive binary
    // rescaling preserves signs and carries endpoint magnitudes for Illinois.
    fn fluid_dispersion<const K: usize>(&self, x: [f64; K]) -> [Value; K] {
        let mut f = [0.0; K];
        let mut g = [0.0; K];
        let mut power = [0; K];
        for k in 0..K {
            let (fk, gk, _) = self.solid_boundary(x[k], false);
            f[k] = fk.re;
            g[k] = gk.re;
        }
        for layer in self.layers.iter().rev() {
            let shift = x.map(|v| layer.h * layer.h * v);
            let mut previous = g.map(|v| -2.0 * v);
            let mut current = std::array::from_fn(|k| {
                (self.b1[layer.coefficient_start + layer.intervals] - shift[k]) * g[k]
                    - (2.0 * f[k]) * (layer.h * layer.density)
            });
            let older = shoot_interior(
                &self.b1[layer.coefficient_start..layer.coefficient_start + layer.intervals],
                shift,
                &mut previous,
                &mut current,
                &mut power,
            );
            for k in 0..K {
                f[k] = -(current[k] - older[k]) / (2.0 * layer.h * layer.density);
                g[k] = -previous[k];
            }
        }
        std::array::from_fn(|k| {
            let (ft, gt, _) = self.solid_boundary(x[k], true);
            Value {
                value: f[k] * gt.re - g[k] * ft.re,
                power: power[k],
            }
        })
    }

    fn fluid_dispersion_tail<const K: usize>(&self, x: [f64; K], live: usize) -> [Value; K] {
        let mut values = [Value {
            value: 0.0,
            power: 0,
        }; K];
        if K > 4 && live <= 4 {
            values[..4]
                .copy_from_slice(&self.fluid_dispersion(std::array::from_fn::<_, 4, _>(|k| x[k])));
        } else if K > 8 && live <= 8 {
            values[..8]
                .copy_from_slice(&self.fluid_dispersion(std::array::from_fn::<_, 8, _>(|k| x[k])));
        } else {
            values = self.fluid_dispersion(x);
        }
        values
    }

    fn isolate_fluid(&self, whole: Interval) -> Result<Vec<Option<Interval>>, DiagnosticReport> {
        let mut isolated = vec![None; whole.ca - whole.cb];
        let mut frontier = vec![whole];
        for _ in 0..ROOT_STEPS {
            let mut next = Vec::new();
            for chunk in frontier.chunks(ROOT_LANES) {
                crate::cancellation::check()?;
                let x = std::array::from_fn(|k| {
                    let interval = chunk[k.min(chunk.len() - 1)];
                    interval.a.midpoint(interval.b)
                });
                let counts = self.count_lanes::<ROOT_LANES>(x);
                for (k, &interval) in chunk.iter().enumerate() {
                    if interval.ca - interval.cb == 1 {
                        if interval.cb <= counts[k] && counts[k] <= interval.ca {
                            isolated[interval.cb - whole.cb] = Some(interval);
                        }
                    } else if let Some(parts) = interval.split(x[k], counts[k]) {
                        for part in parts {
                            match part.ca - part.cb {
                                0 => (),
                                1 => isolated[part.cb - whole.cb] = Some(part),
                                _ => next.push(part),
                            }
                        }
                    }
                    // Unsplit targets stay None, including adjacent-float clusters.
                }
            }
            frontier = next;
            if frontier.is_empty() {
                break;
            }
        }
        Ok(isolated)
    }

    // Predictions are hints, never mode identities. Current-mesh counts must
    // certify exactly the requested target, including at clipped spectral limits.
    fn seed_fluid<const K: usize>(
        &self,
        seeds: &[Option<f64>],
        whole: Interval,
        start: usize,
    ) -> Result<Vec<Option<Interval>>, DiagnosticReport> {
        let x: [f64; K] =
            std::array::from_fn(|k| seeds[k.min(seeds.len() - 1)].unwrap_or(f64::NAN));
        let mut radius = x.map(|x| x.abs() * 1e-6);
        let mut active: [bool; K] =
            std::array::from_fn(|k| k < seeds.len() && x[k] > whole.a && x[k] < whole.b);
        let mut brackets = vec![None; seeds.len()];
        for _ in 0..8 {
            if !active.contains(&true) {
                break;
            }
            crate::cancellation::check()?;
            let a = std::array::from_fn(|k| (x[k] - radius[k]).max(whole.a));
            let b = std::array::from_fn(|k| (x[k] + radius[k]).min(whole.b));
            let ca = self.count_lanes::<K>(a);
            let cb = self.count_lanes::<K>(b);
            for k in 0..seeds.len() {
                if !active[k] {
                    continue;
                }
                let target = whole.cb + start + k;
                if a[k] < b[k] && ca[k] == target + 1 && cb[k] == target {
                    brackets[k] = Some(Interval {
                        a: a[k],
                        b: b[k],
                        ca: ca[k],
                        cb: cb[k],
                    });
                    active[k] = false;
                } else if ca[k] != cb[k] || ca[k] > whole.ca || cb[k] < whole.cb {
                    // A wrong mode, cluster or inconsistent count: widening cannot certify it.
                    active[k] = false;
                } else {
                    radius[k] *= 4.0;
                }
            }
        }
        Ok(brackets)
    }

    #[allow(clippy::too_many_lines)] // Keep endpoint, queue and certification passes together.
    fn refine_fluid<const K: usize>(
        &self,
        intervals: &[Option<Interval>],
        whole: Interval,
    ) -> Result<Vec<Option<f64>>, DiagnosticReport> {
        crate::cancellation::check()?;
        let mut states: Vec<_> = intervals
            .iter()
            .map(|interval| {
                let bracket = interval.unwrap_or(whole);
                RootState {
                    a: bracket.a,
                    b: bracket.b,
                    fa: Value {
                        value: 0.0,
                        power: 0,
                    },
                    fb: Value {
                        value: 0.0,
                        power: 0,
                    },
                    last: 0,
                    iteration: 0,
                    answer: None,
                }
            })
            .collect();
        // Pack both endpoints together, including across the old group boundaries.
        let endpoints: Vec<_> = intervals
            .iter()
            .enumerate()
            .filter(|(_, interval)| interval.is_some())
            .flat_map(|(mode, _)| [2 * mode, 2 * mode + 1])
            .collect();
        for chunk in endpoints.chunks(K) {
            crate::cancellation::check()?;
            let x = std::array::from_fn(|k| {
                let endpoint = chunk[k.min(chunk.len() - 1)];
                let state = &states[endpoint / 2];
                if endpoint % 2 == 0 { state.a } else { state.b }
            });
            let values = self.fluid_dispersion_tail::<K>(x, chunk.len());
            for (&endpoint, value) in chunk.iter().zip(values) {
                let state = &mut states[endpoint / 2];
                if endpoint % 2 == 0 {
                    state.fa = value;
                } else {
                    state.fb = value;
                }
            }
        }
        let pending: Vec<_> = states
            .iter()
            .enumerate()
            .filter(|(_, state)| state.fa.opposite(state.fb))
            .map(|(mode, _)| mode)
            .collect();
        let mut pending = pending.into_iter();
        let mut lanes: Vec<usize> = Vec::with_capacity(K);
        loop {
            crate::cancellation::check()?;
            let mut x = [0.0; K];
            let mut live = 0;
            lanes.retain(|&mode| {
                if let Some(trial) = states[mode].trial() {
                    x[live] = trial;
                    live += 1;
                    true
                } else {
                    false
                }
            });
            while lanes.len() < K {
                let Some(mode) = pending.next() else { break };
                crate::cancellation::check()?;
                if let Some(trial) = states[mode].trial() {
                    x[lanes.len()] = trial;
                    lanes.push(mode);
                }
            }
            if lanes.is_empty() {
                break;
            }
            let padding = x[0];
            x[lanes.len()..].fill(padding);
            let values = self.fluid_dispersion_tail(x, lanes.len());
            let mut k = 0;
            lanes.retain(|&mode| {
                let active = states[mode].accept(x[k], values[k]);
                k += 1;
                active
            });
        }
        // Keep the original clipping and target check, but pack successful modes.
        let endpoints: Vec<_> = states
            .iter()
            .enumerate()
            .filter(|(_, state)| state.answer.is_some())
            .flat_map(|(mode, _)| [2 * mode, 2 * mode + 1])
            .collect();
        let mut counts = vec![[0; 2]; states.len()];
        for chunk in endpoints.chunks(K) {
            crate::cancellation::check()?;
            let x = std::array::from_fn(|k| {
                let endpoint = chunk[k.min(chunk.len() - 1)];
                let mode = endpoint / 2;
                let x = states[mode].answer.unwrap();
                let bracket = intervals[mode].unwrap();
                if endpoint % 2 == 0 {
                    (x - x.abs() * RELATIVE_WIDTH).max(bracket.a)
                } else {
                    (x + x.abs() * RELATIVE_WIDTH).min(bracket.b)
                }
            });
            let values = self.count_lanes::<K>(x);
            for (&endpoint, value) in chunk.iter().zip(values) {
                counts[endpoint / 2][endpoint % 2] = value;
            }
        }
        Ok(states
            .into_iter()
            .enumerate()
            .map(|(mode, state)| {
                state.answer.filter(|_| {
                    let bracket = intervals[mode].unwrap();
                    counts[mode] == [bracket.ca, bracket.cb]
                })
            })
            .collect())
    }

    #[cfg(test)]
    #[allow(clippy::needless_range_loop)] // Each lane owns its bracket and stopping point.
    fn refine_fluid_original<const K: usize>(
        &self,
        intervals: &[Option<Interval>],
        whole: Interval,
    ) -> Result<[Option<f64>; K], DiagnosticReport> {
        let bracket: [Interval; K] =
            std::array::from_fn(|k| intervals[k.min(intervals.len() - 1)].unwrap_or(whole));
        let mut a = bracket.map(|i| i.a);
        let mut b = bracket.map(|i| i.b);
        crate::cancellation::check()?;
        let mut fa = self.fluid_dispersion(a);
        let mut fb = self.fluid_dispersion(b);
        let mut active = [false; K];
        for k in 0..intervals.len() {
            active[k] = intervals[k].is_some() && fa[k].opposite(fb[k]);
        }
        let mut last = [0_i8; K];
        let mut answer = [None; K];
        for iteration in 0..REFINE_STEPS {
            crate::cancellation::check()?;
            let mut x = a;
            for k in 0..intervals.len() {
                if !active[k] {
                    continue;
                }
                let tolerance = b[k].abs().max(a[k].abs()) * RELATIVE_WIDTH;
                if b[k] - a[k] <= tolerance || a[k].next_up() >= b[k] {
                    active[k] = false;
                    answer[k] = Some(a[k].midpoint(b[k]));
                } else {
                    x[k] = a[k] + (b[k] - a[k]) * fa[k].fraction(fb[k]);
                    if iteration % 8 == 7 || !x[k].is_finite() || x[k] <= a[k] || x[k] >= b[k] {
                        x[k] = a[k].midpoint(b[k]);
                    }
                }
            }
            if !active.contains(&true) {
                break;
            }
            let f = self.fluid_dispersion(x);
            for k in 0..intervals.len() {
                if !active[k] {
                    continue;
                }
                if !f[k].value.is_finite() {
                    active[k] = false;
                } else if f[k].value == 0.0 {
                    active[k] = false;
                    answer[k] = Some(x[k]);
                } else if f[k].value.is_sign_positive() == fa[k].value.is_sign_positive() {
                    a[k] = x[k];
                    fa[k] = f[k];
                    if last[k] == 1 {
                        fb[k].value *= 0.5;
                    }
                    last[k] = 1;
                } else {
                    b[k] = x[k];
                    fb[k] = f[k];
                    if last[k] == -1 {
                        fa[k].value *= 0.5;
                    }
                    last[k] = -1;
                }
            }
        }
        // Shooting and LDLᵀ round differently. Check the target within one
        // relative tolerance of the answer, not at its possibly rounded zero.
        if answer.iter().any(Option::is_some) {
            let left = std::array::from_fn(|k| {
                let x = answer[k].unwrap_or(bracket[k].a);
                (x - x.abs() * RELATIVE_WIDTH).max(bracket[k].a)
            });
            let right = std::array::from_fn(|k| {
                let x = answer[k].unwrap_or(bracket[k].b);
                (x + x.abs() * RELATIVE_WIDTH).min(bracket[k].b)
            });
            crate::cancellation::check()?;
            let ca = self.count_lanes::<K>(left);
            let cb = self.count_lanes::<K>(right);
            for k in 0..intervals.len() {
                if ca[k] != bracket[k].ca || cb[k] != bracket[k].cb {
                    answer[k] = None;
                }
            }
        }
        Ok(answer)
    }

    fn fluid_group(
        &self,
        intervals: &[Option<Interval>],
        whole: Interval,
        start: usize,
    ) -> Result<Vec<f64>, DiagnosticReport> {
        let answers = self.refine_fluid::<SHOOT_LANES>(intervals, whole)?;
        let mut roots = Vec::with_capacity(intervals.len());
        for (k, answer) in answers.into_iter().take(intervals.len()).enumerate() {
            roots.push(if let Some(x) = answer {
                x
            } else {
                let mode = start + k;
                self.bisect::<1>(whole.a, whole.b, whole.cb, mode..mode + 1)?[0]
            });
        }
        Ok(roots)
    }

    /// Roots in descending order. `roots` has already charged the original
    /// bisection's fixed work, so this search adds no admission rule of its own;
    /// its isolation, refinement and fallback steps are each capped.
    pub(super) fn fluid_roots(
        &self,
        low: f64,
        high: f64,
        above: usize,
        count: usize,
        refinement: &Refinement<f64>,
    ) -> Result<Vec<f64>, DiagnosticReport> {
        let whole = Interval {
            a: low,
            b: high,
            ca: above + count,
            cb: above,
        };
        let isolated = if refinement.seed(0, self.h).is_none() {
            Some(self.isolate_fluid(whole)?)
        } else {
            None
        };
        // One queue for the one-thread workload; otherwise fixed, ordered chunks.
        let chunk_size = if crate::parallelism::threads() == 1 {
            count.max(1)
        } else {
            REFINE_CHUNK
        };
        let groups = count.div_ceil(chunk_size);
        let estimated_work = count.saturating_mul(self.b1.len()).saturating_mul(32);
        let mut roots = Vec::with_capacity(count);
        for result in crate::parallelism::map(groups, estimated_work, |group| {
            let start = group * chunk_size;
            let end = count.min(start + chunk_size);
            if let Some(isolated) = &isolated {
                self.fluid_group(&isolated[start..end], whole, start)
            } else {
                let seeds: Vec<_> = (start..end)
                    .map(|index| refinement.seed(index, self.h).map(|x| x.re))
                    .collect();
                let mut brackets = Vec::with_capacity(seeds.len());
                for (group, chunk) in seeds.chunks(ROOT_LANES).enumerate() {
                    brackets.extend(self.seed_fluid::<ROOT_LANES>(
                        chunk,
                        whole,
                        start + group * ROOT_LANES,
                    )?);
                }
                self.fluid_group(&brackets, whole, start)
            }
        }) {
            crate::cancellation::check()?;
            roots.extend(result?);
        }
        Ok(roots)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::solver::modes::MAX_WORK;
    use crate::{BottomBoundary, SurfaceBoundary, solver::modes::Meshes};
    use std::path::Path;

    fn load(name: &str) -> crate::Case {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(name);
        crate::legacy::load_case(root.with_extension("env"), root.with_extension("flp")).unwrap()
    }

    #[allow(clippy::float_cmp)] // The production plane-mode guard uses exact input equality.
    fn interval(mesh: &Mesh<'_>) -> Interval {
        let a = 1.00001 * (mesh.omega / mesh.case.c_high_mps).powi(2);
        let mut b = (mesh.omega / mesh.case.c_low_mps.max(mesh.min_speed)).powi(2);
        if mesh.case.surface_boundary == SurfaceBoundary::Rigid
            && mesh.case.bottom_boundary == BottomBoundary::Rigid
            && mesh.case.c_low_mps <= mesh.min_speed
            && crate::solver::layers::iter(mesh.case)
                .flat_map(|layer| layer.points)
                .all(|point| point.sound_speed_mps == mesh.min_speed)
        {
            b = b.max((mesh.b1[0] + 2.0) / (mesh.h * mesh.h)).next_up();
        }
        Interval {
            a,
            b,
            ca: mesh.count(a),
            cb: mesh.count(b),
        }
    }

    /// The original shooting kernel, one lane at a time.
    fn shoot_lane(
        rows: &[f64],
        shift: f64,
        previous: &mut f64,
        current: &mut f64,
        power: &mut i32,
    ) -> f64 {
        let mut older = 0.0;
        for &b in rows.iter().rev() {
            let next = (shift - b) * *current - *previous;
            older = *previous;
            *previous = *current;
            *current = next;
            if current.abs() > 1e100 {
                *current *= 2_f64.powi(-256);
                *previous *= 2_f64.powi(-256);
                older *= 2_f64.powi(-256);
                *power += 256;
            }
        }
        older
    }

    #[test]
    fn shooting_kernel_matches_every_lane_of_the_original_bit_for_bit() {
        fn same(a: f64, b: f64) -> bool {
            a.to_bits() == b.to_bits() || (a.is_nan() && b.is_nan())
        }
        fn check<const K: usize>(
            rows: &[f64],
            shift: [f64; K],
            start: [(f64, f64); K],
        ) -> [i32; K] {
            let mut previous = start.map(|(p, _)| p);
            let mut current = start.map(|(_, c)| c);
            let mut power = [0; K];
            let older = shoot_interior(rows, shift, &mut previous, &mut current, &mut power);
            for k in 0..K {
                let (mut p, mut c) = start[k];
                let mut w = 0;
                let o = shoot_lane(rows, shift[k], &mut p, &mut c, &mut w);
                assert!(
                    same(older[k], o)
                        && same(previous[k], p)
                        && same(current[k], c)
                        && power[k] == w,
                    "lane {k} of {K}: ({}, {}, {}, {}) vs ({o}, {p}, {c}, {w})",
                    older[k],
                    previous[k],
                    current[k],
                    power[k]
                );
            }
            power
        }
        // Real duct coefficients: oscillating inside the duct, growing past 1e100 outside it.
        let case = load("SductTrapped");
        let meshes = Meshes::new(&case).unwrap();
        let mesh = Mesh::new(&meshes, 4).unwrap();
        let rows = &mesh.b1[..];
        let (low, high) = rows
            .iter()
            .fold((f64::INFINITY, f64::NEG_INFINITY), |(l, h), &b| {
                (l.min(b), h.max(b))
            });
        let middle = low.midpoint(high);
        let shift = [
            middle,
            high + 8.0,
            rows[rows.len() - 1], // an exact zero on the first row
            high + 8.0,           // a padded lane repeats its neighbour
            f64::NAN,
            f64::INFINITY,
            middle,
            low - 8.0,
        ];
        let start = [
            (1.0, 0.5),
            (-2.0, 3.0),
            (1.0, 0.5),
            (-2.0, 3.0),
            (1.0, 0.5),
            (1.0, 0.5),
            (0.0, 0.0), // zero stays zero
            (0.25, -1.0),
        ];
        let power = check::<ROOT_LANES>(rows, shift, start);
        check::<12>(
            rows,
            std::array::from_fn(|k| shift[k % 8]),
            std::array::from_fn(|k| start[k % 8]),
        );
        check::<16>(
            rows,
            std::array::from_fn(|k| shift[k % 8]),
            std::array::from_fn(|k| start[k % 8]),
        );
        assert!(power[1] > 0 && power[7] > 0 && power[6] == 0, "{power:?}");
        for k in 0..ROOT_LANES {
            check::<1>(rows, [shift[k]], [start[k]]);
        }
        // Short rows, as in thin layers, including none at all.
        for length in [0, 1, 2, 3] {
            check::<ROOT_LANES>(&rows[..length], shift, start);
        }
    }

    fn oracle(mesh: &Mesh<'_>, brackets: &[Option<Interval>], whole: Interval) -> Vec<Option<f64>> {
        brackets
            .chunks(ROOT_LANES)
            .flat_map(|chunk| {
                mesh.refine_fluid_original::<ROOT_LANES>(chunk, whole)
                    .unwrap()
                    .into_iter()
                    .take(chunk.len())
            })
            .collect()
    }

    fn assert_oracle(mesh: &Mesh<'_>, brackets: &[Option<Interval>], whole: Interval) {
        let expected = oracle(mesh, brackets, whole);
        for threads in [1, 4] {
            crate::parallelism::with_threads(std::num::NonZeroUsize::new(threads).unwrap(), || {
                let bits = |values: Vec<Option<f64>>| {
                    values
                        .into_iter()
                        .map(|x| x.map(f64::to_bits))
                        .collect::<Vec<_>>()
                };
                assert_eq!(
                    bits(mesh.refine_fluid::<4>(brackets, whole).unwrap()),
                    bits(expected.clone())
                );
                assert_eq!(
                    bits(mesh.refine_fluid::<8>(brackets, whole).unwrap()),
                    bits(expected.clone())
                );
                assert_eq!(
                    bits(mesh.refine_fluid::<12>(brackets, whole).unwrap()),
                    bits(expected.clone())
                );
                assert_eq!(
                    bits(mesh.refine_fluid::<16>(brackets, whole).unwrap()),
                    bits(expected.clone())
                );
                let actual = mesh.fluid_group(brackets, whole, 0).unwrap();
                for (mode, (actual, expected)) in actual.into_iter().zip(&expected).enumerate() {
                    let expected = expected.unwrap_or_else(|| {
                        mesh.bisect::<1>(whole.a, whole.b, whole.cb, mode..mode + 1)
                            .unwrap()[0]
                    });
                    assert_eq!(
                        actual.to_bits(),
                        expected.to_bits(),
                        "mode {mode}, threads {threads}"
                    );
                }
            });
        }
    }

    fn original_fluid_roots(
        mesh: &Mesh<'_>,
        whole: Interval,
        history: &Refinement<f64>,
    ) -> Vec<f64> {
        let count = whole.ca - whole.cb;
        let isolated = history
            .seed(0, mesh.h)
            .is_none()
            .then(|| mesh.isolate_fluid(whole).unwrap());
        let mut roots = Vec::with_capacity(count);
        for start in (0..count).step_by(ROOT_LANES) {
            let end = count.min(start + ROOT_LANES);
            let brackets = if let Some(isolated) = &isolated {
                isolated[start..end].to_vec()
            } else {
                let seeds: Vec<_> = (start..end)
                    .map(|mode| history.seed(mode, mesh.h).map(|x| x.re))
                    .collect();
                mesh.seed_fluid::<ROOT_LANES>(&seeds, whole, start).unwrap()
            };
            let answers = mesh
                .refine_fluid_original::<ROOT_LANES>(&brackets, whole)
                .unwrap();
            for (k, answer) in answers.into_iter().take(brackets.len()).enumerate() {
                let mode = start + k;
                roots.push(answer.unwrap_or_else(|| {
                    mesh.bisect::<1>(whole.a, whole.b, whole.cb, mode..mode + 1)
                        .unwrap()[0]
                }));
            }
        }
        roots
    }

    #[test]
    fn fluid_roots_dispatch_matches_original_groups_across_chunk_and_seed_boundaries() {
        let mut definition = load("MunkLossless").into_definition();
        definition.max_range_m = 1e20;
        let case = crate::Case::from_definition(definition).unwrap();
        let meshes = Meshes::new(&case).unwrap();
        let mut history = Refinement::<f64>::default();
        let bits = |roots: &[f64]| roots.iter().map(|x| x.to_bits()).collect::<Vec<_>>();
        for multiplier in [1, 2, 4] {
            let mesh = Mesh::new(&meshes, multiplier).unwrap();
            let full = interval(&mesh);
            assert_eq!(history.seed(0, mesh.h).is_some(), multiplier == 4);
            let original = original_fluid_roots(&mesh, full, &history);
            assert!(original.len() > 3 * REFINE_CHUNK + 1);
            for count in [
                REFINE_CHUNK - 1,
                REFINE_CHUNK,
                REFINE_CHUNK + 1,
                3 * REFINE_CHUNK - 1,
                3 * REFINE_CHUNK,
                3 * REFINE_CHUNK + 1,
                original.len(),
            ] {
                let whole = if count == original.len() {
                    full
                } else {
                    Interval {
                        a: original[count - 1].midpoint(original[count]),
                        ca: full.cb + count,
                        ..full
                    }
                };
                assert_eq!(mesh.count(whole.a), whole.ca);
                let expected = original_fluid_roots(&mesh, whole, &history);
                for threads in [1, 4] {
                    let actual = crate::parallelism::with_threads(
                        std::num::NonZeroUsize::new(threads).unwrap(),
                        || {
                            mesh.fluid_roots(whole.a, whole.b, whole.cb, count, &history)
                                .unwrap()
                        },
                    );
                    assert_eq!(
                        bits(&actual),
                        bits(&expected),
                        "mesh {multiplier}, modes {count}, threads {threads}"
                    );
                }
            }
            history
                .accept(original, Some(mesh.h), &case, |roots| mesh.modes(roots))
                .unwrap();
        }
    }

    #[test]
    fn refill_matches_original_groups_bit_for_bit() {
        for name in [
            "Pekeris",
            "LayeredFluidThree",
            "MunkLossless",
            "SductTrapped",
            "FluidBoundaryRR",
        ] {
            let case = load(name);
            let meshes = Meshes::new(&case).unwrap();
            for multiplier in [1, 2, 4] {
                let mesh = Mesh::new(&meshes, multiplier).unwrap();
                let whole = interval(&mesh);
                let isolated = mesh.isolate_fluid(whole).unwrap();
                let mut seeded = Vec::new();
                for (group, chunk) in isolated.chunks(ROOT_LANES).enumerate() {
                    let start = group * ROOT_LANES;
                    let roots = mesh
                        .bisect::<ROOT_LANES>(
                            whole.a,
                            whole.b,
                            whole.cb,
                            start..start + chunk.len(),
                        )
                        .unwrap();
                    let seeds: Vec<_> = roots.into_iter().map(Some).collect();
                    seeded.extend(mesh.seed_fluid::<ROOT_LANES>(&seeds, whole, start).unwrap());
                }
                for brackets in [&isolated, &seeded] {
                    for count in [1, 2, 3, 7, 8, 9, 15, 16, 17, brackets.len()] {
                        if count <= brackets.len() {
                            assert_oracle(&mesh, &brackets[..count], whole);
                        }
                    }
                }
                // Interleave immediate fallback, narrow seeded and wide isolated modes.
                let mixed: Vec<_> = isolated
                    .iter()
                    .zip(&seeded)
                    .enumerate()
                    .map(|(mode, (&a, &b))| match mode % 3 {
                        0 => None,
                        1 => b,
                        _ => a,
                    })
                    .collect();
                assert_oracle(&mesh, &mixed, whole);
            }
        }
    }

    #[test]
    fn refill_fallbacks_match_original_groups() {
        let case = load("Pekeris");
        let meshes = Meshes::new(&case).unwrap();
        let mut mesh = Mesh::new(&meshes, 1).unwrap();
        let whole = interval(&mesh);
        let isolated = mesh.isolate_fluid(whole).unwrap();
        let bracket = isolated[0].unwrap();
        for brackets in [
            vec![None],
            vec![Some(Interval {
                a: whole.b.next_down(),
                b: whole.b,
                ..bracket
            })],
            vec![Some(Interval {
                ca: bracket.ca + 1,
                cb: bracket.cb + 1,
                ..bracket
            })],
        ] {
            assert!(oracle(&mesh, &brackets, whole)[0].is_none());
            assert_oracle(&mesh, &brackets, whole);
        }
        mesh.b1[1] = f64::INFINITY;
        assert!(oracle(&mesh, &isolated, whole).iter().all(Option::is_none));
        assert_oracle(&mesh, &isolated, whole);
    }

    #[test]
    fn iteration_cap_matches_the_original_loop_with_lane_refills() {
        let case = load("FluidBoundaryVV");
        let meshes = Meshes::new(&case).unwrap();
        let mut mesh = Mesh::new(&meshes, 1).unwrap();
        mesh.layers[0].intervals = 4;
        mesh.layers[0].h = 1.0;
        mesh.h = 1.0;
        mesh.b1 = vec![2.0; 5];
        let whole = Interval {
            a: 0.0,
            b: 1e90,
            ca: 3,
            cb: 0,
        };
        let [fa, fb] = mesh.fluid_dispersion([whole.a, whole.b]);
        assert!(fa.opposite(fb));
        let mut state = RootState {
            a: whole.a,
            b: whole.b,
            fa,
            fb,
            last: 0,
            iteration: 0,
            answer: None,
        };
        for _ in 0..REFINE_STEPS {
            let x = state.trial().expect("wide interval still active");
            assert!(state.accept(x, mesh.fluid_dispersion([x])[0]));
        }
        assert!(state.trial().is_none());
        assert!(state.answer.is_none());
        assert!(oracle(&mesh, &[Some(whole)], whole)[0].is_none());
        assert_oracle(&mesh, &[Some(whole)], whole);
        let zero = Interval {
            a: 1.0,
            b: 3.0,
            ca: mesh.count(1.0),
            cb: mesh.count(3.0),
        };
        assert_eq!(
            mesh.fluid_dispersion([2.0])[0].value.abs().to_bits(),
            0.0_f64.to_bits()
        );
        // Cap modes outnumber the lanes; first-trial zeros refill beside them and after them.
        let brackets: Vec<_> = (0..4 * SHOOT_LANES + 2)
            .map(|mode| match mode % 4 {
                0 => Some(whole),
                1 => Some(zero),
                2 => None,
                _ => Some(Interval { a: 2.0, ..zero }), // A zero endpoint rejects refinement.
            })
            .collect();
        for (mode, answer) in oracle(&mesh, &brackets, whole).into_iter().enumerate() {
            assert_eq!(
                answer.map(f64::to_bits),
                (mode % 4 == 1).then_some(2.0_f64.to_bits())
            );
        }
        assert_oracle(&mesh, &brackets, whole);
    }

    #[test]
    fn nonfinite_interior_values_reject_an_active_refinement() {
        for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            let mut state = RootState {
                a: 1.0,
                b: 2.0,
                fa: Value {
                    value: -f64::MIN_POSITIVE,
                    power: -1000,
                },
                fb: Value {
                    value: 1.0,
                    power: 1000,
                },
                last: 0,
                iteration: 0,
                answer: None,
            };
            assert!(state.fa.opposite(state.fb));
            let x = state.trial().unwrap();
            // An underflowed secant fraction rounds to a; the guard must pick an interior midpoint.
            assert_eq!(x.to_bits(), 1.5_f64.to_bits());
            assert_eq!(state.iteration, 1);
            assert!(!state.accept(x, Value { value, power: 0 }));
            assert!(state.answer.is_none());
            assert_eq!(
                (state.a.to_bits(), state.b.to_bits(), state.last),
                (1.0_f64.to_bits(), 2.0_f64.to_bits(), 0)
            );
        }
    }

    #[test]
    fn last_shooting_step_accepts_zero_but_not_a_newly_narrow_bracket() {
        let mut state = RootState {
            a: 1.0,
            b: 2.0,
            fa: Value {
                value: -1.0,
                power: 0,
            },
            fb: Value {
                value: 1.0,
                power: 0,
            },
            last: 0,
            iteration: REFINE_STEPS - 1,
            answer: None,
        };
        let x = state.trial().unwrap();
        assert_eq!(x.to_bits(), 1.5_f64.to_bits());
        assert!(state.accept(
            x,
            Value {
                value: -0.5,
                power: 0
            }
        ));
        state.b = state.a.next_up();
        assert!(state.trial().is_none());
        assert!(state.answer.is_none());
        assert!(!state.accept(
            x,
            Value {
                value: 0.0,
                power: 0
            }
        ));
        assert_eq!(state.answer.unwrap().to_bits(), x.to_bits());
    }

    #[test]
    fn split_rejects_reversed_counts_and_stalled_brackets() {
        let whole = Interval {
            a: 1.0,
            b: 2.0,
            ca: 5,
            cb: 2,
        };
        for (x, count) in [(1.5, 1), (1.5, 6), (1.0, 4), (2.0, 4), (f64::NAN, 4)] {
            assert!(whole.split(x, count).is_none());
        }
        let [left, right] = whole.split(1.5, 4).unwrap();
        assert_eq!((left.ca, left.cb, right.ca, right.cb), (5, 4, 4, 2));
        assert_eq!(left.b.to_bits(), right.a.to_bits());
    }

    #[test]
    fn fluid_brackets_and_refinement_track_exact_roots() {
        for name in [
            "Pekeris",
            "LayeredFluidThree",
            "SductTrapped",
            "FluidBoundaryRR",
        ] {
            let case = load(name);
            let meshes = Meshes::new(&case).unwrap();
            for multiplier in [1, 2, 4] {
                let mesh = Mesh::new(&meshes, multiplier).unwrap();
                let whole = interval(&mesh);
                let isolated = mesh.isolate_fluid(whole).unwrap();
                for (mode, bracket) in isolated.iter().enumerate() {
                    let bracket = bracket.expect("separated fixture root");
                    assert_eq!(bracket.ca, whole.cb + mode + 1);
                    assert_eq!(bracket.cb, whole.cb + mode);
                    assert_eq!(mesh.count(bracket.a), bracket.ca);
                    assert_eq!(mesh.count(bracket.b), bracket.cb);
                }
                let mut refined = 0;
                for (group, chunk) in isolated.chunks(ROOT_LANES).enumerate() {
                    let fast = mesh.refine_fluid::<ROOT_LANES>(chunk, whole).unwrap();
                    for (k, x) in fast.into_iter().take(chunk.len()).enumerate() {
                        if let Some(x) = x {
                            refined += 1;
                            let mode = group * ROOT_LANES + k;
                            let exact = mesh
                                .bisect::<1>(whole.a, whole.b, whole.cb, mode..mode + 1)
                                .unwrap()[0];
                            assert!(
                                (x - exact).abs() <= 2.0 * exact.abs() * RELATIVE_WIDTH,
                                "{name} mesh {multiplier} mode {mode}: {x} vs {exact}"
                            );
                        }
                    }
                }
                assert!(refined > 0, "{name} mesh {multiplier} never refined");
            }
        }
    }

    #[test]
    fn missing_isolation_and_same_sign_brackets_fall_back_to_exact_bisection() {
        let case = load("Pekeris");
        let meshes = Meshes::new(&case).unwrap();
        let mesh = Mesh::new(&meshes, 1).unwrap();
        let whole = interval(&mesh);
        let wrong = Interval {
            a: whole.b.next_down(),
            b: whole.b,
            ca: whole.cb + 1,
            cb: whole.cb,
        };
        let answers = mesh
            .refine_fluid::<ROOT_LANES>(&[Some(wrong)], whole)
            .unwrap();
        assert!(answers[0].is_none());
        for brackets in [[None], [Some(wrong)]] {
            let actual = mesh.fluid_group(&brackets, whole, 0).unwrap();
            let exact = mesh.bisect::<1>(whole.a, whole.b, whole.cb, 0..1).unwrap();
            assert_eq!(actual[0].to_bits(), exact[0].to_bits());
        }
    }

    #[test]
    fn final_count_mismatch_and_nonfinite_dispersion_reject_refinement() {
        let case = load("Pekeris");
        let meshes = Meshes::new(&case).unwrap();
        let mut mesh = Mesh::new(&meshes, 1).unwrap();
        let whole = interval(&mesh);
        let isolated = mesh.isolate_fluid(whole).unwrap();
        let bracket = isolated[0].unwrap();
        assert!(
            mesh.refine_fluid::<ROOT_LANES>(&[Some(bracket)], whole)
                .unwrap()[0]
                .is_some()
        );
        let wrong = Interval {
            ca: bracket.ca + 1,
            cb: bracket.cb + 1,
            ..bracket
        };
        assert!(
            mesh.refine_fluid::<ROOT_LANES>(&[Some(wrong)], whole)
                .unwrap()[0]
                .is_none()
        );
        mesh.b1[1] = f64::INFINITY;
        assert!(
            mesh.refine_fluid::<ROOT_LANES>(&[Some(bracket)], whole)
                .unwrap()[0]
                .is_none()
        );
    }

    #[test]
    fn rigid_plane_endpoint_uses_exact_fallback() {
        let case = load("PekerisRigidPlane");
        let meshes = Meshes::new(&case).unwrap();
        let mesh = Mesh::new(&meshes, 1).unwrap();
        let whole = interval(&mesh);
        let isolated = mesh.isolate_fluid(whole).unwrap();
        assert!(
            mesh.refine_fluid::<ROOT_LANES>(&isolated[..1], whole)
                .unwrap()[0]
                .is_none()
        );
        let roots = mesh
            .fluid_roots(
                whole.a,
                whole.b,
                whole.cb,
                whole.ca - whole.cb,
                &Refinement::default(),
            )
            .unwrap();
        let exact = mesh.bisect::<1>(whole.a, whole.b, whole.cb, 0..1).unwrap();
        assert_eq!(roots[0].to_bits(), exact[0].to_bits());
    }

    #[test]
    fn fluid_search_pays_and_is_admitted_like_the_original_bisection() {
        // Several groups of eight, so concurrent groups meet the boundary too.
        let case = load("MunkLossless");
        let meshes = Meshes::new(&case).unwrap();
        let mesh = Mesh::new(&meshes, 1).unwrap();
        let whole = interval(&mesh);
        let count = whole.ca - whole.cb;
        assert!(count > 2 * ROOT_LANES, "only {count} modes");
        // The fixed charge the original bisection made, however its search went.
        let original = count * mesh.b1.len() * (ROOT_STEPS + 5);
        let bits = |roots: &[f64]| roots.iter().map(|x| x.to_bits()).collect::<Vec<_>>();
        let mut used = 0;
        let expected = mesh.roots(&Refinement::default(), &mut used).unwrap();
        let brackets = mesh.isolate_fluid(whole).unwrap();
        let original_roots: Vec<_> = oracle(&mesh, &brackets, whole)
            .into_iter()
            .enumerate()
            .map(|(mode, answer)| {
                answer.unwrap_or_else(|| {
                    mesh.bisect::<1>(whole.a, whole.b, whole.cb, mode..mode + 1)
                        .unwrap()[0]
                })
            })
            .collect();
        assert_eq!(bits(&expected), bits(&original_roots));
        assert_eq!(used, original);
        for threads in [1, 2, 4] {
            crate::parallelism::with_threads(std::num::NonZeroUsize::new(threads).unwrap(), || {
                let mut work = MAX_WORK - original;
                assert_eq!(
                    bits(&mesh.roots(&Refinement::default(), &mut work).unwrap()),
                    bits(&expected)
                );
                assert_eq!(work, MAX_WORK);
                let mut work = MAX_WORK - original + 1;
                let error = mesh.roots(&Refinement::default(), &mut work).unwrap_err();
                assert_eq!(error.diagnostics()[0].code, "KR0302");
            });
        }
        let token = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
        let error = crate::cancellation::with_token(std::sync::Arc::clone(&token), || {
            mesh.isolate_fluid(whole)
        })
        .unwrap_err();
        assert_eq!(error.diagnostics()[0].code, "KR0304");
        let error = crate::cancellation::with_token(token, || {
            mesh.refine_fluid::<ROOT_LANES>(&[Some(whole)], whole)
        })
        .unwrap_err();
        assert_eq!(error.diagnostics()[0].code, "KR0304");
    }

    #[test]
    fn fluid_fixture_meshes_keep_exact_mode_counts_and_order() {
        let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
        let mut paths: Vec<_> = std::fs::read_dir(fixtures)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| path.extension().is_some_and(|extension| extension == "env"))
            .collect();
        paths.sort();
        let mut checked = 0;
        for env in paths {
            // Unsupported KRAKENC-only/auxiliary-resource inputs have no real roots.
            let Ok(case) = crate::legacy::load_case(&env, env.with_extension("flp")) else {
                continue;
            };
            if crate::solver::elastic::has_layers(&case)
                || crate::solver::elastic::has_half_space(&case)
            {
                continue;
            }
            let meshes = Meshes::new(&case).unwrap();
            let mesh = Mesh::new(&meshes, 1).unwrap();
            let whole = interval(&mesh);
            let roots = mesh.roots(&Refinement::default(), &mut 0).unwrap();
            let brackets = mesh.isolate_fluid(whole).unwrap();
            assert_oracle(&mesh, &brackets, whole);
            assert_eq!(roots.len(), whole.ca - whole.cb, "{}", env.display());
            for (group, chunk) in roots.chunks(ROOT_LANES).enumerate() {
                let start = group * ROOT_LANES;
                let exact = mesh
                    .bisect::<ROOT_LANES>(whole.a, whole.b, whole.cb, start..start + chunk.len())
                    .unwrap();
                for (&x, &expected) in chunk.iter().zip(&exact) {
                    assert!(
                        (x - expected).abs() <= 2.0 * expected.abs() * RELATIVE_WIDTH,
                        "{}: {x} vs {expected}",
                        env.display()
                    );
                }
            }
            assert!(
                roots.windows(2).all(|pair| pair[0] >= pair[1]),
                "{}",
                env.display()
            );
            checked += 1;
        }
        assert!(checked >= 65, "only {checked} fluid fixture meshes");
    }

    #[test]
    fn raw_history_seeds_certify_later_meshes_and_keep_mode_order() {
        for name in [
            "Pekeris",
            "LayeredFluidThree",
            "SductTrapped",
            "PekerisRigidPlane",
        ] {
            let mut definition = load(name).into_definition();
            definition.max_range_m = 1e20;
            let case = crate::Case::from_definition(definition).unwrap();
            let meshes = Meshes::new(&case).unwrap();
            let mut history = Refinement::<f64>::default();
            let mut certified = 0;
            for multiplier in [1, 2, 4, 8, 16] {
                let mesh = Mesh::new(&meshes, multiplier).unwrap();
                let whole = interval(&mesh);
                let roots = mesh.roots(&history, &mut 0).unwrap();
                assert_eq!(roots.len(), whole.ca - whole.cb);
                assert!(roots.windows(2).all(|pair| pair[0] > pair[1]));
                if multiplier < 4 {
                    assert!(history.seed(0, mesh.h).is_none());
                    let original = mesh.roots(&Refinement::default(), &mut 0).unwrap();
                    assert_eq!(
                        roots.iter().map(|x| x.to_bits()).collect::<Vec<_>>(),
                        original.iter().map(|x| x.to_bits()).collect::<Vec<_>>()
                    );
                } else {
                    for (group, chunk) in roots.chunks(ROOT_LANES).enumerate() {
                        let start = group * ROOT_LANES;
                        let seeds: Vec<_> = (start..start + chunk.len())
                            .map(|i| history.seed(i, mesh.h).map(|x| x.re))
                            .collect();
                        let brackets = mesh.seed_fluid::<ROOT_LANES>(&seeds, whole, start).unwrap();
                        for (k, bracket) in brackets.iter().enumerate() {
                            if let Some(bracket) = bracket {
                                certified += 1;
                                assert_eq!(mesh.count(bracket.a), whole.cb + start + k + 1);
                                assert_eq!(mesh.count(bracket.b), whole.cb + start + k);
                            }
                        }
                        let exact = mesh
                            .bisect::<ROOT_LANES>(
                                whole.a,
                                whole.b,
                                whole.cb,
                                start..start + chunk.len(),
                            )
                            .unwrap();
                        for (&x, expected) in chunk.iter().zip(exact) {
                            assert!(
                                (x - expected).abs() <= 2.0 * expected.abs() * RELATIVE_WIDTH,
                                "{name}, mesh {multiplier}: {x} vs {expected}"
                            );
                        }
                    }
                }
                // Only the raw roots enter history; accept mutates a separate Richardson table.
                history
                    .accept(roots, Some(mesh.h), &case, |roots| mesh.modes(roots))
                    .unwrap();
            }
            assert!(certified > 0, "{name}: no seeded brackets");
        }
    }

    #[test]
    fn invalid_missing_shifted_and_out_of_bounds_seeds_use_full_interval_fallback() {
        let case = load("Pekeris");
        let meshes = Meshes::new(&case).unwrap();
        let mesh = Mesh::new(&meshes, 4).unwrap();
        let whole = interval(&mesh);
        let exact = mesh
            .bisect::<ROOT_LANES>(whole.a, whole.b, whole.cb, 0..2)
            .unwrap();
        for seed in [
            None,
            Some(f64::NAN),
            Some(f64::INFINITY),
            Some(whole.a),
            Some(whole.b),
            Some(exact[1]),
        ] {
            let brackets = mesh.seed_fluid::<ROOT_LANES>(&[seed], whole, 0).unwrap();
            assert!(brackets[0].is_none(), "{seed:?}");
            let roots = mesh.fluid_group(&brackets, whole, 0).unwrap();
            assert_eq!(roots[0].to_bits(), exact[0].to_bits());
        }
        // A seed from a different count/index cannot take the requested mode's place.
        let shifted = Interval {
            cb: whole.cb + 1,
            ..whole
        };
        let brackets = mesh
            .seed_fluid::<ROOT_LANES>(&[Some(exact[0])], shifted, 0)
            .unwrap();
        assert!(brackets[0].is_none());
        assert_eq!(
            mesh.fluid_group(&brackets, shifted, 0).unwrap()[0].to_bits(),
            exact[1].to_bits()
        );
        // A clipped bracket can certify a root arbitrarily close to either limit.
        let narrow = Interval {
            a: exact[0] * (1.0 - 1e-9),
            b: exact[0] * (1.0 + 1e-9),
            ca: whole.cb + 1,
            cb: whole.cb,
        };
        let brackets = mesh
            .seed_fluid::<ROOT_LANES>(&[Some(exact[0])], narrow, 0)
            .unwrap();
        let bracket = brackets[0].unwrap();
        assert_eq!(bracket.a.to_bits(), narrow.a.to_bits());
        assert_eq!(bracket.b.to_bits(), narrow.b.to_bits());
        assert_eq!(
            (mesh.count(bracket.a), mesh.count(bracket.b)),
            (narrow.ca, narrow.cb)
        );
        let token = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
        let error = crate::cancellation::with_token(token, || {
            mesh.seed_fluid::<ROOT_LANES>(&[Some(exact[0])], whole, 0)
        })
        .unwrap_err();
        assert_eq!(error.diagnostics()[0].code, "KR0304");
    }

    #[test]
    fn clustered_seed_brackets_fall_back_instead_of_returning_multiple_modes() {
        let case = load("FluidBoundaryVV");
        let meshes = Meshes::new(&case).unwrap();
        let mut mesh = Mesh::new(&meshes, 4).unwrap();
        let mut whole = interval(&mesh);
        // Translate a vacuum/vacuum operator's spectrum: roots become close relative
        // to their magnitude without changing their separation or boundary conditions.
        let shift = 1e4;
        for layer in &mesh.layers {
            for b in
                &mut mesh.b1[layer.coefficient_start..=layer.coefficient_start + layer.intervals]
            {
                *b += layer.h * layer.h * shift;
            }
        }
        whole.a += shift;
        whole.b += shift;
        whole.ca = mesh.count(whole.a);
        whole.cb = mesh.count(whole.b);
        assert!(whole.ca - whole.cb >= 2);
        let exact = mesh.bisect::<1>(whole.a, whole.b, whole.cb, 0..1).unwrap();
        let brackets = mesh
            .seed_fluid::<ROOT_LANES>(&[Some(exact[0])], whole, 0)
            .unwrap();
        assert!(brackets[0].is_none());
        assert_eq!(
            mesh.fluid_group(&brackets, whole, 0).unwrap()[0].to_bits(),
            exact[0].to_bits()
        );
    }

    fn assert_same_root(actual: f64, expected: f64) {
        assert!(actual.is_finite());
        let tolerance =
            (2.0 * RELATIVE_WIDTH * expected.abs()).max(8.0 * (expected.next_up() - expected));
        assert!(
            (actual - expected).abs() <= tolerance,
            "{actual:e} != {expected:e}"
        );
    }

    #[test]
    fn mesh_three_count_increase_keeps_unseeded_targets_and_rejects_acceptance() {
        let original = load("Pekeris");
        let original_meshes = Meshes::new(&original).unwrap();
        let raw: Vec<_> = [1, 2, 4]
            .into_iter()
            .map(|multiplier| {
                Mesh::new(&original_meshes, multiplier)
                    .unwrap()
                    .roots(&Refinement::default(), &mut 0)
                    .unwrap()
            })
            .collect();
        assert!(raw[1][0] > raw[2][0]);
        assert!(raw[0].last().unwrap() > raw[1].last().unwrap());
        for counts in [[2, 2, 3], [2, 1, 2]] {
            let mut definition = original.clone().into_definition();
            let omega = 2.0 * std::f64::consts::PI * definition.frequency_hz;
            // The leading mode enters cLow on mesh three; optionally a trailing mode exits earlier.
            definition.c_low_mps = omega / raw[1][0].midpoint(raw[2][0]).sqrt();
            if counts[1] == 1 {
                let low = raw[0].last().unwrap().midpoint(*raw[1].last().unwrap());
                definition.c_high_mps = omega * (1.00001 / low).sqrt();
            }
            definition.max_range_m = 1e20;
            let case = crate::Case::from_definition(definition).unwrap();
            let meshes = Meshes::new(&case).unwrap();
            let mut history = Refinement::<f64>::default();
            for (index, multiplier) in [1, 2].into_iter().enumerate() {
                let mesh = Mesh::new(&meshes, multiplier).unwrap();
                let roots = mesh.roots(&history, &mut 0).unwrap();
                assert_eq!(roots.len(), counts[index]);
                assert!(
                    history
                        .accept(roots, Some(mesh.h), &case, |roots| mesh.modes(roots))
                        .unwrap()
                        .is_none()
                );
            }
            let mesh = Mesh::new(&meshes, 4).unwrap();
            let whole = interval(&mesh);
            assert_eq!(whole.ca - whole.cb, counts[2]);
            assert!(history.seed(0, mesh.h).is_some());
            assert!(history.seed(counts[2] - 1, mesh.h).is_none());
            let roots = mesh.roots(&history, &mut 0).unwrap();
            assert_eq!(roots.len(), counts[2]);
            assert!(roots.windows(2).all(|pair| pair[0] > pair[1]));
            for (mode, &root) in roots.iter().enumerate() {
                let exact = mesh
                    .bisect::<1>(whole.a, whole.b, whole.cb, mode..mode + 1)
                    .unwrap()[0];
                assert_same_root(root, exact);
                if history.seed(mode, mesh.h).is_none() {
                    assert_eq!(root.to_bits(), exact.to_bits());
                }
            }
            let error = history
                .accept(roots, Some(mesh.h), &case, |_| panic!("later mesh shapes"))
                .unwrap_err();
            assert_eq!(error.diagnostics()[0].code, "KR0303");
        }
    }

    #[test]
    fn seed_brackets_widen_and_stop_after_eight_attempts() {
        let case = load("Pekeris");
        let meshes = Meshes::new(&case).unwrap();
        let mesh = Mesh::new(&meshes, 4).unwrap();
        let whole = interval(&mesh);
        let mode = whole.ca - whole.cb - 1;
        let target = whole.cb + mode;
        let exact = mesh
            .bisect::<1>(whole.a, whole.b, whole.cb, mode..mode + 1)
            .unwrap()[0];
        // Success on attempts two/eight; a ninth would succeed but must not be tried.
        for (offset, attempt) in [(2e-6, 1), (0.008, 7), (0.02, 8)] {
            let seed = exact * (1.0 + offset);
            assert!(whole.a < seed && seed < whole.b);
            let mut radius = seed.abs() * 1e-6;
            for _ in 0..attempt {
                let ca = mesh.count((seed - radius).max(whole.a));
                let cb = mesh.count((seed + radius).min(whole.b));
                assert_eq!((ca, cb), (target, target));
                radius *= 4.0;
            }
            let a = (seed - radius).max(whole.a);
            let b = (seed + radius).min(whole.b);
            assert_eq!((mesh.count(a), mesh.count(b)), (target + 1, target));
            let brackets = mesh
                .seed_fluid::<ROOT_LANES>(&[Some(seed)], whole, mode)
                .unwrap();
            if attempt < 8 {
                let bracket = brackets[0].expect("certified widened bracket");
                assert_eq!(bracket.a.to_bits(), a.to_bits());
                assert_eq!(bracket.b.to_bits(), b.to_bits());
                assert_eq!((bracket.ca, bracket.cb), (target + 1, target));
            } else {
                assert!(brackets[0].is_none());
            }
            let root = mesh.fluid_group(&brackets, whole, mode).unwrap()[0];
            assert_same_root(root, exact);
            if attempt == 8 {
                assert_eq!(root.to_bits(), exact.to_bits());
            }
        }
    }

    #[test]
    fn seed_brackets_reject_counts_outside_the_whole_interval() {
        let case = load("Pekeris");
        let meshes = Meshes::new(&case).unwrap();
        let mesh = Mesh::new(&meshes, 4).unwrap();
        let whole = interval(&mesh);
        let exact = mesh.bisect::<1>(whole.a, whole.b, whole.cb, 0..1).unwrap()[0];
        // Corrupt only the whole-interval labels to exercise each monotonicity guard.
        for (seed, bad, violations) in [
            (
                exact * (1.0 - 2e-6),
                Interval {
                    ca: whole.cb,
                    ..whole
                },
                (true, false),
            ),
            (
                exact * (1.0 + 2e-6),
                Interval {
                    cb: whole.cb + 1,
                    ..whole
                },
                (false, true),
            ),
        ] {
            assert!(whole.a < seed && seed < whole.b);
            let radius = seed.abs() * 1e-6;
            let ca = mesh.count((seed - radius).max(whole.a));
            let cb = mesh.count((seed + radius).min(whole.b));
            assert_eq!(ca, cb); // Neither a sign-changing target nor the ca != cb rejection.
            assert_eq!((ca > bad.ca, cb < bad.cb), violations);
            let brackets = mesh
                .seed_fluid::<ROOT_LANES>(&[Some(seed)], bad, 0)
                .unwrap();
            assert!(brackets[0].is_none());
            let root = mesh.fluid_group(&brackets, bad, 0).unwrap()[0];
            let fallback = mesh.bisect::<1>(bad.a, bad.b, bad.cb, 0..1).unwrap()[0];
            assert_eq!(root.to_bits(), fallback.to_bits());
        }
    }

    #[test]
    fn exact_root_and_bracket_edge_seeds_keep_the_target() {
        let case = load("FluidBoundaryVV");
        let meshes = Meshes::new(&case).unwrap();
        let mut mesh = Mesh::new(&meshes, 4).unwrap();
        // An even vacuum/vacuum mesh with zero diagonal at x = 1 has an exact shooting zero.
        assert_eq!(mesh.layers[0].intervals % 2, 0);
        mesh.h = 1.0;
        mesh.layers[0].h = 1.0;
        mesh.b1.fill(1.0);
        let root = 1.0_f64;
        assert_eq!(
            mesh.fluid_dispersion([root])[0].value.abs().to_bits(),
            0.0_f64.to_bits()
        );
        let whole = Interval {
            a: 0.99,
            b: 1.01,
            ca: mesh.count(0.99),
            cb: mesh.count(1.01),
        };
        assert_eq!(whole.ca - whole.cb, 1);
        let exact = mesh.bisect::<1>(whole.a, whole.b, whole.cb, 0..1).unwrap()[0];
        assert_same_root(exact, root);
        let left_edge = root / (1.0 - 1e-6);
        let right_edge = root / (1.0 + 1e-6);
        assert_eq!(
            (left_edge - left_edge.abs() * 1e-6).to_bits(),
            root.to_bits()
        );
        assert_eq!(
            (right_edge + right_edge.abs() * 1e-6).to_bits(),
            root.to_bits()
        );
        let mut edge_fallbacks = 0;
        for seed in [
            root,
            root.next_down(),
            root.next_up(),
            left_edge,
            right_edge,
        ] {
            let brackets = mesh
                .seed_fluid::<ROOT_LANES>(&[Some(seed)], whole, 0)
                .unwrap();
            let bracket = brackets[0].expect("exact or edge seed certifies the target");
            assert_eq!(
                (mesh.count(bracket.a), mesh.count(bracket.b)),
                (whole.ca, whole.cb)
            );
            let actual = mesh.fluid_group(&brackets, whole, 0).unwrap()[0];
            assert_same_root(actual, exact);
            if bracket.a.to_bits() == root.to_bits() || bracket.b.to_bits() == root.to_bits() {
                assert!(mesh.refine_fluid::<ROOT_LANES>(&brackets, whole).unwrap()[0].is_none());
                assert_eq!(actual.to_bits(), exact.to_bits());
                edge_fallbacks += 1;
            }
        }
        assert!(edge_fallbacks > 0, "no exact shooting endpoint exercised");
    }

    #[test]
    fn seeded_roots_at_extreme_scales_stay_finite_and_ordered() {
        let case = load("FluidBoundaryVV");
        let meshes = Meshes::new(&case).unwrap();
        for scale in [1e-307_f64, 1e-200, 1.0, 1e200, 1e300] {
            let mut mesh = Mesh::new(&meshes, 4).unwrap();
            let original = interval(&mesh);
            // Rescale squared wavenumber while preserving the dimensionless vacuum operator.
            mesh.h /= scale.sqrt();
            for layer in &mut mesh.layers {
                layer.h /= scale.sqrt();
            }
            let whole = Interval {
                a: original.a * scale,
                b: original.b * scale,
                ..original
            };
            assert_eq!(
                (mesh.count(whole.a), mesh.count(whole.b)),
                (whole.ca, whole.cb)
            );
            let mut roots = Vec::new();
            for mode in 0..whole.ca - whole.cb {
                let exact = mesh
                    .bisect::<1>(whole.a, whole.b, whole.cb, mode..mode + 1)
                    .unwrap()[0];
                let brackets = mesh
                    .seed_fluid::<ROOT_LANES>(&[Some(exact)], whole, mode)
                    .unwrap();
                let bracket = brackets[0].expect("extreme-scale seed certifies the target");
                assert_eq!(
                    (mesh.count(bracket.a), mesh.count(bracket.b)),
                    (whole.cb + mode + 1, whole.cb + mode)
                );
                let root = mesh.fluid_group(&brackets, whole, mode).unwrap()[0];
                assert_same_root(root, exact);
                roots.push(root);
            }
            assert_eq!(roots.len(), whole.ca - whole.cb);
            assert!(roots.windows(2).all(|pair| pair[0] > pair[1]));
        }
    }

    #[test]
    fn elastic_count_reversal_reproduces_in_scalar_and_packed_lanes() {
        let case = load("ElasticHalfBottomC");
        let meshes = Meshes::new(&case).unwrap();
        let mesh = Mesh::new(&meshes, 1).unwrap();
        let b = f64::from_bits(4_585_822_887_773_648_427);
        let a = b.next_down();
        assert_eq!((mesh.count(a), mesh.count(b)), (3, 4));
        assert_eq!(
            mesh.count_lanes([a, b, a, b, a, b, a, b]),
            [3, 4, 3, 4, 3, 4, 3, 4]
        );
    }
}

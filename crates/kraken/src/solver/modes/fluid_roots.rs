//! Fluid-only isolation and safeguarded Illinois refinement.
//!
//! Invariant: C(x) is nonincreasing, and [a, b] owns precisely the targets
//! C(b)..C(a). A single-root interval must also bracket the continuous fluid
//! shooting function. Elastic impedance poles do not satisfy this invariant.
//! Failed isolation, sign, finiteness or final count checks leave the affected
//! targets to the original full-interval bisection, never to a guessed root.
use super::{DiagnosticReport, Mesh, ROOT_LANES, ROOT_STEPS};

const RELATIVE_WIDTH: f64 = 1e-13;
const REFINE_STEPS: usize = 160;

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

// Keep independent multiply/subtract chains visible to the vectorizer.
#[inline(never)]
fn shoot_interior<const K: usize>(
    rows: &[f64],
    shift: [f64; K],
    previous: &mut [f64; K],
    current: &mut [f64; K],
    power: &mut [i32; K],
) -> [f64; K] {
    let mut older = [0.0; K];
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
        older = *previous;
        *previous = *current;
        *current = next;
        if current.iter().any(|v| v.abs() > 1e100) {
            for k in 0..K {
                if current[k].abs() > 1e100 {
                    current[k] *= 2_f64.powi(-256);
                    previous[k] *= 2_f64.powi(-256);
                    older[k] *= 2_f64.powi(-256);
                    power[k] += 256;
                }
            }
        }
    }
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

    #[allow(clippy::needless_range_loop)] // Each lane owns its bracket and stopping point.
    fn refine_fluid<const K: usize>(
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
        let answers = self.refine_fluid::<ROOT_LANES>(intervals, whole)?;
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
    ) -> Result<Vec<f64>, DiagnosticReport> {
        let whole = Interval {
            a: low,
            b: high,
            ca: above + count,
            cb: above,
        };
        let isolated = self.isolate_fluid(whole)?;
        let groups = count.div_ceil(ROOT_LANES);
        let estimated_work = count.saturating_mul(self.b1.len()).saturating_mul(32);
        let mut roots = Vec::with_capacity(count);
        // Fixed groups: neither membership nor arithmetic depends on worker splits.
        for result in crate::parallelism::map(groups, estimated_work, |group| {
            let start = group * ROOT_LANES;
            self.fluid_group(
                &isolated[start..count.min(start + ROOT_LANES)],
                whole,
                start,
            )
        }) {
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
            .fluid_roots(whole.a, whole.b, whole.cb, whole.ca - whole.cb)
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
        let expected = mesh.roots(&mut used).unwrap();
        assert_eq!(used, original);
        for threads in [1, 2, 4] {
            crate::parallelism::with_threads(std::num::NonZeroUsize::new(threads).unwrap(), || {
                let mut work = MAX_WORK - original;
                assert_eq!(bits(&mesh.roots(&mut work).unwrap()), bits(&expected));
                assert_eq!(work, MAX_WORK);
                let mut work = MAX_WORK - original + 1;
                let error = mesh.roots(&mut work).unwrap_err();
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
            let roots = mesh.roots(&mut 0).unwrap();
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

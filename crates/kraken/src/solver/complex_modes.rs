// Adapted from Acoustics Toolbox v2023.5 Kraken/krakenc.f90,
// Kraken/InverseIterationMod.f90 and misc/RootFinderSecantMod.f90.
// Copyright (C) 2009 Michael B. Porter. GPL-3.0-or-later; see LICENSE.
//! N/C/P/S fluid stacks with smooth V/R/A boundaries and bounded Richardson
//! extrapolation; analytic Munk and N/C table boundaries remain single-layer.
use crate::solver::profile::Profile;
use crate::solver::{elastic::complex_speed, error};
use crate::{BottomBoundary, Case, DiagnosticReport, ModeSet, NormalMode, SurfaceBoundary};
use num_complex::Complex64;
use std::f64::consts::PI;

const MAX_ROOT_WORK: usize = 300_000_000;
const SECANT_RELATIVE_TOLERANCE: f64 = 1e-14;

#[allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::float_cmp,
    clippy::too_many_lines
)]
pub(super) fn solve(case: &Case) -> Result<ModeSet, DiagnosticReport> {
    let omega = 2.0 * PI * case.frequency_hz;
    let profiles = crate::solver::layers::iter(case)
        .map(|layer| Profile::new_layer(case, layer))
        .collect::<Result<Vec<_>, _>>()?;
    let bottom_c = complex_speed(
        case.bottom_sound_speed_mps,
        case.bottom_attenuation_db_per_wavelength,
    );
    let surface_c = complex_speed(
        case.surface_sound_speed_mps,
        case.surface_attenuation_db_per_wavelength,
    );
    let surface_k2 = if case.surface_boundary.is_half_space() {
        (Complex64::new(omega, 0.0) / surface_c).powi(2)
    } else {
        Complex64::new(0.0, 0.0)
    };
    let tabulated = case.bottom_boundary.is_tabulated() || case.surface_boundary.is_tabulated();
    let bottom_k2 = if case.bottom_boundary.is_half_space() {
        (Complex64::new(omega, 0.0) / bottom_c).powi(2)
    } else {
        Complex64::new(0.0, 0.0)
    };
    let low_k2 = (omega / case.c_high_mps).powi(2);
    if !low_k2.is_finite() || !bottom_k2.re.is_finite() || !bottom_k2.im.is_finite() {
        return Err(error(
            "KR0302",
            "mesh coefficients exceed numeric range",
            "mesh_points",
        ));
    }

    let water_loss = case
        .water_attenuation_db_per_wavelength
        .iter()
        .any(|&a| a != 0.0);
    // Separate ducts can interleave nearly coincident roots; spacing is not one smooth sequence.
    let stratified_trapped = !water_loss
        && case.c_high_mps <= case.bottom_sound_speed_mps
        && (case.interpolation == crate::Interpolation::AnalyticMunk
            || case
                .sound_speed_profile
                .windows(2)
                .any(|p| p[0].sound_speed_mps != p[1].sound_speed_mps));
    let mut refinement = crate::solver::refinement::Refinement::<Complex64>::default();
    let mut work = 0;
    for multiplier in crate::solver::refinement::MULTIPLIERS {
        let layers = crate::solver::layers::mesh_layers(case, multiplier)?;
        let h = layers[0].h;
        let mut min_speed = f64::INFINITY;
        let mut b = Vec::new();
        for ((layer, profile), material) in layers
            .iter()
            .zip(&profiles)
            .zip(crate::solver::layers::iter(case))
        {
            let lossy = material.loss.iter().any(|&a| a != 0.0);
            for i in 0..=layer.intervals {
                let c = profile.mesh_complex_speed(i, layer.intervals);
                min_speed = min_speed.min(c.re);
                b.push(if lossy {
                    -2.0 + Complex64::new(layer.h * layer.h * omega.powi(2), 0.0) / c.powi(2)
                } else if tabulated {
                    Complex64::new(
                        -2.0 + (Complex64::new(layer.h * layer.h * omega.powi(2), 0.0) / c.powi(2))
                            .re,
                        0.0,
                    )
                } else {
                    Complex64::new(-2.0 + layer.h * layer.h * (omega / c.re).powi(2), 0.0)
                });
            }
        }
        let mut decreasing_slowness = false;
        let separated_ducts = stratified_trapped
            && b.windows(2).any(|pair| {
                decreasing_slowness |= pair[1].re < pair[0].re;
                decreasing_slowness && pair[1].re > pair[0].re
            });
        let inside_h = layers.last().unwrap().h;
        let inside_c = pekeris_root(if water_loss {
            Complex64::new(omega * omega * inside_h * inside_h, 0.0) / (2.0 + b[b.len() - 1])
        } else {
            Complex64::new(
                omega * omega * inside_h * inside_h / (2.0 + b[b.len() - 1].re),
                0.0,
            )
        });
        let bottom = Bottom {
            case,
            k2: bottom_k2,
            surface_k2,
            water_k2: (omega * omega / (inside_c * inside_c)).re,
            top_solids: crate::solver::elastic::SolidMesh::build(case, multiplier, true, false)?,
            lower_solids: crate::solver::elastic::SolidMesh::build(case, multiplier, false, false)?,
        };
        let seed_h = bottom
            .top_solids
            .first()
            .map_or(h, crate::solver::elastic::SolidMesh::spacing);
        let elastic = crate::solver::elastic::has_half_space(case)
            || crate::solver::elastic::has_layers(case);
        if elastic {
            min_speed = crate::solver::elastic::minimum_speed(
                case,
                min_speed,
                &bottom.top_solids,
                &bottom.lower_solids,
            );
        }
        let water_k2 = (omega / min_speed).powi(2);
        if !water_k2.is_finite() || b.iter().any(|x| !x.re.is_finite() || !x.im.is_finite()) {
            return Err(error(
                "KR0302",
                "mesh coefficients exceed numeric range",
                "mesh_points",
            ));
        }
        // The minimum water speed bounds the first root; deflation keeps
        // subsequent secants distinct.
        // A wavenumber-dependent table may introduce extra roots; the water-only
        // estimate is not its root bound. Keep the existing 20k/300m ceilings.
        let guesses = if tabulated || layers.len() > 1 || elastic {
            crate::MAX_MODE_LIMIT
        } else {
            (case.total_depth_m() * (water_k2 - low_k2).max(0.0).sqrt() / PI).ceil() as usize + 1
        };
        if guesses > crate::MAX_MODE_LIMIT || guesses == 0 {
            return Err(error(
                "KR0302",
                "complex root work limit exceeded",
                "mesh_points",
            ));
        }
        // Continuing from a deflated root can skip the next mode, including below a rigid top.
        let predict_spacing = !water_loss
            && !separated_ducts
            && !elastic
            && layers.len() == 1
            && matches!(
                case.surface_boundary,
                SurfaceBoundary::Vacuum | SurfaceBoundary::Rigid
            )
            && case.bottom_boundary == BottomBoundary::FluidHalfSpace
            && case.c_high_mps <= case.bottom_sound_speed_mps
            && case.c_high_mps <= 10.0 * case.c_low_mps;
        let mut roots: Vec<Complex64> = Vec::new();
        let mut reached_lower_limit = false;
        for index in 1..=guesses {
            let vertical = (index as f64 - 0.5) * PI / case.water_depth_m;
            // ponytail: wide spectral intervals use the previous-root seed from
            // Fortran; revisit the 10x cutoff if a medium-width spectrum fails.
            let guess = if predict_spacing && roots.len() >= 2 {
                let previous = roots[roots.len() - 1];
                previous + (previous - roots[roots.len() - 2])
            } else if tabulated
                || separated_ducts
                || elastic
                || layers.len() > 1
                || (case.surface_boundary != SurfaceBoundary::Vacuum && !predict_spacing)
                || case.bottom_boundary != BottomBoundary::FluidHalfSpace
            {
                roots.last().copied().unwrap_or_else(|| {
                    Complex64::new(
                        omega * omega / case.c_low_mps.max(f64::from(0.99_f32) * min_speed).powi(2),
                        0.0,
                    )
                }) * f64::from(1.000_01_f32)
            } else if case.c_high_mps > 10.0 * case.c_low_mps {
                roots.last().map_or_else(
                    || Complex64::new(water_k2 - vertical * vertical, 0.0),
                    |&previous| previous * 1.000_01,
                )
            } else {
                Complex64::new(water_k2 - vertical * vertical, 0.0)
            };
            // Solve2 seeds refined meshes with unmodified EVMat roots, then
            // Neville interpolation in h²; restarting from above can skip roots.
            let guess = refinement.seed(index - 1, seed_h).unwrap_or(guess);
            let root = secant(guess, &roots, &b, &layers, &bottom, &mut work)?;
            if ((tabulated || elastic) && root.sqrt().re < omega / case.c_high_mps)
                || (!(tabulated || elastic) && root.re <= low_k2)
            {
                reached_lower_limit = true;
                break;
            }
            // Even excluded roots must be deflated so the next secant finds a new mode.
            if roots.iter().any(|&previous: &Complex64| {
                (root - previous).norm()
                    < root.norm().max(previous.norm()) * b.len() as f64 * SECANT_RELATIVE_TOLERANCE
            }) {
                return Err(error(
                    "KR0303",
                    "complex root search repeated a mode",
                    "phase_speed_limits",
                ));
            }
            roots.push(root);
        }
        if !reached_lower_limit {
            return Err(error(
                "KR0303",
                "complex root search cannot prove the full spectral interval",
                "phase_speed_limits",
            ));
        }
        if let Some(result) = refinement.accept(roots, seed_h, case, |root| {
            mode(case, &b, &layers, omega, bottom_c, &bottom, root)
        })? {
            return Ok(result);
        }
    }
    Err(error(
        "KR0303",
        "KRAKENC eigenvalue extrapolation did not converge within five meshes",
        "max_range_m",
    ))
}

// The upstream PekerisRoot branch is not the principal complex square root.
pub(crate) fn pekeris_root(z: Complex64) -> Complex64 {
    if z.re >= 0.0 {
        z.sqrt()
    } else {
        Complex64::new(0.0, 1.0) * (-z).sqrt()
    }
}

struct Bottom<'a> {
    case: &'a Case,
    k2: Complex64,
    surface_k2: Complex64,
    water_k2: f64,
    top_solids: Vec<crate::solver::elastic::SolidMesh>,
    lower_solids: Vec<crate::solver::elastic::SolidMesh>,
}

impl Bottom<'_> {
    fn evaluate(&self, x: Complex64) -> (Complex64, Complex64, i32) {
        if !self.lower_solids.is_empty() {
            return crate::solver::elastic::cap_impedance(
                &self.case.bottom_boundary,
                x,
                2.0 * PI * self.case.frequency_hz,
                self.case.bottom_sound_speed_mps,
                self.case.bottom_density_g_cm3,
                self.case.bottom_attenuation_db_per_wavelength,
                &self.lower_solids,
                false,
                false,
            );
        }
        boundary_impedance(
            &self.case.bottom_boundary,
            x,
            self.k2,
            self.case.bottom_density_g_cm3,
            self.water_k2,
            self.case.frequency_hz,
            self.case.bottom_sound_speed_mps,
            self.case.bottom_attenuation_db_per_wavelength,
        )
    }

    fn admittance(&self, x: Complex64) -> Complex64 {
        let (f, g, _) = self.evaluate(x);
        f / g
    }

    fn surface(&self, x: Complex64) -> (Complex64, Complex64, i32) {
        if !self.top_solids.is_empty() {
            return crate::solver::elastic::cap_impedance(
                &self.case.surface_boundary,
                x,
                2.0 * PI * self.case.frequency_hz,
                self.case.surface_sound_speed_mps,
                self.case.surface_density_g_cm3,
                self.case.surface_attenuation_db_per_wavelength,
                &self.top_solids,
                true,
                false,
            );
        }
        let (f, g, power) = boundary_impedance(
            &self.case.surface_boundary,
            x,
            self.surface_k2,
            self.case.surface_density_g_cm3,
            self.water_k2,
            self.case.frequency_hz,
            self.case.surface_sound_speed_mps,
            self.case.surface_attenuation_db_per_wavelength,
        );
        (f, -g, power)
    }
}

#[allow(clippy::too_many_arguments)]
fn boundary_impedance(
    boundary: &BottomBoundary,
    x: Complex64,
    k2: Complex64,
    density: f64,
    water_k2: f64,
    frequency: f64,
    cp: f64,
    loss: f64,
) -> (Complex64, Complex64, i32) {
    match boundary {
        BottomBoundary::Vacuum => (1.0.into(), 0.0.into(), 0),
        BottomBoundary::Rigid => (0.0.into(), 1.0.into(), 0),
        BottomBoundary::FluidHalfSpace => (pekeris_root(x - k2), density.into(), 0),
        BottomBoundary::ElasticHalfSpace { .. } => {
            let (f, g) = crate::solver::elastic::half_space(
                boundary,
                x,
                2.0 * PI * frequency,
                cp,
                density,
                loss,
                false,
            );
            (f, g, 0)
        }
        _ => crate::solver::reflection::impedance(boundary, x, water_k2),
    }
}

#[allow(clippy::many_single_char_names)]
fn dispersion(
    x: Complex64,
    roots: &[Complex64],
    b: &[Complex64],
    layers: &[crate::solver::layers::MeshLayer],
    bottom: &Bottom<'_>,
) -> (Complex64, i32) {
    if layers.len() > 1 || crate::solver::elastic::has_layers(bottom.case) {
        return layered_dispersion(x, roots, b, layers, bottom);
    }
    let h = layers[0].h;
    let water_rho = layers[0].density;
    let (f, g, mut power) = bottom.evaluate(x);
    let mut prev = -2.0 * g;
    let tabulated =
        bottom.case.bottom_boundary.is_tabulated() || bottom.case.surface_boundary.is_tabulated();
    let mut current = if tabulated {
        tabulated_shoot_step(b[b.len() - 1] - h * h * x, g, 2.0 * h * f * water_rho)
    } else {
        (b[b.len() - 1] - h * h * x) * g - 2.0 * h * f * water_rho
    };
    // Keep the decimal scaling exponent: secant compares evaluations at
    // different scales after shooting and deflation (as in RootFinderSecant).
    let mut previous2 = prev;
    for &coefficient in b[..b.len() - 1].iter().rev() {
        let next = if tabulated {
            tabulated_shoot_step(h * h * x - coefficient, current, prev)
        } else {
            (h * h * x - coefficient) * current - prev
        };
        previous2 = prev;
        prev = current;
        current = next;
        while current.re.is_finite() && current.re.abs() > 1e50 {
            previous2 *= 1e-50;
            prev *= 1e-50;
            current *= 1e-50;
            power += 50;
        }
    }
    // Preserve the established vacuum-top grouping; general top impedance uses
    // f = -(p2-p0)/(2h)/rho and g = -p1 after shooting upwards.
    let mut value = if bottom.case.surface_boundary == SurfaceBoundary::Vacuum {
        prev
    } else {
        let (f_top, g_top, top_power) = bottom.surface(x);
        power += top_power;
        -(current - previous2) / (2.0 * h) / water_rho * g_top + prev * f_top
    };
    for &root in roots {
        value /= x - root;
        if value.re.abs() > 1e50 {
            value *= 1e-50;
            power += 50;
        }
        if value.re.abs() < 1e-50 && value.norm() > 0.0 {
            value *= 1e50;
            power -= 50;
        }
    }
    (value, power)
}

#[allow(clippy::many_single_char_names)]
fn layered_dispersion(
    x: Complex64,
    roots: &[Complex64],
    b: &[Complex64],
    layers: &[crate::solver::layers::MeshLayer],
    bottom: &Bottom<'_>,
) -> (Complex64, i32) {
    let (mut f, mut g, mut power) = bottom.evaluate(x);
    for layer in layers.iter().rev() {
        let shift = layer.h * layer.h * x;
        let coefficients = &b[layer.coefficient_start..=layer.coefficient_start + layer.intervals];
        let mut p0 = Complex64::new(0.0, 0.0);
        let mut p1 = -2.0 * g;
        let mut p2 = tabulated_shoot_step(
            coefficients[layer.intervals] - shift,
            g,
            2.0 * layer.h * f * layer.density,
        );
        for &coefficient in coefficients[..layer.intervals].iter().rev() {
            p0 = p1;
            p1 = p2;
            p2 = tabulated_shoot_step(shift - coefficient, p1, p0);
            while p2.re.is_finite() && p2.re.abs() > 1e50 {
                p0 *= 1e-50;
                p1 *= 1e-50;
                p2 *= 1e-50;
                power += 50;
            }
        }
        // GNU -ffast-math combines the two real divisors before division.
        f = -(p2 - p0) / (layer.density * (2.0 * layer.h));
        g = -p1;
    }
    let (f_top, g_top, top_power) = bottom.surface(x);
    power += top_power;
    // Retain the established vacuum-top sign; a global sign does not affect roots.
    let mut value = f * g_top - g * f_top;
    for &root in roots {
        value /= x - root;
        if value.re.abs() > 1e50 {
            value *= 1e-50;
            power += 50;
        }
        if value.re.abs() < 1e-50 && value.norm() > 0.0 {
            value *= 1e50;
            power -= 50;
        }
    }
    (value, power)
}

fn tabulated_shoot_step(
    coefficient: Complex64,
    current: Complex64,
    previous: Complex64,
) -> Complex64 {
    // GNU Fortran 12.2.0 -ffast-math reassociates the real recurrence this way.
    // Table and multi-layer previous-root seeds are sensitive to single-ulp changes.
    Complex64::new(
        (coefficient.re * current.re - previous.re) - coefficient.im * current.im,
        coefficient.re * current.im + coefficient.im * current.re - previous.im,
    )
}

#[allow(clippy::cast_precision_loss, clippy::too_many_arguments)]
fn secant(
    mut x: Complex64,
    roots: &[Complex64],
    b: &[Complex64],
    layers: &[crate::solver::layers::MeshLayer],
    bottom: &Bottom<'_>,
    work: &mut usize,
) -> Result<Complex64, DiagnosticReport> {
    let mut evaluate = |x| {
        let table_work: usize = [&bottom.case.bottom_boundary, &bottom.case.surface_boundary]
            .iter()
            .map(|boundary| match boundary {
                BottomBoundary::Reflection(points) => points.len().ilog2() as usize + 3,
                BottomBoundary::Impedance { points, .. } => points.len().ilog2() as usize + 8,
                _ => 0,
            })
            .sum();
        let solid_points: usize = bottom
            .top_solids
            .iter()
            .chain(&bottom.lower_solids)
            .map(crate::solver::elastic::SolidMesh::points)
            .sum();
        *work += b.len() + roots.len() + table_work + 5 * solid_points;
        if *work > MAX_ROOT_WORK {
            return Err(error(
                "KR0302",
                "complex root work limit exceeded",
                "mesh_points",
            ));
        }
        let value = dispersion(x, roots, b, layers, bottom);
        if !value.0.re.is_finite() || !value.0.im.is_finite() {
            return Err(error(
                "KR0303",
                "non-finite complex boundary/dispersion",
                if bottom.case.surface_boundary.is_tabulated() {
                    "surface_boundary"
                } else {
                    "bottom_boundary"
                },
            ));
        }
        Ok(value)
    };
    let solid_points: usize = bottom
        .top_solids
        .iter()
        .chain(&bottom.lower_solids)
        .map(crate::solver::elastic::SolidMesh::points)
        .sum();
    let tolerance = x.norm() * (b.len() + solid_points) as f64 * SECANT_RELATIVE_TOLERANCE;
    let mut previous = x + 100.0 * tolerance;
    let (mut f_previous, mut previous_power) = evaluate(previous)?;
    for _ in 0..1000 {
        let (f, power) = evaluate(x)?;
        let numerator = f * (x - previous);
        let denominator = f - f_previous * 10_f64.powi(previous_power - power);
        if !denominator.re.is_finite()
            || !denominator.im.is_finite()
            || !numerator.re.is_finite()
            || !numerator.im.is_finite()
        {
            break;
        }
        let shift = if numerator.norm() >= (denominator * x).norm() {
            Complex64::new(0.1 * tolerance, 0.0)
        } else {
            numerator / denominator
        };
        let next = x - shift;
        if !next.re.is_finite() || !next.im.is_finite() {
            break;
        }
        if (next - x).norm() + (next - previous).norm() < tolerance {
            return Ok(next);
        }
        previous = x;
        f_previous = f;
        previous_power = power;
        x = next;
    }
    Err(error(
        "KR0303",
        "complex secant did not converge",
        "phase_speed_limits",
    ))
}

// Complex tridiagonal inverse iteration from upstream InverseIterationMod.f90.
#[allow(
    clippy::cast_precision_loss,
    clippy::float_cmp,
    clippy::many_single_char_names
)]
fn inverse_iteration(d: &[Complex64], e: &[f64]) -> Result<Vec<Complex64>, DiagnosticReport> {
    let n = d.len();
    let eps3 = 100.0
        * f64::EPSILON
        * (d.iter().map(|v| v.re.abs() + v.im.abs()).sum::<f64>()
            + e[1..n].iter().map(|v| v.abs()).sum::<f64>());
    let eps4 = n as f64 * eps3;
    let mut a = vec![Complex64::new(0.0, 0.0); n];
    let mut b = a.clone();
    let mut c = a.clone();
    let mut multipliers = a.clone();
    let mut swapped = vec![false; n];
    let mut u = d[0];
    let mut v = Complex64::new(e[1], 0.0);
    for i in 1..n {
        if e[i].abs() >= u.norm() {
            let ratio = u / e[i];
            multipliers[i] = ratio;
            swapped[i] = true;
            a[i - 1] = e[i].into();
            b[i - 1] = d[i];
            c[i - 1] = e[i + 1].into();
            u = v - ratio * d[i];
            v = -ratio * e[i + 1];
        } else {
            let ratio = e[i] / u;
            multipliers[i] = ratio;
            a[i - 1] = u;
            b[i - 1] = v;
            u = d[i] - ratio * v;
            v = e[i + 1].into();
        }
    }
    a[n - 1] = if u == Complex64::new(0.0, 0.0) {
        Complex64::new(eps3, 0.0)
    } else {
        u
    };
    c[n - 2] = Complex64::new(0.0, 0.0);
    let mut phi = vec![Complex64::new(eps4 / (n as f64).sqrt(), 0.0); n];
    for iteration in 0..3 {
        let mut next = Complex64::new(0.0, 0.0);
        let mut next2 = next;
        for i in (0..n).rev() {
            phi[i] = (phi[i] - b[i] * next - c[i] * next2) / a[i];
            next2 = next;
            next = phi[i];
        }
        let norm: f64 = phi.iter().map(|v| v.re.abs() + v.im.abs()).sum();
        if !norm.is_finite() || norm == 0.0 {
            break;
        }
        // One large solve can still contain starting-vector error; polish before accepting it.
        if iteration > 0 && norm >= 1.0 {
            return Ok(phi);
        }
        for value in &mut phi {
            *value *= eps4 / norm;
        }
        for i in 1..n {
            if swapped[i] {
                phi.swap(i, i - 1);
            }
            let before = phi[i - 1];
            phi[i] -= multipliers[i] * before;
        }
    }
    Err(error("KR0303", "complex inverse iteration failed", "modes"))
}

#[allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::many_single_char_names,
    clippy::too_many_lines
)]
fn mode(
    case: &Case,
    b: &[Complex64],
    layers: &[crate::solver::layers::MeshLayer],
    omega: f64,
    bottom_c: Complex64,
    bottom: &Bottom<'_>,
    x: Complex64,
) -> Result<(NormalMode, Complex64), DiagnosticReport> {
    let last = layers.last().unwrap();
    let n = last.node_start + last.intervals + 1;
    let h = layers[0].h;
    let shift = h * h * x;
    let h_rho = h * layers[0].density;
    let mut d = vec![Complex64::new(0.0, 0.0); n];
    let mut e = vec![0.0; n + 1];
    for (medium, layer) in layers.iter().enumerate() {
        let shift = layer.h * layer.h * x;
        let h_rho = layer.h * layer.density;
        for i in 0..=layer.intervals {
            let j = layer.node_start + i;
            let diagonal = (b[layer.coefficient_start + i] - shift) / h_rho;
            d[j] = if medium > 0 && i == 0 {
                (d[j] + diagonal) / 2.0
            } else {
                diagonal
            };
            if i > 0 {
                e[j] = 1.0 / h_rho;
            }
        }
    }
    e[n] = 1.0 / (last.h * last.density);
    let (f_top, g_top, _) = bottom.surface(x);
    if g_top == Complex64::new(0.0, 0.0) {
        d[0] = Complex64::new(1.0, 0.0);
        e[1] = 0.0;
    } else {
        d[0] = (b[0] - shift) / h_rho / 2.0 + f_top / g_top;
    }
    let (f_bot, g_bot, _) = bottom.evaluate(x);
    if g_bot == Complex64::new(0.0, 0.0) {
        d[n - 1] = Complex64::new(1.0, 0.0);
        e[n - 1] = 0.0;
    } else {
        d[n - 1] = (b[b.len() - 1] - last.h * last.h * x) / (2.0 * (last.h * last.density))
            - f_bot / g_bot;
    }
    let mut phi = inverse_iteration(&d, &e)?;
    let mut sq_norm = Complex64::new(0.0, 0.0);
    let mut slow = Complex64::new(0.0, 0.0);
    for layer in layers {
        for i in 0..=layer.intervals {
            let value = phi[layer.node_start + i];
            let weight = if i == 0 || i == layer.intervals {
                0.5
            } else {
                1.0
            };
            let mass = weight * layer.h / layer.density * value * value;
            sq_norm += mass;
            slow +=
                mass * (b[layer.coefficient_start + i] + 2.0) / (omega * omega * layer.h * layer.h);
        }
    }
    if case.surface_boundary.is_half_space() {
        let c = complex_speed(
            case.surface_sound_speed_mps,
            case.surface_attenuation_db_per_wavelength,
        );
        slow += phi[0].powi(2)
            / (2.0 * (x - bottom.surface_k2).sqrt())
            / (case.surface_density_g_cm3 * c.powi(2));
    }
    if case.bottom_boundary.is_half_space() {
        let gamma = (x - bottom.k2).sqrt();
        slow += phi[n - 1].powi(2) / (2.0 * gamma * case.bottom_density_g_cm3 * bottom_c.powi(2));
    }
    let x1 = x * 0.999_999_9;
    let x2 = x * 1.000_000_1;
    let (ft1, gt1, _) = bottom.surface(x1);
    let (ft2, gt2, _) = bottom.surface(x2);
    let top_derivative = if gt1 == Complex64::new(0.0, 0.0) {
        0.0.into()
    } else {
        (ft2 / gt2 - ft1 / gt1) / (x2 - x1)
    };
    let derivative = if !bottom.lower_solids.is_empty() {
        (bottom.admittance(x2) - bottom.admittance(x1)) / (x2 - x1)
    } else if case.bottom_boundary == BottomBoundary::Vacuum
        || case.bottom_boundary == BottomBoundary::Rigid
    {
        Complex64::new(0.0, 0.0)
    } else if case.bottom_boundary.is_tabulated()
        || matches!(
            case.bottom_boundary,
            BottomBoundary::ElasticHalfSpace { .. }
        )
    {
        (bottom.admittance(x2) - bottom.admittance(x1)) / (x2 - x1)
    } else {
        // Retain the established half-space rounding order.
        (pekeris_root(x2 - bottom.k2) - pekeris_root(x1 - bottom.k2))
            / (case.bottom_density_g_cm3 * (x2 - x1))
    };
    let norm = sq_norm - top_derivative * phi[0].powi(2) + derivative * phi[n - 1].powi(2);
    let turning = layers
        .iter()
        .flat_map(|layer| (1..=layer.intervals).map(move |i| (layer, i)))
        .find(|(layer, i)| (b[layer.coefficient_start + i] - layer.h * layer.h * x).re + 2.0 > 0.0)
        .map_or(n - 2, |(layer, i)| layer.node_start + i);
    let mut scale = Complex64::new(1.0, 0.0) / norm.sqrt();
    if (scale * phi[turning]).re < 0.0 {
        scale = -scale;
    }
    for value in &mut phi {
        *value *= scale;
    }
    let group_speed = (Complex64::new(1.0, 0.0) / (scale * scale * slow * omega / x.sqrt())).re;
    let grid = crate::solver::layers::grid(layers, case.fluid_top_depth_m());
    let eigenfunction: Vec<Complex64> = case
        .mode_sample_depths_m
        .iter()
        .map(|&depth| {
            let depth = depth as f32;
            let upper = grid.partition_point(|&z| z < depth).clamp(1, n - 1);
            let weight = (depth - grid[upper - 1]) / (grid[upper] - grid[upper - 1]);
            let first = phi[upper - 1];
            let difference = phi[upper] - first;
            Complex64::new(
                f64::from(first.re as f32 + weight * difference.re as f32),
                f64::from(first.im as f32 + weight * difference.im as f32),
            )
        })
        .collect();
    let perturbation =
        super::scattering::perturbation(case, layers, x, omega, |i| b[i], |i| phi[i]);
    let k = (x + perturbation).sqrt();
    if !k.re.is_finite()
        || !k.im.is_finite()
        // Pinned Fortran roots can carry positive imaginary roundoff near 1e-18.
        || k.im > k.re * 1e-12
        || !group_speed.is_finite()
        || !norm.re.is_finite()
        || !norm.im.is_finite()
        || eigenfunction
            .iter()
            .any(|v: &Complex64| !v.re.is_finite() || !v.im.is_finite())
    {
        return Err(error(
            "KR0303",
            "invalid complex mode normalization",
            "modes",
        ));
    }
    Ok((
        NormalMode {
            horizontal_wavenumber_rad_per_m: k,
            phase_speed_mps: omega / k.re,
            group_speed_mps: group_speed,
            attenuation_nepers_per_m: -k.im,
            eigenfunction,
        },
        perturbation,
    ))
}

#[cfg(test)]
#[test]
fn tabulated_shooting_preserves_pinned_real_grouping() {
    let value = tabulated_shoot_step(
        Complex64::new(1.0, 1.0),
        Complex64::new(1.0, 0.3),
        Complex64::new(0.7, 0.0),
    );
    assert_eq!(value.re.to_bits(), 5.551_115_123_125_783e-17_f64.to_bits());
}

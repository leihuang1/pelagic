// Adapted from Acoustics Toolbox v2023.5 Kraken/BCImpedanceMod.f90 and
// BCImpedancecMod.f90, Copyright (C) 2009 Michael B. Porter.
// GPL-3.0-or-later; see LICENSE.
//! Elastic half-space impedance and depth-sampled finite solid-cap transfer.
use crate::{Boundary, CaseDefinition, DiagnosticReport, Interpolation, ModeSolver, error};
use num_complex::Complex64;
use std::f64::consts::PI;

#[cfg(test)]
mod tests;

#[allow(clippy::float_cmp, clippy::too_many_lines)] // Depth bounds and scalar/profile agreement are exact.
pub(crate) fn validate(case: &CaseDefinition, diagnostics: &mut DiagnosticReport) {
    if case.mode_solver == ModeSolver::Kraken
        && has_layers(case)
        && matches!(case.surface_boundary, Boundary::ElasticHalfSpace { .. })
    {
        diagnostics.push(error(
            "surface_boundary",
            "KRAKEN elastic top half-space with finite solids is not validated",
        ));
    }
    for (boundary, cp, field) in [
        (
            &case.surface_boundary,
            case.surface_sound_speed_mps,
            "surface_boundary",
        ),
        (
            &case.bottom_boundary,
            case.bottom_sound_speed_mps,
            "bottom_boundary",
        ),
    ] {
        if let Boundary::ElasticHalfSpace {
            shear_sound_speed_mps: cs,
            shear_attenuation_db_per_wavelength: loss,
        } = boundary
        {
            // Positive bulk modulus: cp² > 4/3 cs², not merely cp > cs.
            if !cs.is_finite() || *cs <= 0.0 || cp <= (4.0_f64 / 3.0).sqrt() * cs {
                diagnostics.push(error(
                    field,
                    "elastic half-space requires positive shear speed and cp² > 4/3 cs²",
                ));
            }
            if !(0.0..=8.685_889_6 * 2.0 * PI).contains(loss) {
                diagnostics.push(error(
                    field,
                    "shear loss requires finite 0 <= Im(cs) <= Re(cs)",
                ));
            }
            if case.interpolation == Interpolation::AnalyticMunk
                || case.surface_boundary.is_tabulated()
                || case.bottom_boundary.is_tabulated()
            {
                diagnostics.push(error(
                    field,
                    "elastic half-spaces require N/C/P/S and smooth V/R/A boundaries",
                ));
            }
        }
    }
    if has_layers(case) {
        if case.interpolation == Interpolation::AnalyticMunk
            || case.surface_boundary.is_tabulated()
            || case.bottom_boundary.is_tabulated()
        {
            diagnostics.push(error(
                "top_elastic_layers",
                "finite elastic layers require N/C/P/S and smooth V/R/A boundaries",
            ));
        }
        for (name, layers, start, boundary) in [
            (
                "top_elastic_layers",
                &case.top_elastic_layers,
                0.0,
                &case.surface_boundary,
            ),
            (
                "bottom_elastic_layers",
                &case.bottom_elastic_layers,
                case.fluid_bottom_depth_m(),
                &case.bottom_boundary,
            ),
        ] {
            if !layers.is_empty() && *boundary == Boundary::FluidHalfSpace {
                diagnostics.push(error(name, "finite elastic layers require vacuum, rigid or elastic outer boundary; pinned acoustic half-space compound state is undefined"));
            }
            let mut top = start;
            for (index, layer) in layers.iter().enumerate() {
                let field = format!("{name}[{index}]");
                if !layer.bottom_depth_m.is_finite() || layer.bottom_depth_m <= top {
                    diagnostics.push(error(
                        format!("{field}.bottom_depth_m"),
                        "elastic interfaces must be finite and strictly increasing",
                    ));
                }
                let cp = layer.compressional_sound_speed_mps;
                let cs = layer.shear_sound_speed_mps;
                if !cp.is_finite()
                    || !cs.is_finite()
                    || cs <= 0.0
                    || cp <= (4.0_f64 / 3.0).sqrt() * cs
                    || !layer.density_g_cm3.is_finite()
                    || layer.density_g_cm3 <= 0.0
                {
                    diagnostics.push(error(
                        format!("{field}.material"),
                        "require finite positive density and shear speed, cp² > 4/3 cs²",
                    ));
                }
                for loss in [
                    layer.compressional_attenuation_db_per_wavelength,
                    layer.shear_attenuation_db_per_wavelength,
                ] {
                    if !(0.0..=8.685_889_6 * 2.0 * PI).contains(&loss) {
                        diagnostics.push(error(
                            format!("{field}.attenuation"),
                            "elastic loss requires finite 0 <= Im(c) <= Re(c)",
                        ));
                    }
                }
                if layer.mesh_points != 0
                    && !(10..=crate::MAX_MESH_POINTS).contains(&layer.mesh_points)
                {
                    diagnostics.push(error(
                        format!("{field}.mesh_points"),
                        "mesh points must be 0 or in 10..=1000000",
                    ));
                }
                if !layer.material_profile.is_empty() {
                    let points = &layer.material_profile;
                    let first = points[0];
                    let invalid = !(2..=crate::MAX_VECTOR_LENGTH).contains(&points.len())
                        || first.depth_m != top
                        || points.last().unwrap().depth_m != layer.bottom_depth_m
                        || first.compressional_sound_speed_mps != cp
                        || first.shear_sound_speed_mps != cs
                        || first.density_g_cm3 != layer.density_g_cm3
                        || first.compressional_attenuation_db_per_wavelength
                            != layer.compressional_attenuation_db_per_wavelength
                        || first.shear_attenuation_db_per_wavelength
                            != layer.shear_attenuation_db_per_wavelength
                        || points.windows(2).any(|p| p[0].depth_m >= p[1].depth_m)
                        || points.iter().any(|p| {
                            !p.depth_m.is_finite()
                                || !p.compressional_sound_speed_mps.is_finite()
                                || !p.shear_sound_speed_mps.is_finite()
                                || p.shear_sound_speed_mps <= 0.0
                                || p.compressional_sound_speed_mps
                                    <= (4.0_f64 / 3.0).sqrt() * p.shear_sound_speed_mps
                                || !p.density_g_cm3.is_finite()
                                || p.density_g_cm3 <= 0.0
                                || [
                                    p.compressional_attenuation_db_per_wavelength,
                                    p.shear_attenuation_db_per_wavelength,
                                ]
                                .iter()
                                .any(|a| !(0.0..=8.685_889_6 * 2.0 * PI).contains(a))
                        });
                    if invalid {
                        diagnostics.push(error(format!("{field}.material_profile"), "require finite increasing top-to-bottom elastic samples, first sample matching scalar material, positive bulk modulus/density and nonnegative bounded losses"));
                    } else if let Err(report) = sampled_material(case, layer, top, 16) {
                        diagnostics.push(error(
                            format!("{field}.material_profile"),
                            &report.diagnostics()[0].message,
                        ));
                    }
                }
                top = layer.bottom_depth_m;
            }
        }
        if case.fluid_top_depth_m() >= case.water_depth_m {
            diagnostics.push(error(
                "water_depth_m",
                "first fluid bottom must lie below the elastic cap",
            ));
        }
    }
}

pub(crate) fn has_layers(case: &CaseDefinition) -> bool {
    !case.top_elastic_layers.is_empty() || !case.bottom_elastic_layers.is_empty()
}

pub(crate) fn has_half_space(case: &CaseDefinition) -> bool {
    [&case.surface_boundary, &case.bottom_boundary]
        .iter()
        .any(|boundary| matches!(boundary, Boundary::ElasticHalfSpace { .. }))
}

pub(crate) fn minimum_speed(
    case: &CaseDefinition,
    water: f64,
    top: &[SolidMesh],
    bottom: &[SolidMesh],
) -> f64 {
    let mut minimum = water;
    for (boundary, cp) in [
        (&case.surface_boundary, case.surface_sound_speed_mps),
        (&case.bottom_boundary, case.bottom_sound_speed_mps),
    ] {
        match boundary {
            Boundary::ElasticHalfSpace {
                shear_sound_speed_mps,
                ..
            } => {
                minimum = minimum.min(*shear_sound_speed_mps);
            }
            Boundary::FluidHalfSpace => minimum = minimum.min(cp),
            _ => {}
        }
    }
    for mesh in top.iter().chain(bottom) {
        minimum = minimum.min(mesh.minimum_speed);
    }
    // Initialize reduces cMin for Scholte waves; 0.85 is a default-kind literal.
    f64::from(0.85_f32) * minimum
}

pub(crate) fn maximum_speed(case: &CaseDefinition) -> f64 {
    [&case.surface_boundary, &case.bottom_boundary].iter().fold(
        case.c_high_mps,
        |maximum, boundary| match boundary {
            Boundary::ElasticHalfSpace {
                shear_sound_speed_mps,
                ..
            } => maximum.min(*shear_sound_speed_mps),
            _ => maximum,
        },
    )
}

pub(crate) fn complex_speed(speed: f64, loss: f64) -> Complex64 {
    Complex64::new(speed, loss * speed / (8.685_889_6 * 2.0 * PI))
}

pub(crate) fn half_space(
    boundary: &Boundary,
    x: Complex64,
    omega: f64,
    cp: f64,
    density: f64,
    loss: f64,
    real: bool,
) -> (Complex64, Complex64) {
    let Boundary::ElasticHalfSpace {
        shear_sound_speed_mps: cs,
        shear_attenuation_db_per_wavelength: shear_loss,
    } = boundary
    else {
        unreachable!()
    };
    // BCImpedanceMod ignores half-space elastic attenuation even with ComplexFlag.
    if real {
        let y = real_half_space(x.re, omega, cp, *cs, density);
        return (Complex64::from(omega * omega * y[3]), Complex64::from(y[1]));
    }
    let cp = complex_speed(cp, loss);
    let cs = complex_speed(*cs, *shear_loss);
    let shear = x - omega * omega / cs.powi(2);
    let pressure = x - omega * omega / cp.powi(2);
    let gs = crate::solver::complex_modes::pekeris_root(shear);
    let gp = crate::solver::complex_modes::pekeris_root(pressure);
    let mu = density * cs.powi(2);
    let f = omega * omega * gp * (x - shear);
    let g = ((shear + x).powi(2) - 4.0 * gs * gp * x) * mu;
    (f, g)
}

// BCImpedanceMod's real branch, including pinned GNU grouping: x - (x - q)
// simplifies to q, rather than introducing another subtraction near a pole.
fn real_half_space(x: f64, omega: f64, cp: f64, cs: f64, density: f64) -> [f64; 5] {
    let shear_k2 = omega * omega / (cs * cs);
    let s = x - shear_k2;
    let p = x - omega * omega / (cp * cp);
    let gs = if s < 0.0 { 0.0 } else { s.sqrt() };
    let gp = if p < 0.0 { 0.0 } else { p.sqrt() };
    let product = gs * gp;
    let mu = density * (cs * cs);
    [
        (product - x) / mu,
        mu * ((x + s).powi(2) - (4.0 * x) * product),
        (shear_k2 - 2.0 * x) + 2.0 * product,
        gp * shear_k2,
        gs * -shear_k2,
    ]
}

/// Depth-sampled compound-matrix mesh, outside the acoustic pressure unknowns.
pub(crate) struct SolidMesh {
    intervals: usize,
    h: f64,
    coefficients: Vec<([Complex64; 4], f64)>,
    minimum_speed: f64,
    real: bool,
}

#[allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]
fn intervals(
    case: &crate::Case,
    layer: &crate::ElasticLayer,
    top: f64,
    multiplier: usize,
) -> Result<usize, DiagnosticReport> {
    let reference = case
        .mesh_reference_frequency_hz
        .unwrap_or(case.frequency_hz);
    let last_speed = layer
        .material_profile
        .last()
        .map_or(layer.shear_sound_speed_mps, |p| p.shear_sound_speed_mps);
    let needed = ((layer.bottom_depth_m - top) / (last_speed / reference / 20.0))
        .floor()
        .max(10.0);
    let base = if layer.mesh_points == 0 {
        needed
    } else {
        layer.mesh_points as f64
    };
    let scaled = if case.mesh_reference_frequency_hz.is_some() {
        (base * multiplier as f64 * case.frequency_hz / reference).floor()
    } else {
        base * multiplier as f64
    };
    if !needed.is_finite()
        || base < (needed as usize / 2) as f64
        || !scaled.is_finite()
        || !(10.0..=crate::MAX_MESH_POINTS as f64).contains(&scaled)
    {
        return Err(crate::solver::error(
            "KR0302",
            "elastic mesh is too coarse or exceeds the mesh limit",
            "elastic_layers.mesh_points",
        ));
    }
    Ok(scaled as usize)
}

pub(crate) fn mesh_points(
    case: &crate::Case,
    multiplier: usize,
) -> Result<usize, DiagnosticReport> {
    let mut total = 0;
    for (layers, mut top) in [
        (&case.top_elastic_layers, 0.0),
        (&case.bottom_elastic_layers, case.fluid_bottom_depth_m()),
    ] {
        for layer in layers {
            total += intervals(case, layer, top, multiplier)?;
            top = layer.bottom_depth_m;
            if total > crate::MAX_MESH_POINTS {
                return Err(crate::solver::error(
                    "KR0302",
                    "total elastic mesh exceeds the mesh limit",
                    "mesh_points",
                ));
            }
        }
    }
    Ok(total)
}

#[allow(clippy::cast_precision_loss, clippy::too_many_lines)]
fn sampled_material(
    case: &CaseDefinition,
    layer: &crate::ElasticLayer,
    top: f64,
    n: usize,
) -> Result<Vec<(Complex64, Complex64, f64)>, DiagnosticReport> {
    let uniform = layer.material_profile.is_empty();
    let homogeneous = [top, layer.bottom_depth_m].map(|depth_m| crate::ElasticMaterialPoint {
        depth_m,
        compressional_sound_speed_mps: layer.compressional_sound_speed_mps,
        shear_sound_speed_mps: layer.shear_sound_speed_mps,
        density_g_cm3: layer.density_g_cm3,
        compressional_attenuation_db_per_wavelength: layer
            .compressional_attenuation_db_per_wavelength,
        shear_attenuation_db_per_wavelength: layer.shear_attenuation_db_per_wavelength,
    });
    let points = if uniform {
        &homogeneous[..]
    } else {
        &layer.material_profile
    };
    let pressure: Vec<_> = points
        .iter()
        .map(|p| crate::SoundSpeedPoint {
            depth_m: p.depth_m,
            sound_speed_mps: p.compressional_sound_speed_mps,
        })
        .collect();
    let shear: Vec<_> = points
        .iter()
        .map(|p| crate::SoundSpeedPoint {
            depth_m: p.depth_m,
            sound_speed_mps: p.shear_sound_speed_mps,
        })
        .collect();
    let pressure_loss: Vec<_> = points
        .iter()
        .map(|p| p.compressional_attenuation_db_per_wavelength)
        .collect();
    let shear_loss: Vec<_> = points
        .iter()
        .map(|p| p.shear_attenuation_db_per_wavelength)
        .collect();
    let build_profile = if uniform {
        crate::solver::profile::Profile::new_layer
    } else {
        crate::solver::profile::Profile::new_elastic_layer
    };
    let cp = build_profile(
        case,
        crate::solver::layers::Layer {
            top,
            bottom: layer.bottom_depth_m,
            density: layer.density_g_cm3,
            points: &pressure,
            loss: &pressure_loss,
            mesh_points: n,
        },
    )?;
    let cs = build_profile(
        case,
        crate::solver::layers::Layer {
            top,
            bottom: layer.bottom_depth_m,
            density: layer.density_g_cm3,
            points: &shear,
            loss: &shear_loss,
            mesh_points: n,
        },
    )?;
    let bulk_margin = cp.minimum_scaled_difference(&cs, (4.0_f64 / 3.0).sqrt());
    if !bulk_margin.is_finite() || bulk_margin <= 0.0 {
        return Err(crate::solver::error(
            "KR0302",
            "interpolated elastic material requires positive bulk modulus",
            "material_profile",
        ));
    }
    let h: Vec<_> = points
        .windows(2)
        .map(|p| p[1].depth_m - p[0].depth_m)
        .collect();
    let density: Vec<_> = points.iter().map(|p| p.density_g_cm3).collect();
    let cubic = if matches!(
        case.interpolation,
        Interpolation::Pchip | Interpolation::Spline
    ) {
        crate::solver::profile::cubic_coefficients(case.interpolation, &h, &density)?
    } else {
        Vec::new()
    };
    if cubic.iter().zip(&h).any(|(&c, &h)| {
        let minimum = crate::solver::profile::segment_minimum(c, h);
        !minimum.is_finite() || minimum <= 0.0
    }) {
        return Err(crate::solver::error(
            "KR0302",
            "interpolated elastic density must stay finite and positive",
            "material_profile",
        ));
    }
    let spacing = (layer.bottom_depth_m - top) / n as f64;
    Ok((0..=n)
        .map(|i| {
            let depth = if i == n {
                layer.bottom_depth_m
            } else {
                top + i as f64 * spacing
            };
            let upper = points
                .partition_point(|p| p.depth_m < depth)
                .clamp(1, points.len() - 1);
            let t = depth - points[upper - 1].depth_m;
            let rho = if cubic.is_empty() {
                let weight = t / h[upper - 1];
                (1.0 - weight) * density[upper - 1] + weight * density[upper]
            } else {
                let [c0, c1, c2, c3] = cubic[upper - 1];
                c0 + t * (c1 + t * (c2 + t * c3))
            };
            if uniform {
                (
                    cp.mesh_complex_speed(i, n),
                    cs.mesh_complex_speed(i, n),
                    rho,
                )
            } else {
                (
                    cp.mesh_elastic_speed(i, n),
                    cs.mesh_elastic_speed(i, n),
                    rho,
                )
            }
        })
        .collect())
}

// Keep rare scaling out of the loop: inlining it spills the real state on x86-64.
#[cold]
#[inline(never)]
fn rescale_real(mut y: [f64; 5], mut z: [f64; 5], mut power: i32) -> ([f64; 5], [f64; 5], i32) {
    let scale = if z[1].abs() < 1e-50 {
        power -= 50;
        1e50
    } else {
        power += 50;
        1e-50
    };
    for i in 0..5 {
        y[i] *= scale;
        z[i] *= scale;
    }
    (y, z, power)
}

impl SolidMesh {
    #[allow(clippy::cast_precision_loss, clippy::too_many_lines)] // Keep pinned coefficient grouping together.
    pub(crate) fn build(
        case: &crate::Case,
        multiplier: usize,
        top_side: bool,
        real: bool,
    ) -> Result<Vec<Self>, DiagnosticReport> {
        let layers = if top_side {
            &case.top_elastic_layers
        } else {
            &case.bottom_elastic_layers
        };
        let mut top = if top_side {
            0.0
        } else {
            case.fluid_bottom_depth_m()
        };
        let mut meshes = Vec::new();
        let omega = 2.0 * PI * case.frequency_hz;
        for layer in layers {
            let n = intervals(case, layer, top, multiplier)?;
            let h = (layer.bottom_depth_m - top) / n as f64;
            let cp = complex_speed(
                layer.compressional_sound_speed_mps,
                layer.compressional_attenuation_db_per_wavelength,
            )
            .powi(2);
            let cs = complex_speed(
                layer.shear_sound_speed_mps,
                layer.shear_attenuation_db_per_wavelength,
            )
            .powi(2);
            let two_h = 2.0 * h;
            let rho = layer.density_g_cm3;
            let samples = if real || !layer.material_profile.is_empty() {
                sampled_material(case, layer, top, n)?
            } else {
                Vec::new()
            };
            let minimum_speed = if layer.material_profile.is_empty() {
                layer.shear_sound_speed_mps
            } else {
                samples
                    .iter()
                    .map(|(_, cs, _)| cs.re)
                    .fold(f64::INFINITY, f64::min)
            };
            let coefficients = if real {
                // Initialize uses Re(c²), not the half-space's Re(c)².
                samples
                    .iter()
                    .map(|&(cp, cs, density)| {
                        let pressure_squared = cp.powi(2).re;
                        let cs_real2 = cs.re * cs.re;
                        let cs_imag2 = cs.im * cs.im;
                        let shear_squared = cs_real2 - cs_imag2;
                        // Preserve Initialize's pinned reassociation of Re(cP²-cS²).
                        let difference = pressure_squared - cs_real2 + cs_imag2;
                        let b = [
                            two_h / (density * shear_squared),
                            two_h / (density * pressure_squared),
                            (shear_squared * difference) * (4.0 * density) * two_h
                                / pressure_squared,
                            two_h * (pressure_squared - 2.0 * shear_squared) / pressure_squared,
                        ]
                        .map(Complex64::from);
                        (b, (omega * omega * density) * two_h)
                    })
                    .collect::<Vec<_>>()
            } else if !layer.material_profile.is_empty() {
                samples
                    .iter()
                    .map(|&(cp, cs, rho)| {
                        let cp = cp.powi(2);
                        let cs = cs.powi(2);
                        (
                            [
                                two_h / (rho * cs),
                                two_h / (rho * cp),
                                4.0 * two_h * rho * cs * (cp - cs) / cp,
                                two_h * (cp - 2.0 * cs) / cp,
                            ],
                            two_h * omega * omega * rho,
                        )
                    })
                    .collect()
            } else {
                vec![(
                    [
                        two_h / (rho * cs),
                        two_h / (rho * cp),
                        4.0 * two_h * rho * cs * (cp - cs) / cp,
                        two_h * (cp - 2.0 * cs) / cp,
                    ],
                    two_h * omega * omega * rho,
                )]
            };
            if coefficients.iter().any(|(b, rho)| {
                !rho.is_finite() || b.iter().any(|v| !v.re.is_finite() || !v.im.is_finite())
            }) {
                return Err(crate::solver::error(
                    "KR0302",
                    "non-finite elastic mesh coefficients",
                    "elastic_layers",
                ));
            }
            meshes.push(Self {
                intervals: n,
                h,
                coefficients,
                minimum_speed,
                real,
            });
            top = layer.bottom_depth_m;
        }
        Ok(meshes)
    }

    pub(crate) fn points(&self) -> usize {
        self.intervals + 1
    }

    pub(crate) fn spacing(&self) -> f64 {
        self.h
    }

    #[inline]
    fn real_step(&self, x: f64, y: [f64; 5], node: usize, euler: bool) -> [f64; 5] {
        let (b, rho) = self.coefficients[node.min(self.coefficients.len() - 1)];
        let [b1, b2, b3, b4] = b.map(|b| b.re);
        let two_x = 2.0 * x;
        let four_h_x = self.h * (4.0 * x);
        let xb3 = x * b3 - rho;
        // GNU's real ElasticUP/DN group Euler and midpoint steps differently.
        let fourth = if euler {
            xb3 * y[0] + (b2 * y[1] - b4 * (two_x * y[2]))
        } else {
            b2 * y[1] + (xb3 * y[0] - two_x * (b4 * y[2]))
        };
        let fifth = if euler {
            (rho * y[0] - four_h_x * y[2]) - b1 * y[1]
        } else {
            (rho * y[0] - b1 * y[1]) - four_h_x * y[2]
        };
        [
            b1 * y[3] - b2 * y[4],
            -(rho * y[3] + xb3 * y[4]),
            2.0 * self.h * y[3] + b4 * y[4],
            fourth,
            fifth,
        ]
    }

    #[allow(clippy::manual_range_contains)] // NaN must not trigger rescaling.
    fn real_transfer(&self, x: f64, mut y: [f64; 5], top: bool, power: &mut i32) -> [f64; 5] {
        let sign = if top { 1.0 } else { -1.0 };
        let delta = self.real_step(x, y, if top { 0 } else { self.intervals }, true);
        let mut z = std::array::from_fn(|i| y[i] + sign * 0.5 * delta[i]);
        let mut previous = y;
        for step in 0..self.intervals {
            previous = y;
            y = z;
            let node = if top {
                step + 1
            } else {
                self.intervals - step - 1
            };
            let delta = self.real_step(x, y, node, false);
            z = std::array::from_fn(|i| previous[i] + sign * delta[i]);
            let magnitude = z[1].abs();
            if step + 1 != self.intervals && (magnitude < 1e-50 || magnitude > 1e50) {
                (y, z, *power) = rescale_real(y, z, *power);
            }
        }
        std::array::from_fn(|i| ((previous[i] + z[i]) + 2.0 * y[i]) * 0.25)
    }

    fn step(&self, x: Complex64, y: [Complex64; 5], node: usize) -> [Complex64; 5] {
        let (b, rho) = self.coefficients[node.min(self.coefficients.len() - 1)];
        let [b1, b2, b3, b4] = b;
        let xb3 = x * b3 - rho;
        [
            b1 * y[3] - b2 * y[4],
            -rho * y[3] - xb3 * y[4],
            2.0 * self.h * y[3] + b4 * y[4],
            xb3 * y[0] + b2 * y[1] - 2.0 * x * b4 * y[2],
            rho * y[0] - b1 * y[1] - 4.0 * self.h * x * y[2],
        ]
    }

    fn transfer(
        &self,
        x: Complex64,
        mut y: [Complex64; 5],
        top: bool,
        power: &mut i32,
    ) -> [Complex64; 5] {
        if self.real {
            // Real cap seeds have zero imaginary parts; retain even their zero signs.
            debug_assert!(y.iter().all(|v| v.im == 0.0));
            let result = self.real_transfer(x.re, y.map(|v| v.re), top, power);
            let zero = if top { 0.0 } else { -0.0 };
            return std::array::from_fn(|i| Complex64::new(result[i], y[i].im + zero));
        }
        let sign = if top { 1.0 } else { -1.0 };
        let delta = self.step(x, y, if top { 0 } else { self.intervals });
        let mut z = std::array::from_fn(|i| y[i] + sign * 0.5 * delta[i]);
        let mut previous = y;
        for step in 0..self.intervals {
            previous = y;
            y = z;
            let node = if top {
                step + 1
            } else {
                self.intervals - step - 1
            };
            let delta = self.step(x, y, node);
            z = std::array::from_fn(|i| previous[i] + sign * delta[i]);
            if step + 1 != self.intervals {
                let scale = if z[1].re.abs() < 1e-50 {
                    *power -= 50;
                    1e50
                } else if z[1].re.abs() > 1e50 {
                    *power += 50;
                    1e-50
                } else {
                    1.0
                };
                for i in 0..5 {
                    y[i] *= scale;
                    z[i] *= scale;
                }
            }
        }
        std::array::from_fn(|i| (previous[i] + 2.0 * y[i] + z[i]) / 4.0)
    }
}

/// Returns the effective Robin data after propagating through a finite solid cap.
#[allow(clippy::too_many_arguments)]
pub(crate) fn cap_impedance(
    boundary: &Boundary,
    x: Complex64,
    omega: f64,
    cp: f64,
    density: f64,
    loss: f64,
    meshes: &[SolidMesh],
    top: bool,
    real: bool,
) -> (Complex64, Complex64, i32) {
    let mut y = match boundary {
        Boundary::Vacuum => [1.0.into(), 0.0.into(), 0.0.into(), 0.0.into(), 0.0.into()],
        Boundary::Rigid => [0.0.into(), 1.0.into(), 0.0.into(), 0.0.into(), 0.0.into()],
        Boundary::ElasticHalfSpace {
            shear_sound_speed_mps: cs,
            shear_attenuation_db_per_wavelength: shear_loss,
        } => {
            if real {
                real_half_space(x.re, omega, cp, *cs, density).map(Complex64::from)
            } else {
                let cp = complex_speed(cp, loss);
                let cs = complex_speed(*cs, *shear_loss);
                let s = x - omega * omega / cs.powi(2);
                let p = x - omega * omega / cp.powi(2);
                let gs = crate::solver::complex_modes::pekeris_root(s);
                let gp = crate::solver::complex_modes::pekeris_root(p);
                let mu = density * cs.powi(2);
                [
                    (gs * gp - x) / mu,
                    ((s + x).powi(2) - 4.0 * gs * gp * x) * mu,
                    2.0 * gs * gp - s - x,
                    gp * (x - s),
                    gs * (s - x),
                ]
            }
        }
        _ => unreachable!("cap outer boundary was validated"),
    };
    let mut power = 0;
    if top {
        for mesh in meshes {
            y = mesh.transfer(x, y, true, &mut power);
        }
    } else {
        for mesh in meshes.iter().rev() {
            y = mesh.transfer(x, y, false, &mut power);
        }
    }
    (omega * omega * y[3], y[1], power)
}

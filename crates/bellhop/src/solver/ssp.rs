use std::f64::consts::PI;

use num_complex::Complex64;

use crate::Case;
use crate::diagnostic::{Diagnostic, SourceLocation};
use crate::model::{AttenuationUnit, SoundSpeedPoint, SspInterpolation, VolumeAttenuation};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct SegmentState {
    pub depth: usize,
    pub range: usize,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct SoundSpeedSample {
    pub speed_mps: f64,
    pub imaginary_speed_mps: f64,
    pub gradient: [f64; 2],
    pub c_rr: f64,
    pub c_rz: f64,
    pub c_zz: f64,
    pub density_g_cm3: f64,
}

#[derive(Clone, Debug)]
pub(crate) struct SoundSpeedModel {
    kind: SspInterpolation,
    depths_m: Vec<f64>,
    speeds: Vec<Complex64>,
    densities_g_cm3: Vec<f64>,
    linear_slopes: Vec<Complex64>,
    pchip_coefficients: Vec<[Complex64; 4]>,
    spline_coefficients: Vec<[Complex64; 4]>,
    range_field: Option<RangeField>,
}

#[derive(Clone, Debug)]
struct RangeField {
    ranges_m: Vec<f64>,
    speeds_mps: Vec<Vec<f64>>,
    depth_slopes: Vec<Vec<f64>>,
}

impl SoundSpeedModel {
    pub fn new(case: &Case) -> Result<Self, Diagnostic> {
        let environment = &case.environment;
        let kind = environment.top_options.interpolation;
        if kind == SspInterpolation::AnalyticMunk {
            return Ok(Self {
                kind,
                depths_m: vec![
                    environment.sound_speed.top_depth_m,
                    environment.sound_speed.bottom_depth_m,
                ],
                speeds: Vec::new(),
                densities_g_cm3: Vec::new(),
                linear_slopes: Vec::new(),
                pchip_coefficients: Vec::new(),
                spline_coefficients: Vec::new(),
                range_field: None,
            });
        }

        let points = &environment.sound_speed.points;
        let depths_m: Vec<f64> = points.iter().map(|point| point.depth_m).collect();
        let speeds: Vec<Complex64> = points
            .iter()
            .map(|point| complex_speed(environment, point))
            .collect();
        if let Some(speed) = speeds.iter().find(|speed| speed.im > speed.re) {
            return Err(Diagnostic::error(
                "BH0302",
                format!(
                    "complex sound speed has imaginary part {} greater than real part {}",
                    speed.im, speed.re
                ),
                "sound_speed.attenuation",
                SourceLocation::file(&environment.source_path),
            ));
        }
        let densities_g_cm3 = points.iter().map(|point| point.density_g_cm3).collect();
        let linear_slopes = interval_slopes(&depths_m, &speeds);
        let pchip_coefficients = if kind == SspInterpolation::Pchip {
            pchip(&depths_m, &speeds)
        } else {
            Vec::new()
        };
        let spline_coefficients = if kind == SspInterpolation::CubicSpline {
            cubic_spline(&depths_m, &speeds, None, None)
        } else {
            Vec::new()
        };
        let range_field = if kind == SspInterpolation::Quadrilateral {
            let field = case.range_dependent_sound_speed.as_ref().ok_or_else(|| {
                Diagnostic::error(
                    "BH0302",
                    "quadrilateral interpolation requires a loaded .ssp field",
                    "range_dependent_sound_speed",
                    SourceLocation::file(&environment.source_path),
                )
            })?;
            let mut depth_slopes = Vec::with_capacity(field.depths_m.len() - 1);
            for depth_index in 0..field.depths_m.len() - 1 {
                let depth_delta = field.depths_m[depth_index + 1] - field.depths_m[depth_index];
                depth_slopes.push(
                    field.speeds_mps[depth_index]
                        .iter()
                        .zip(&field.speeds_mps[depth_index + 1])
                        .map(|(top, bottom)| (bottom - top) / depth_delta)
                        .collect(),
                );
            }
            Some(RangeField {
                ranges_m: field.ranges_m.clone(),
                speeds_mps: field.speeds_mps.clone(),
                depth_slopes,
            })
        } else {
            None
        };

        Ok(Self {
            kind,
            depths_m,
            speeds,
            densities_g_cm3,
            linear_slopes,
            pchip_coefficients,
            spline_coefficients,
            range_field,
        })
    }

    // Measured hot path: called three times in every ray step, which uses only some fields.
    #[inline(always)]
    #[allow(clippy::inline_always)]
    pub fn evaluate(
        &self,
        position_m: [f64; 2],
        segments: &mut SegmentState,
    ) -> Result<SoundSpeedSample, &'static str> {
        if self.kind == SspInterpolation::AnalyticMunk {
            segments.depth = 0;
            return Ok(analytic_munk(position_m[1]));
        }

        update_depth_segment(position_m[1], &self.depths_m, &mut segments.depth);
        let depth_segment = segments.depth;
        let depth_delta = self.depths_m[depth_segment + 1] - self.depths_m[depth_segment];
        let depth_fraction = (position_m[1] - self.depths_m[depth_segment]) / depth_delta;
        let density_g_cm3 = (1.0 - depth_fraction) * self.densities_g_cm3[depth_segment]
            + depth_fraction * self.densities_g_cm3[depth_segment + 1];

        match self.kind {
            SspInterpolation::N2Linear => {
                let n2_left = Complex64::new(1.0, 0.0)
                    / (self.speeds[depth_segment] * self.speeds[depth_segment]);
                let n2_right = Complex64::new(1.0, 0.0)
                    / (self.speeds[depth_segment + 1] * self.speeds[depth_segment + 1]);
                let n2_slope = (n2_right - n2_left) / depth_delta;
                let speed = Complex64::new(1.0, 0.0)
                    / ((1.0 - depth_fraction) * n2_left + depth_fraction * n2_right).sqrt();
                let gradient_z = -0.5 * speed.re.powi(3) * n2_slope.re;
                Ok(SoundSpeedSample {
                    speed_mps: speed.re,
                    imaginary_speed_mps: speed.im,
                    gradient: [0.0, gradient_z],
                    c_rr: 0.0,
                    c_rz: 0.0,
                    c_zz: 3.0 * gradient_z * gradient_z / speed.re,
                    density_g_cm3,
                })
            }
            SspInterpolation::CLinear => {
                let speed = self.speeds[depth_segment]
                    + (position_m[1] - self.depths_m[depth_segment])
                        * self.linear_slopes[depth_segment];
                Ok(SoundSpeedSample {
                    speed_mps: speed.re,
                    imaginary_speed_mps: speed.im,
                    gradient: [0.0, self.linear_slopes[depth_segment].re],
                    c_rr: 0.0,
                    c_rz: 0.0,
                    c_zz: 0.0,
                    density_g_cm3,
                })
            }
            SspInterpolation::Pchip => {
                let x = position_m[1] - self.depths_m[depth_segment];
                let coefficient = self.pchip_coefficients[depth_segment];
                let speed = coefficient[0]
                    + (coefficient[1] + (coefficient[2] + coefficient[3] * x) * x) * x;
                let gradient_z =
                    (coefficient[1] + (2.0 * coefficient[2] + 3.0 * coefficient[3] * x) * x).re;
                Ok(SoundSpeedSample {
                    speed_mps: speed.re,
                    imaginary_speed_mps: speed.im,
                    gradient: [0.0, gradient_z],
                    c_rr: 0.0,
                    c_rz: 0.0,
                    c_zz: (2.0 * coefficient[2] + 6.0 * coefficient[3] * x).re,
                    density_g_cm3,
                })
            }
            SspInterpolation::CubicSpline => {
                let x = position_m[1] - self.depths_m[depth_segment];
                let coefficient = self.spline_coefficients[depth_segment];
                let speed = coefficient[0]
                    + x * (coefficient[1] + x * (0.5 * coefficient[2] + x * coefficient[3] / 6.0));
                let gradient_z =
                    (coefficient[1] + x * (coefficient[2] + 0.5 * x * coefficient[3])).re;
                Ok(SoundSpeedSample {
                    speed_mps: speed.re,
                    imaginary_speed_mps: speed.im,
                    gradient: [0.0, gradient_z],
                    c_rr: 0.0,
                    c_rz: 0.0,
                    c_zz: (coefficient[2] + x * coefficient[3]).re,
                    density_g_cm3,
                })
            }
            SspInterpolation::Quadrilateral => self.evaluate_quadrilateral(
                position_m,
                segments,
                depth_segment,
                depth_fraction,
                density_g_cm3,
            ),
            SspInterpolation::AnalyticMunk => unreachable!("handled above"),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn evaluate_quadrilateral(
        &self,
        position_m: [f64; 2],
        segments: &mut SegmentState,
        depth_segment: usize,
        depth_fraction: f64,
        density_g_cm3: f64,
    ) -> Result<SoundSpeedSample, &'static str> {
        let field = self.range_field.as_ref().expect("constructed for Q SSP");
        if position_m[0] < field.ranges_m[0]
            || position_m[0] > field.ranges_m[field.ranges_m.len() - 1]
        {
            return Err("ray is outside the range where sound speed is defined");
        }
        update_range_segment(position_m[0], &field.ranges_m, &mut segments.range);
        let range_segment = segments.range;
        let depth_offset = position_m[1] - self.depths_m[depth_segment];
        let vertical_gradient_left = field.depth_slopes[depth_segment][range_segment];
        let vertical_gradient_right = field.depth_slopes[depth_segment][range_segment + 1];
        let speed_left =
            field.speeds_mps[depth_segment][range_segment] + depth_offset * vertical_gradient_left;
        let speed_right = field.speeds_mps[depth_segment][range_segment + 1]
            + depth_offset * vertical_gradient_right;
        let range_delta = field.ranges_m[range_segment + 1] - field.ranges_m[range_segment];
        let range_fraction =
            ((position_m[0] - field.ranges_m[range_segment]) / range_delta).clamp(0.0, 1.0);
        let speed_mps = (1.0 - range_fraction) * speed_left + range_fraction * speed_right;
        let gradient_z = (1.0 - range_fraction) * vertical_gradient_left
            + range_fraction * vertical_gradient_right;
        let gradient_r = (speed_right - speed_left) / range_delta;
        let attenuation_speed = (1.0 - depth_fraction) * self.speeds[depth_segment]
            + depth_fraction * self.speeds[depth_segment + 1];
        Ok(SoundSpeedSample {
            speed_mps,
            imaginary_speed_mps: attenuation_speed.im,
            gradient: [gradient_r, gradient_z],
            c_rr: 0.0,
            c_rz: (vertical_gradient_right - vertical_gradient_left) / range_delta,
            c_zz: 0.0,
            density_g_cm3,
        })
    }

    pub fn depth_interface(&self, index: usize) -> f64 {
        self.depths_m[index]
    }

    pub fn range_interface(&self, index: usize) -> Option<f64> {
        self.range_field.as_ref().map(|field| field.ranges_m[index])
    }

    pub fn is_range_dependent(&self) -> bool {
        self.range_field.is_some()
    }
}

fn update_depth_segment(depth_m: f64, depth_grid: &[f64], segment: &mut usize) {
    if depth_m < depth_grid[*segment] || depth_m > depth_grid[*segment + 1] {
        if let Some(index) = depth_grid[1..].iter().position(|depth| depth_m < *depth) {
            *segment = index;
        } else {
            *segment = depth_grid.len() - 2;
        }
    }
}

fn update_range_segment(range_m: f64, range_grid: &[f64], segment: &mut usize) {
    if range_m < range_grid[*segment] || range_m >= range_grid[*segment + 1] {
        if let Some(index) = range_grid[1..].iter().position(|range| range_m < *range) {
            *segment = index;
        } else {
            *segment = range_grid.len() - 2;
        }
    }
}

fn interval_slopes(x: &[f64], y: &[Complex64]) -> Vec<Complex64> {
    x.windows(2)
        .zip(y.windows(2))
        .map(|(x_pair, y_pair)| (y_pair[1] - y_pair[0]) / (x_pair[1] - x_pair[0]))
        .collect()
}

fn pchip(x: &[f64], y: &[Complex64]) -> Vec<[Complex64; 4]> {
    let point_count = x.len();
    if point_count == 2 {
        return vec![[
            y[0],
            (y[1] - y[0]) / (x[1] - x[0]),
            Complex64::new(0.0, 0.0),
            Complex64::new(0.0, 0.0),
        ]];
    }

    let mut derivatives = vec![Complex64::new(0.0, 0.0); point_count];
    let (h1, h2, delta1, delta2) = adjacent_secants(x, y, 1);
    let candidate = ((2.0 * h1 + h2) * delta1 - h1 * delta2) / (h1 + h2);
    derivatives[0] = endpoint_derivative(delta1, delta2, candidate);

    let (h1, h2, delta1, delta2) = adjacent_secants(x, y, point_count - 2);
    let candidate = (-h2 * delta1 + (h1 + 2.0 * h2) * delta2) / (h1 + h2);
    derivatives[point_count - 1] = endpoint_derivative(delta2, delta1, candidate);

    let spline = cubic_spline(
        x,
        y,
        Some(derivatives[0]),
        Some(derivatives[point_count - 1]),
    );
    for index in 1..point_count - 1 {
        let (_, _, delta1, delta2) = adjacent_secants(x, y, index);
        derivatives[index] = interior_derivative(delta1, delta2, spline[index][1]);
    }

    (0..point_count - 1)
        .map(|index| {
            let h = x[index + 1] - x[index];
            let difference = y[index + 1] - y[index];
            let first = derivatives[index];
            let second = derivatives[index + 1];
            [
                y[index],
                first,
                (3.0 * difference - h * (2.0 * first + second)) / h.powi(2),
                (h * (first + second) - 2.0 * difference) / h.powi(3),
            ]
        })
        .collect()
}

fn adjacent_secants(x: &[f64], y: &[Complex64], center: usize) -> (f64, f64, Complex64, Complex64) {
    let h1 = x[center] - x[center - 1];
    let h2 = x[center + 1] - x[center];
    (
        h1,
        h2,
        (y[center] - y[center - 1]) / h1,
        (y[center + 1] - y[center]) / h2,
    )
}

fn interior_derivative(delta1: Complex64, delta2: Complex64, candidate: Complex64) -> Complex64 {
    Complex64::new(
        project_interior(delta1.re, delta2.re, candidate.re),
        project_interior(delta1.im, delta2.im, candidate.im),
    )
}

fn endpoint_derivative(
    primary_delta: Complex64,
    other_delta: Complex64,
    candidate: Complex64,
) -> Complex64 {
    Complex64::new(
        project_endpoint(primary_delta.re, other_delta.re, candidate.re),
        project_endpoint(primary_delta.im, other_delta.im, candidate.im),
    )
}

fn project_interior(delta1: f64, delta2: f64, candidate: f64) -> f64 {
    if delta1 * delta2 > 0.0 {
        if delta1 > 0.0 {
            candidate.clamp(0.0, 3.0 * delta1.min(delta2))
        } else {
            candidate.clamp(3.0 * delta1.max(delta2), 0.0)
        }
    } else {
        0.0
    }
}

fn project_endpoint(primary_delta: f64, other_delta: f64, candidate: f64) -> f64 {
    if primary_delta * candidate <= 0.0 {
        0.0
    } else if primary_delta * other_delta <= 0.0 && candidate.abs() > (3.0 * primary_delta).abs() {
        3.0 * primary_delta
    } else {
        candidate
    }
}

#[allow(clippy::many_single_char_names, clippy::too_many_lines)]
fn cubic_spline(
    x: &[f64],
    y: &[Complex64],
    start_slope: Option<Complex64>,
    end_slope: Option<Complex64>,
) -> Vec<[Complex64; 4]> {
    let n = x.len();
    let last_interval = n - 1;
    let zero = Complex64::new(0.0, 0.0);
    let one = Complex64::new(1.0, 0.0);
    let mut c = vec![vec![zero; n]; 4];
    c[0].clone_from_slice(y);
    if let Some(slope) = start_slope {
        c[1][0] = slope;
    }
    if let Some(slope) = end_slope {
        c[1][n - 1] = slope;
    }
    for m in 1..n {
        c[2][m] = Complex64::new(x[m] - x[m - 1], 0.0);
        c[3][m] = (c[0][m] - c[0][m - 1]) / c[2][m];
    }

    if start_slope.is_none() {
        if n > 2 {
            c[3][0] = c[2][2];
            c[2][0] = c[2][1] + c[2][2];
            c[1][0] = ((c[2][1] + 2.0 * c[2][0]) * c[3][1] * c[2][2] + c[2][1].powu(2) * c[3][2])
                / c[2][0];
        } else {
            c[3][0] = one;
            c[2][0] = one;
            c[1][0] = 2.0 * c[3][1];
        }
    } else {
        c[3][0] = one;
        c[2][0] = zero;
    }

    for m in 1..last_interval {
        let g = -c[2][m + 1] / c[3][m - 1];
        c[1][m] = g * c[1][m - 1] + 3.0 * (c[2][m] * c[3][m + 1] + c[2][m + 1] * c[3][m]);
        c[3][m] = g * c[2][m - 1] + 2.0 * (c[2][m] + c[2][m + 1]);
    }

    if end_slope.is_none() {
        let g;
        if n == 2 && start_slope.is_none() {
            c[1][n - 1] = c[3][n - 1];
            g = zero;
        } else if (n == 3 && start_slope.is_none()) || n == 2 {
            c[1][n - 1] = 2.0 * c[3][n - 1];
            c[3][n - 1] = one;
            g = -one / c[3][n - 2];
        } else {
            let sum = c[2][n - 2] + c[2][n - 1];
            c[1][n - 1] = ((c[2][n - 1] + 2.0 * sum) * c[3][n - 1] * c[2][n - 2]
                + c[2][n - 1].powu(2) * (c[0][n - 2] - c[0][n - 3]) / c[2][n - 2])
                / sum;
            g = -sum / c[3][n - 2];
            c[3][n - 1] = c[2][n - 2];
        }
        if start_slope.is_some() || n > 2 {
            c[3][n - 1] = g * c[2][n - 2] + c[3][n - 1];
            c[1][n - 1] = (g * c[1][n - 2] + c[1][n - 1]) / c[3][n - 1];
        }
    }

    for j in (0..last_interval).rev() {
        c[1][j] = (c[1][j] - c[2][j] * c[1][j + 1]) / c[3][j];
    }
    for i in 1..n {
        let interval = c[2][i];
        let first_difference = (c[0][i] - c[0][i - 1]) / interval;
        let third_difference = c[1][i - 1] + c[1][i] - 2.0 * first_difference;
        c[2][i - 1] = 2.0 * (first_difference - c[1][i - 1] - third_difference) / interval;
        c[3][i - 1] = (third_difference / interval) * (6.0 / interval);
    }

    (0..n - 1)
        .map(|index| [c[0][index], c[1][index], c[2][index], c[3][index]])
        .collect()
}

fn complex_speed(
    environment: &crate::model::EnvironmentCase,
    point: &SoundSpeedPoint,
) -> Complex64 {
    material_complex_speed(
        point.depth_m,
        point.compressional_speed_mps,
        point.compressional_attenuation,
        environment.frequency_hz,
        environment.top_options.attenuation_unit,
        &environment.top_options.volume_attenuation,
    )
}

pub(super) fn material_complex_speed(
    depth_m: f64,
    speed_mps: f64,
    attenuation: f64,
    frequency_hz: f64,
    unit: AttenuationUnit,
    volume: &VolumeAttenuation,
) -> Complex64 {
    let attenuation_nepers_per_m =
        attenuation_nepers_per_m(depth_m, speed_mps, attenuation, frequency_hz, unit, volume);
    Complex64::new(
        speed_mps,
        attenuation_nepers_per_m * speed_mps.powi(2) / (2.0 * PI * frequency_hz),
    )
}

fn attenuation_nepers_per_m(
    depth_m: f64,
    speed_mps: f64,
    attenuation: f64,
    frequency_hz: f64,
    unit: AttenuationUnit,
    volume: &VolumeAttenuation,
) -> f64 {
    let mut value = match unit {
        AttenuationUnit::NepersPerMeter => attenuation,
        AttenuationUnit::DbPerMeter => attenuation / 8.685_889_6,
        AttenuationUnit::DbPerMeterKhz => attenuation * frequency_hz / 8_685.889_6,
        AttenuationUnit::DbPerWavelength => {
            if speed_mps == 0.0 {
                0.0
            } else {
                attenuation * frequency_hz / (8.685_889_6 * speed_mps)
            }
        }
        AttenuationUnit::QualityFactor => {
            if speed_mps * attenuation == 0.0 {
                0.0
            } else {
                2.0 * PI * frequency_hz / (2.0 * speed_mps * attenuation)
            }
        }
        AttenuationUnit::LossParameter => {
            if speed_mps == 0.0 {
                0.0
            } else {
                attenuation * 2.0 * PI * frequency_hz / speed_mps
            }
        }
    };
    value += match volume {
        VolumeAttenuation::None => 0.0,
        VolumeAttenuation::Thorp => {
            let frequency_khz_squared = (frequency_hz / 1000.0).powi(2);
            (3.3e-3
                + 0.11 * frequency_khz_squared / (1.0 + frequency_khz_squared)
                + 44.0 * frequency_khz_squared / (4100.0 + frequency_khz_squared)
                + 3.0e-4 * frequency_khz_squared)
                / 8_685.889_6
        }
        VolumeAttenuation::FrancoisGarrison {
            temperature_c,
            salinity_psu,
            ph,
            mean_depth_m,
        } => {
            francois_garrison_db_per_km(
                frequency_hz / 1000.0,
                *temperature_c,
                *salinity_psu,
                *ph,
                *mean_depth_m,
            ) / 8_685.889_6
        }
        VolumeAttenuation::Biological { layers } => layers
            .iter()
            .filter(|layer| depth_m >= layer.top_depth_m && depth_m <= layer.bottom_depth_m)
            .map(|layer| {
                layer.attenuation
                    / ((1.0 - layer.resonance_frequency_hz.powi(2) / frequency_hz.powi(2)).powi(2)
                        + 1.0 / layer.quality_factor.powi(2))
                    / 8_685.889_6
            })
            .sum(),
    };
    value
}

fn francois_garrison_db_per_km(
    frequency_khz: f64,
    temperature_c: f64,
    salinity_psu: f64,
    ph: f64,
    mean_depth_m: f64,
) -> f64 {
    let speed = 1412.0 + 3.21 * temperature_c + 1.19 * salinity_psu + 0.0167 * mean_depth_m;
    let a1 = 8.86 / speed * 10.0_f64.powf(0.78 * ph - 5.0);
    let f1 =
        2.8 * (salinity_psu / 35.0).sqrt() * 10.0_f64.powf(4.0 - 1245.0 / (temperature_c + 273.0));
    let a2 = 21.44 * salinity_psu / speed * (1.0 + 0.025 * temperature_c);
    let p2 = 1.0 - 1.37e-4 * mean_depth_m + 6.2e-9 * mean_depth_m.powi(2);
    let f2 = 8.17 * 10.0_f64.powf(8.0 - 1990.0 / (temperature_c + 273.0))
        / (1.0 + 0.0018 * (salinity_psu - 35.0));
    let p3 = 1.0 - 3.83e-5 * mean_depth_m + 4.9e-10 * mean_depth_m.powi(2);
    let a3 = if temperature_c < 20.0 {
        4.937e-4 - 2.59e-5 * temperature_c + 9.11e-7 * temperature_c.powi(2)
            - 1.5e-8 * temperature_c.powi(3)
    } else {
        3.964e-4 - 1.146e-5 * temperature_c + 1.45e-7 * temperature_c.powi(2)
            - 6.5e-10 * temperature_c.powi(3)
    };
    let frequency_squared = frequency_khz.powi(2);
    a1 * f1 * frequency_squared / (f1.powi(2) + frequency_squared)
        + a2 * p2 * f2 * frequency_squared / (f2.powi(2) + frequency_squared)
        + a3 * p3 * frequency_squared
}

fn analytic_munk(depth_m: f64) -> SoundSpeedSample {
    let base_speed = 1500.0;
    let scaled_depth = 2.0 * (depth_m - 1300.0) / 1300.0;
    let scaled_depth_derivative = 2.0 / 1300.0;
    let exponential = (-scaled_depth).exp();
    let speed_mps = base_speed * (1.0 + 0.007_37 * (scaled_depth - 1.0 + exponential));
    let gradient_z = base_speed * 0.007_37 * (1.0 - exponential) * scaled_depth_derivative;
    SoundSpeedSample {
        speed_mps,
        imaginary_speed_mps: 0.0,
        gradient: [0.0, gradient_z],
        c_rr: 0.0,
        c_rz: 0.0,
        c_zz: base_speed * 0.007_37 * exponential * scaled_depth_derivative.powi(2),
        density_g_cm3: 1.0,
    }
}

#[cfg(test)]
mod tests {
    use num_complex::Complex64;

    use super::{cubic_spline, pchip};

    #[test]
    fn cubic_spline_reduces_two_points_to_a_line() {
        let coefficients = cubic_spline(
            &[0.0, 10.0],
            &[Complex64::new(1500.0, 0.0), Complex64::new(1520.0, 0.0)],
            None,
            None,
        );
        assert!((coefficients[0][1].re - 2.0).abs() < 1.0e-14);
        assert!(coefficients[0][2].norm() < 1.0e-14);
        assert!(coefficients[0][3].norm() < 1.0e-14);
    }

    #[test]
    fn pchip_preserves_monotone_nodes_without_overshoot() {
        let x = [0.0, 1.0, 2.0, 3.0];
        let y = [
            Complex64::new(0.0, 0.0),
            Complex64::new(1.0, 0.0),
            Complex64::new(1.5, 0.0),
            Complex64::new(4.0, 0.0),
        ];
        let coefficients = pchip(&x, &y);
        for (index, coefficient) in coefficients.iter().enumerate() {
            for sample in 0..=20 {
                let offset = f64::from(sample) / 20.0;
                let value = coefficient[0]
                    + offset
                        * (coefficient[1] + offset * (coefficient[2] + offset * coefficient[3]));
                assert!(value.re >= y[index].re - 1.0e-12);
                assert!(value.re <= y[index + 1].re + 1.0e-12);
            }
        }
    }
}

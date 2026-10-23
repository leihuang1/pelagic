use std::f64::consts::PI;

use num_complex::{Complex32, Complex64};

use crate::Case;
use crate::model::{BeamComponent, BeamFamily, BeamWidth, ReceiverGrid, RunKind, SourceGeometry};

use super::integrator::RayState;
use super::ssp::{SegmentState, SoundSpeedModel};
use super::{Arrival, RayTermination, ReceiverArrivals, ReceiverEigenrays, trajectory_from_states};

const PHASE_TOLERANCE: f64 = 0.05;

#[allow(clippy::struct_field_names)]
pub(super) struct InfluenceLimits {
    pub max_arrivals_per_receiver: usize,
    pub max_total_arrivals: usize,
    pub max_eigenrays: usize,
    pub max_eigenray_points: usize,
}

pub(super) struct InfluenceCounts {
    pub arrivals: usize,
    pub eigenrays: usize,
    pub eigenray_points: usize,
}

pub(super) enum InfluenceTarget<'a> {
    Eigenrays(&'a mut [ReceiverEigenrays]),
    Arrivals(&'a mut [ReceiverArrivals]),
    Field {
        pressure: &'a mut [Complex32],
        coherent: bool,
    },
}

#[allow(
    clippy::cast_sign_loss,
    clippy::needless_range_loop,
    clippy::too_many_arguments,
    clippy::too_many_lines
)]
pub(super) fn cerveny_cartesian(
    case: &Case,
    sound_speed: &SoundSpeedModel,
    states: &[RayState],
    launch_angle_radians: f64,
    angular_spacing_radians: f64,
    target: &mut InfluenceTarget<'_>,
) -> Result<(), &'static str> {
    if states.len() < 3 {
        return Ok(());
    }
    let options = case
        .environment
        .trace
        .cerveny
        .as_ref()
        .ok_or("Cerveny beam options are missing")?;
    let angular_frequency = 2.0 * PI * case.environment.frequency_hz;
    let epsilon = pick_epsilon(
        case,
        sound_speed,
        states[0],
        launch_angle_radians,
        angular_spacing_radians,
    )?;
    let p_beam: Vec<Complex64> = states
        .iter()
        .map(|state| Complex64::new(state.p[0], 0.0) + epsilon * state.p[1])
        .collect();
    let q_beam: Vec<Complex64> = states
        .iter()
        .map(|state| Complex64::new(state.q[0], 0.0) + epsilon * state.q[1])
        .collect();
    let mut gamma = Vec::with_capacity(states.len());
    let mut branch = Vec::with_capacity(states.len());
    let mut segments = SegmentState::default();
    for (state_index, state) in states.iter().enumerate() {
        let ray_tangent = [
            state.speed_mps * state.tangent[0],
            state.speed_mps * state.tangent[1],
        ];
        let ray_normal = [ray_tangent[1], -ray_tangent[0]];
        let sample = sound_speed.evaluate(state.position_m, &mut segments)?;
        let speed_squared = sample.speed_mps.powi(2);
        let tangent_gradient = dot(sample.gradient, ray_tangent);
        let normal_gradient = dot(sample.gradient, ray_normal);
        let tangent_range = ray_tangent[0];
        let tangent_depth = ray_tangent[1];
        let value = if q_beam[state_index] == Complex64::new(0.0, 0.0) {
            Complex64::new(0.0, 0.0)
        } else {
            0.5 * (p_beam[state_index] / q_beam[state_index] * tangent_range.powi(2)
                + 2.0 * normal_gradient / speed_squared * tangent_depth * tangent_range
                - tangent_gradient / speed_squared * tangent_depth.powi(2))
        };
        gamma.push(value);
        let mut index = branch.last().copied().unwrap_or(1);
        if state_index > 0 {
            branch_cut(
                q_beam[state_index - 1],
                q_beam[state_index],
                options.width,
                &mut index,
            );
        }
        branch.push(index);
    }

    let ranges = &case.environment.positions.receiver_ranges_m;
    let depths = &case.environment.positions.receiver_depths_m;
    let depth_count = match case.environment.run.receiver_grid {
        ReceiverGrid::Rectilinear => depths.len(),
        ReceiverGrid::Irregular => 1,
    };
    let ratio = match case.environment.run.source_geometry {
        SourceGeometry::Point => launch_angle_radians.cos().abs().sqrt(),
        SourceGeometry::Line => 1.0,
    };
    let radius_max = 50.0 * states[0].speed_mps / case.environment.frequency_hz;
    let beam_window_squared = f64::from(options.beam_window).powi(2);
    let image_count = options.image_count.max(0) as usize;
    let top_depth = case.environment.sound_speed.top_depth_m;
    let bottom_depth = case.environment.sound_speed.bottom_depth_m;

    for state_index in 2..states.len() {
        if state_index % 256 == 0 {
            crate::cancellation::check()?;
        }
        let previous = states[state_index - 1];
        let current = states[state_index];
        if current.position_m[0] > ranges[ranges.len() - 1] {
            return Ok(());
        }
        let range_a = previous.position_m[0];
        let range_b = current.position_m[0];
        if (range_b - range_a).abs() < 1.0e3 * spacing(range_b) {
            continue;
        }
        let receiver_a = uniform_receiver_index(ranges, range_a);
        let receiver_b = uniform_receiver_index(ranges, range_b);
        if receiver_a >= receiver_b {
            continue;
        }

        for receiver_index in receiver_a + 1..=receiver_b {
            let fraction = (ranges[receiver_index] - range_a) / (range_b - range_a);
            let position = [
                previous.position_m[0]
                    + fraction * (current.position_m[0] - previous.position_m[0]),
                previous.position_m[1]
                    + fraction * (current.position_m[1] - previous.position_m[1]),
            ];
            let tangent = [
                previous.tangent[0] + fraction * (current.tangent[0] - previous.tangent[0]),
                previous.tangent[1] + fraction * (current.tangent[1] - previous.tangent[1]),
            ];
            let speed = previous.speed_mps + fraction * (current.speed_mps - previous.speed_mps);
            let q = q_beam[state_index - 1]
                + fraction * (q_beam[state_index] - q_beam[state_index - 1]);
            let delay = previous.travel_time_s
                + fraction * (current.travel_time_s - previous.travel_time_s);
            let interpolated_gamma =
                gamma[state_index - 1] + fraction * (gamma[state_index] - gamma[state_index - 1]);
            if interpolated_gamma.im > 0.0 {
                continue;
            }
            // A complex numerator preserves -0 on the WKB square-root branch cut.
            let mut constant = ratio * (Complex64::new(speed * epsilon.norm(), 0.0) / q).sqrt();
            let mut branch_index = branch[state_index - 1];
            branch_cut(q_beam[state_index - 1], q, options.width, &mut branch_index);
            if branch_index < 0 {
                constant = -constant;
            }

            for (depth_index, &receiver_depth) in depths.iter().take(depth_count).enumerate() {
                let mut image_sum = Complex64::new(0.0, 0.0);
                let mut depth_delta = 0.0;
                let mut polarity = 1.0;
                for image in 0..image_count {
                    match image {
                        0 => {
                            depth_delta = receiver_depth - position[1];
                            polarity = 1.0;
                        }
                        1 => {
                            depth_delta = -receiver_depth + 2.0 * top_depth - position[1];
                            polarity = -1.0;
                        }
                        2 => {
                            depth_delta = -receiver_depth + 2.0 * bottom_depth - position[1];
                            polarity = 1.0;
                        }
                        _ => {}
                    }
                    if angular_frequency * interpolated_gamma.im * depth_delta.powi(2)
                        < beam_window_squared
                    {
                        let beam_delay = delay
                            + tangent[1] * depth_delta
                            + interpolated_gamma * depth_delta.powi(2);
                        let exponent = Complex64::new(0.0, -1.0)
                            * (angular_frequency * beam_delay - current.phase_radians);
                        image_sum += polarity
                            * current.amplitude
                            * hermite(depth_delta, radius_max, 2.0 * radius_max)
                            * exponent.exp();
                    }
                }
                let contribution = if case.environment.run.kind == RunKind::Coherent {
                    constant * image_sum
                } else {
                    Complex64::new((constant * image_sum).norm_sqr(), 0.0)
                };
                let cell_index = receiver_index * depth_count + depth_index;
                add_raw_field_contribution(target, cell_index, contribution)?;
            }
        }
    }
    Ok(())
}

#[allow(
    clippy::cast_sign_loss,
    clippy::needless_range_loop,
    clippy::too_many_arguments,
    clippy::too_many_lines
)]
pub(super) fn cerveny_ray_centered(
    case: &Case,
    sound_speed: &SoundSpeedModel,
    states: &[RayState],
    launch_angle_radians: f64,
    angular_spacing_radians: f64,
    target: &mut InfluenceTarget<'_>,
) -> Result<(), &'static str> {
    if states.len() < 2 {
        return Ok(());
    }
    let options = case
        .environment
        .trace
        .cerveny
        .as_ref()
        .ok_or("Cerveny beam options are missing")?;
    let angular_frequency = 2.0 * PI * case.environment.frequency_hz;
    let epsilon = pick_epsilon(
        case,
        sound_speed,
        states[0],
        launch_angle_radians,
        angular_spacing_radians,
    )?;
    let q_beam: Vec<Complex64> = states
        .iter()
        .map(|state| Complex64::new(state.q[0], 0.0) + epsilon * state.q[1])
        .collect();
    let gamma: Vec<Complex64> = states
        .iter()
        .zip(&q_beam)
        .map(|(state, &q)| (Complex64::new(state.p[0], 0.0) + epsilon * state.p[1]) / q)
        .collect();
    let mut branch = vec![1; states.len()];
    for state_index in 1..states.len() {
        branch[state_index] = branch[state_index - 1];
        branch_cut(
            q_beam[state_index - 1],
            q_beam[state_index],
            options.width,
            &mut branch[state_index],
        );
    }
    let normal_depth: Vec<f64> = states
        .iter()
        .map(|state| -state.tangent[0] * state.speed_mps)
        .collect();
    let mut normal_range: Vec<f64> = states
        .iter()
        .map(|state| state.tangent[1] * state.speed_mps)
        .collect();
    let ranges = &case.environment.positions.receiver_ranges_m;
    let depths = &case.environment.positions.receiver_depths_m;
    let depth_count = match case.environment.run.receiver_grid {
        ReceiverGrid::Rectilinear => depths.len(),
        ReceiverGrid::Irregular => 1,
    };
    let ratio = match case.environment.run.source_geometry {
        SourceGeometry::Point => launch_angle_radians.cos().abs().sqrt(),
        SourceGeometry::Line => 1.0,
    };
    let radius_max = 50.0 * states[0].speed_mps / case.environment.frequency_hz;
    let beam_window_squared = f64::from(options.beam_window).powi(2);
    let image_count = options.image_count.max(0) as usize;
    let top_depth = case.environment.sound_speed.top_depth_m;
    let bottom_depth = case.environment.sound_speed.bottom_depth_m;

    let mut stale_normal = 0.0;
    for (depth_index, &receiver_depth) in depths.iter().take(depth_count).enumerate() {
        for image in 0..image_count {
            let mut prior: Option<(f64, f64, usize)> = None;
            for state_index in 1..states.len() {
                if state_index % 256 == 0 {
                    crate::cancellation::check()?;
                }
                if normal_depth[state_index].abs() < f64::EPSILON {
                    continue;
                }
                if image == 1 || image == 2 {
                    for value in &mut normal_range {
                        *value = -*value;
                    }
                }
                let normal_b = match image {
                    0 => {
                        (receiver_depth - states[state_index].position_m[1])
                            / normal_depth[state_index]
                    }
                    1 => {
                        (receiver_depth - (2.0 * top_depth - states[state_index].position_m[1]))
                            / normal_depth[state_index]
                    }
                    2 => {
                        (receiver_depth - (2.0 * bottom_depth - states[state_index].position_m[1]))
                            / normal_depth[state_index]
                    }
                    _ => stale_normal,
                };
                stale_normal = normal_b;
                let range_b =
                    states[state_index].position_m[0] + normal_b * normal_range[state_index];
                let receiver_b = uniform_receiver_index(ranges, range_b);
                let Some((normal_a, range_a, receiver_a)) = prior else {
                    prior = Some((normal_b, range_b, receiver_b));
                    continue;
                };
                if receiver_a >= receiver_b
                    || (states[state_index].position_m[0] - states[state_index - 1].position_m[0])
                        .abs()
                        < 1.0e3 * spacing(states[state_index].position_m[0])
                {
                    prior = Some((normal_b, range_b, receiver_b));
                    continue;
                }

                for receiver_index in receiver_a + 1..=receiver_b {
                    let fraction = (ranges[receiver_index] - range_a) / (range_b - range_a);
                    let q = q_beam[state_index - 1]
                        + fraction * (q_beam[state_index] - q_beam[state_index - 1]);
                    let interpolated_gamma = gamma[state_index - 1]
                        + fraction * (gamma[state_index] - gamma[state_index - 1]);
                    let normal = normal_a + fraction * (normal_b - normal_a);
                    if interpolated_gamma.im > 0.0
                        || -0.5 * angular_frequency * interpolated_gamma.im * normal.powi(2)
                            >= beam_window_squared
                    {
                        continue;
                    }
                    let delay = states[state_index - 1].travel_time_s
                        + fraction
                            * (states[state_index].travel_time_s
                                - states[state_index - 1].travel_time_s);
                    let speed = states[state_index - 1].speed_mps;
                    let mut contribution = ratio
                        * states[state_index].amplitude
                        * (Complex64::new(speed * epsilon.norm(), 0.0) / q).sqrt()
                        * (Complex64::new(0.0, -1.0)
                            * (angular_frequency
                                * (delay + 0.5 * interpolated_gamma * normal.powi(2))
                                - states[state_index].phase_radians))
                            .exp();
                    match options.component {
                        BeamComponent::Vertical => {
                            let normal_pressure = Complex64::new(0.0, -1.0)
                                * angular_frequency
                                * interpolated_gamma
                                * normal
                                * contribution;
                            let tangent_pressure = Complex64::new(0.0, -1.0) * angular_frequency
                                / speed
                                * contribution;
                            contribution = speed
                                * (normal_pressure.conj() * states[state_index].tangent[0]
                                    + tangent_pressure.conj() * states[state_index].tangent[1]);
                        }
                        BeamComponent::Horizontal => {
                            let normal_pressure = Complex64::new(0.0, -1.0)
                                * angular_frequency
                                * interpolated_gamma
                                * normal
                                * contribution;
                            let tangent_pressure = Complex64::new(0.0, -1.0) * angular_frequency
                                / speed
                                * contribution;
                            contribution = speed
                                * (-normal_pressure * states[state_index].tangent[1]
                                    + tangent_pressure * states[state_index].tangent[0]);
                        }
                        BeamComponent::Pressure | BeamComponent::Displacement => {}
                    }
                    let mut branch_index = branch[state_index - 1];
                    branch_cut(q_beam[state_index - 1], q, options.width, &mut branch_index);
                    if branch_index < 0 {
                        contribution = -contribution;
                    }
                    if image == 1 {
                        contribution = -contribution;
                    }
                    if case.environment.run.kind != RunKind::Coherent {
                        contribution = Complex64::new(contribution.norm_sqr(), 0.0);
                    }
                    contribution *= hermite(normal, radius_max, 2.0 * radius_max);
                    let cell_index = receiver_index * depth_count + depth_index;
                    add_raw_field_contribution(target, cell_index, contribution)?;
                }
                prior = Some((normal_b, range_b, receiver_b));
            }
        }
    }
    Ok(())
}

fn pick_epsilon(
    case: &Case,
    sound_speed: &SoundSpeedModel,
    source: RayState,
    launch_angle_radians: f64,
    angular_spacing_radians: f64,
) -> Result<Complex64, &'static str> {
    let options = case
        .environment
        .trace
        .cerveny
        .as_ref()
        .ok_or("Cerveny beam options are missing")?;
    let angular_frequency = 2.0 * PI * case.environment.frequency_hz;
    let epsilon = match options.width {
        BeamWidth::SpaceFilling => {
            let half_width = 2.0 / (angular_frequency / source.speed_mps * angular_spacing_radians);
            Complex64::new(0.0, 0.5 * angular_frequency * half_width.powi(2))
        }
        BeamWidth::Minimum => {
            let half_width =
                (2.0 * source.speed_mps * 1000.0 * options.loop_range / angular_frequency).sqrt();
            Complex64::new(0.0, 0.5 * angular_frequency * half_width.powi(2))
        }
        BeamWidth::Wkb => {
            let sample = sound_speed.evaluate(source.position_m, &mut SegmentState::default())?;
            if sample.gradient[1] == 0.0 {
                Complex64::new(1.0e10, 0.0)
            } else {
                Complex64::new(
                    (-launch_angle_radians.sin() / launch_angle_radians.powi(2).cos())
                        * source.speed_mps.powi(2)
                        / sample.gradient[1],
                    0.0,
                )
            }
        }
    };
    Ok(options.epsilon_multiplier * epsilon)
}

fn branch_cut(q1: Complex64, q2: Complex64, width: BeamWidth, index: &mut i32) {
    let crossed = match width {
        BeamWidth::Wkb => (q1.re < 0.0 && q2.re >= 0.0) || (q1.re > 0.0 && q2.re <= 0.0),
        BeamWidth::SpaceFilling | BeamWidth::Minimum => {
            q2.re < 0.0 && ((q1.im < 0.0 && q2.im >= 0.0) || (q1.im > 0.0 && q2.im <= 0.0))
        }
    };
    if crossed {
        *index = -*index;
    }
}

fn hermite(value: f64, inner: f64, outer: f64) -> f64 {
    let absolute = value.abs();
    if absolute <= inner {
        1.0
    } else if absolute >= outer {
        0.0
    } else {
        let fraction = (absolute - inner) / (outer - inner);
        (1.0 + 2.0 * fraction) * (1.0 - fraction).powi(2)
    }
}

#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
pub(super) fn geo_hat_cartesian(
    case: &Case,
    states: &[RayState],
    launch_angle_radians: f64,
    angular_spacing_radians: f64,
    target: &mut InfluenceTarget<'_>,
    limits: &InfluenceLimits,
    counts: &mut InfluenceCounts,
) -> Result<(), &'static str> {
    if states.len() < 2 {
        return Ok(());
    }
    let positions = &case.environment.positions;
    let ranges = &positions.receiver_ranges_m;
    let depths = &positions.receiver_depths_m;
    let rectilinear = case.environment.run.receiver_grid == ReceiverGrid::Rectilinear;
    let depth_count = if rectilinear { depths.len() } else { 1 };
    let q0 = states[0].speed_mps / angular_spacing_radians;
    let launch_angle_degrees = launch_angle_radians.to_degrees();
    let ratio = match case.environment.run.source_geometry {
        SourceGeometry::Point => launch_angle_radians.cos().abs().sqrt(),
        SourceGeometry::Line => 1.0,
    };
    let mut caustic_phase = 0.0;
    let mut q_old = states[0].q[0];
    let mut range_a = states[0].position_m[0];
    let mut receiver_index = ranges
        .iter()
        .position(|range| *range > range_a)
        .unwrap_or(0);
    if states[0].tangent[0] < 0.0 && receiver_index > 0 {
        receiver_index -= 1;
    }

    for state_index in 1..states.len() {
        let previous = states[state_index - 1];
        let current = states[state_index];
        let range_b = current.position_m[0];
        let segment = [
            current.position_m[0] - previous.position_m[0],
            current.position_m[1] - previous.position_m[1],
        ];
        let segment_length = segment[0].hypot(segment[1]);
        if segment_length < 1.0e3 * spacing(current.position_m[0]) {
            continue;
        }
        let tangent = [segment[0] / segment_length, segment[1] / segment_length];
        let normal = [-tangent[1], tangent[0]];
        let receiver_angle_degrees = tangent[1].atan2(tangent[0]).to_degrees();
        let delta_q = current.q[0] - previous.q[0];
        let delta_time = current.travel_time_s - previous.travel_time_s;
        let segment_q = previous.q[0];
        if crosses_caustic(segment_q, q_old) {
            caustic_phase += PI / 2.0;
        }
        q_old = segment_q;
        let radius_projected =
            previous.q[0].abs().max(current.q[0].abs()) / q0.abs() / tangent[0].abs();
        let (minimum_depth, maximum_depth) = if tangent[0].abs() > 0.5 {
            (
                previous.position_m[1].min(current.position_m[1]) - radius_projected,
                previous.position_m[1].max(current.position_m[1]) + radius_projected,
            )
        } else {
            (f64::NEG_INFINITY, f64::INFINITY)
        };

        loop {
            let receiver_range = ranges[receiver_index];
            if receiver_range >= range_a.min(range_b) && receiver_range < range_a.max(range_b) {
                for depth_index in 0..depth_count {
                    let receiver_depth = if rectilinear {
                        depths[depth_index]
                    } else {
                        depths[receiver_index]
                    };
                    if receiver_depth < minimum_depth || receiver_depth > maximum_depth {
                        continue;
                    }
                    let receiver_delta = [
                        receiver_range - previous.position_m[0],
                        receiver_depth - previous.position_m[1],
                    ];
                    let fraction = dot(receiver_delta, tangent) / segment_length;
                    let normal_distance = dot(receiver_delta, normal).abs();
                    let interpolated_q = previous.q[0] + fraction * delta_q;
                    let radius = (interpolated_q / q0).abs();
                    if normal_distance >= radius {
                        continue;
                    }
                    let delay = previous.travel_time_s + fraction * delta_time;
                    let constant = ratio
                        * (current.speed_mps / interpolated_q.abs()).sqrt()
                        * current.amplitude;
                    let hat_weight = (radius - normal_distance) / radius;
                    let amplitude = constant * hat_weight;
                    let mut phase = previous.phase_radians + caustic_phase;
                    if crosses_caustic(interpolated_q, q_old) {
                        phase += PI / 2.0;
                    }
                    let cell_index = receiver_index * depth_count + depth_index;
                    apply_contribution(
                        target,
                        cell_index,
                        state_index,
                        states,
                        ArrivalCandidate {
                            amplitude,
                            phase_radians: phase,
                            delay,
                            source_angle_degrees: launch_angle_degrees,
                            receiver_angle_degrees,
                            top_bounces: current.top_bounces,
                            bottom_bounces: current.bottom_bounces,
                            incoherent_intensity: constant.powi(2) * hat_weight,
                        },
                        case.environment.frequency_hz,
                        limits,
                        counts,
                    )?;
                }
            }

            let next_index = if ranges[receiver_index] < range_b {
                if receiver_index + 1 >= ranges.len() {
                    break;
                }
                let next = receiver_index + 1;
                if ranges[next] >= range_b {
                    break;
                }
                next
            } else {
                if receiver_index == 0 {
                    break;
                }
                let next = receiver_index - 1;
                if ranges[next] <= range_b {
                    break;
                }
                next
            };
            receiver_index = next_index;
        }
        range_a = range_b;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
pub(super) fn geo_hat_ray_centered(
    case: &Case,
    states: &[RayState],
    launch_angle_radians: f64,
    angular_spacing_radians: f64,
    target: &mut InfluenceTarget<'_>,
    limits: &InfluenceLimits,
    counts: &mut InfluenceCounts,
) -> Result<(), &'static str> {
    if states.len() < 2 {
        return Ok(());
    }
    let positions = &case.environment.positions;
    let ranges = &positions.receiver_ranges_m;
    let depths = &positions.receiver_depths_m;
    let rectilinear = case.environment.run.receiver_grid == ReceiverGrid::Rectilinear;
    let depth_count = if rectilinear { depths.len() } else { 1 };
    let q0 = states[0].speed_mps / angular_spacing_radians;
    let launch_angle_degrees = launch_angle_radians.to_degrees();
    let ratio = match case.environment.run.source_geometry {
        SourceGeometry::Point => launch_angle_radians.cos().abs().sqrt(),
        SourceGeometry::Line => 1.0,
    };
    let normal_depth: Vec<f64> = states
        .iter()
        .map(|state| -state.tangent[0] * state.speed_mps)
        .collect();
    let normal_range: Vec<f64> = states
        .iter()
        .map(|state| state.tangent[1] * state.speed_mps)
        .collect();

    for (depth_index, &receiver_depth) in depths.iter().take(depth_count).enumerate() {
        let mut caustic_phase = 0.0;
        let mut q_old = states[0].q[0];
        let (mut normal_a, mut range_a, mut receiver_a) = if normal_depth[0].abs() < 1.0e-6 {
            (1.0e10, 1.0e10, 0)
        } else {
            let normal = (receiver_depth - states[0].position_m[1]) / normal_depth[0];
            let range = states[0].position_m[0] + normal * normal_range[0];
            (normal, range, uniform_receiver_index(ranges, range))
        };

        for state_index in 1..states.len() {
            if state_index % 256 == 0 {
                crate::cancellation::check()?;
            }
            let previous = states[state_index - 1];
            let current = states[state_index];
            if normal_depth[state_index].abs() < 1.0e-10 {
                continue;
            }
            let normal_b = (receiver_depth - current.position_m[1]) / normal_depth[state_index];
            let range_b = current.position_m[0] + normal_b * normal_range[state_index];
            let receiver_b = uniform_receiver_index(ranges, range_b);
            if (current.position_m[0] - previous.position_m[0]).abs()
                < 1.0e3 * spacing(current.position_m[0])
                || receiver_a == receiver_b
            {
                normal_a = normal_b;
                range_a = range_b;
                receiver_a = receiver_b;
                continue;
            }

            let segment_q = previous.q[0];
            if crosses_caustic(segment_q, q_old) {
                caustic_phase += PI / 2.0;
            }
            q_old = segment_q;
            let receiver_angle_degrees =
                (current.tangent[1]).atan2(current.tangent[0]).to_degrees();
            let backwards = receiver_b <= receiver_a;
            let mut receiver_index = if backwards {
                receiver_a
            } else {
                receiver_a + 1
            };
            let final_index = if backwards {
                receiver_b + 1
            } else {
                receiver_b
            };
            loop {
                let fraction = (ranges[receiver_index] - range_a) / (range_b - range_a);
                let normal_distance = (normal_a + fraction * (normal_b - normal_a)).abs();
                let interpolated_q = previous.q[0] + fraction * (current.q[0] - previous.q[0]);
                let radius = interpolated_q.abs() / q0;
                if normal_distance < radius {
                    let delay = previous.travel_time_s
                        + fraction * (current.travel_time_s - previous.travel_time_s);
                    let constant = ratio * current.speed_mps.sqrt() * current.amplitude
                        / interpolated_q.abs().sqrt();
                    let hat_weight = (radius - normal_distance) / radius;
                    let amplitude = constant * hat_weight;
                    let mut phase = previous.phase_radians + caustic_phase;
                    if crosses_caustic(interpolated_q, q_old) {
                        phase += PI / 2.0;
                    }
                    let cell_index = receiver_index * depth_count + depth_index;
                    apply_contribution(
                        target,
                        cell_index,
                        state_index,
                        states,
                        ArrivalCandidate {
                            amplitude,
                            phase_radians: phase,
                            delay,
                            source_angle_degrees: launch_angle_degrees,
                            receiver_angle_degrees,
                            top_bounces: current.top_bounces,
                            bottom_bounces: current.bottom_bounces,
                            incoherent_intensity: constant.powi(2) * hat_weight,
                        },
                        case.environment.frequency_hz,
                        limits,
                        counts,
                    )?;
                }
                if receiver_index == final_index {
                    break;
                }
                if backwards {
                    receiver_index -= 1;
                } else {
                    receiver_index += 1;
                }
            }
            normal_a = normal_b;
            range_a = range_b;
            receiver_a = receiver_b;
        }
    }
    Ok(())
}

#[allow(
    clippy::cast_precision_loss,
    clippy::similar_names,
    clippy::too_many_arguments,
    clippy::too_many_lines
)]
pub(super) fn simple_gaussian(
    case: &Case,
    states: &[RayState],
    launch_angle_radians: f64,
    angular_spacing_radians: f64,
    target: &mut InfluenceTarget<'_>,
    limits: &InfluenceLimits,
    counts: &mut InfluenceCounts,
) -> Result<(), &'static str> {
    if states.len() < 2 {
        return Ok(());
    }
    let ranges = &case.environment.positions.receiver_ranges_m;
    let depths = &case.environment.positions.receiver_depths_m;
    let depth_count = match case.environment.run.receiver_grid {
        ReceiverGrid::Rectilinear => depths.len(),
        ReceiverGrid::Irregular => 1,
    };
    let launch_angle_degrees = launch_angle_radians.to_degrees();
    let angular_frequency = 2.0 * PI * case.environment.frequency_hz;
    let ratio = launch_angle_radians.cos().sqrt();
    let beam_factor = f64::from(0.98_f32); // Reference BETA promotes a default-real literal.
    let decay = -4.0 * beam_factor.ln() / angular_spacing_radians.powi(2);
    let normalization = angular_spacing_radians * (decay / PI).sqrt();
    let step_m = if case.environment.trace.step_m == 0.0 {
        (case.environment.sound_speed.bottom_depth_m - case.environment.sound_speed.top_depth_m)
            / 10.0
    } else {
        case.environment.trace.step_m
    };
    let mut phase = 0.0;
    let mut q_old = 1.0;
    let mut range_a = states[0].position_m[0];
    let mut receiver_index = 0_usize;
    for state_index in 1..states.len() {
        if state_index % 256 == 0 {
            crate::cancellation::check()?;
        }
        let previous = states[state_index - 1];
        let current = states[state_index];
        let range_b = current.position_m[0];
        let segment_q = previous.q[0];
        if crosses_simple_caustic(segment_q, q_old) {
            phase += PI / 2.0;
        }
        q_old = segment_q;
        while (range_b - range_a).abs() > 1.0e3 * spacing(range_a)
            && range_b > ranges[receiver_index]
        {
            let fraction = (ranges[receiver_index] - range_a) / (range_b - range_a);
            let position = [
                previous.position_m[0]
                    + fraction * (current.position_m[0] - previous.position_m[0]),
                previous.position_m[1]
                    + fraction * (current.position_m[1] - previous.position_m[1]),
            ];
            let tangent = [
                previous.tangent[0] + fraction * (current.tangent[0] - previous.tangent[0]),
                previous.tangent[1] + fraction * (current.tangent[1] - previous.tangent[1]),
            ];
            let q = previous.q[0] + fraction * (current.q[0] - previous.q[0]);
            let interpolated_delay = previous.travel_time_s
                + fraction * (current.travel_time_s - previous.travel_time_s);
            let integrated_distance = state_index as f64 * step_m + fraction * step_m;
            if crosses_simple_caustic(q, q_old) {
                phase += PI / 2.0;
            }

            for (depth_index, &receiver_depth) in depths.iter().take(depth_count).enumerate() {
                let cell_index = receiver_index * depth_count + depth_index;
                if matches!(target, InfluenceTarget::Eigenrays(_)) {
                    apply_contribution(
                        target,
                        cell_index,
                        state_index,
                        states,
                        ArrivalCandidate {
                            amplitude: 0.0,
                            phase_radians: 0.0,
                            delay: Complex64::new(0.0, 0.0),
                            source_angle_degrees: launch_angle_degrees,
                            receiver_angle_degrees: 0.0,
                            top_bounces: current.top_bounces,
                            bottom_bounces: current.bottom_bounces,
                            incoherent_intensity: 0.0,
                        },
                        case.environment.frequency_hz,
                        limits,
                        counts,
                    )?;
                    continue;
                }
                let depth_delta = receiver_depth - position[1];
                let range_delta = range_b - range_a;
                let step_depth = current.position_m[1] - previous.position_m[1];
                let closest_approach =
                    (depth_delta * range_delta).abs() / range_delta.hypot(step_depth);
                let along_distance = (depth_delta.powi(2) - closest_approach.powi(2)).sqrt();
                let source_distance = integrated_distance + along_distance;
                let angle = (closest_approach / source_distance).atan();
                let contribution_delay = interpolated_delay + tangent[1] * depth_delta;
                let exponent = Complex64::new(-decay * angle.powi(2), 0.0)
                    + Complex64::new(0.0, -1.0)
                        * (angular_frequency * contribution_delay - current.phase_radians - phase);
                let contribution = ratio * normalization * current.amplitude * exponent.exp()
                    / source_distance.sqrt();
                add_raw_field_contribution(target, cell_index, contribution)?;
            }
            q_old = q;
            receiver_index += 1;
            if receiver_index >= ranges.len() {
                return Ok(());
            }
        }
        range_a = range_b;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
pub(super) fn geo_gaussian_cartesian(
    case: &Case,
    states: &[RayState],
    launch_angle_radians: f64,
    angular_spacing_radians: f64,
    target: &mut InfluenceTarget<'_>,
    limits: &InfluenceLimits,
    counts: &mut InfluenceCounts,
) -> Result<(), &'static str> {
    if states.len() < 2 {
        return Ok(());
    }
    let positions = &case.environment.positions;
    let ranges = &positions.receiver_ranges_m;
    let depths = &positions.receiver_depths_m;
    let rectilinear = case.environment.run.receiver_grid == ReceiverGrid::Rectilinear;
    let depth_count = if rectilinear { depths.len() } else { 1 };
    let q0 = states[0].speed_mps / angular_spacing_radians;
    let launch_angle_degrees = launch_angle_radians.to_degrees();
    let ratio = match case.environment.run.source_geometry {
        SourceGeometry::Point => launch_angle_radians.cos().abs().sqrt() / (2.0 * PI).sqrt(),
        SourceGeometry::Line => 1.0 / (2.0 * PI).sqrt(),
    };
    let mut caustic_phase = 0.0;
    let mut q_old = states[0].q[0];
    let mut range_a = states[0].position_m[0];
    let mut receiver_index = ranges
        .iter()
        .position(|range| *range > range_a)
        .unwrap_or(0);
    if states[0].tangent[0] < 0.0 && receiver_index > 0 {
        receiver_index -= 1;
    }

    for state_index in 1..states.len() {
        let previous = states[state_index - 1];
        let current = states[state_index];
        let range_b = current.position_m[0];
        let segment = [
            current.position_m[0] - previous.position_m[0],
            current.position_m[1] - previous.position_m[1],
        ];
        let segment_length = segment[0].hypot(segment[1]);
        if segment_length < 1.0e3 * spacing(current.position_m[0]) {
            continue;
        }
        let tangent = [segment[0] / segment_length, segment[1] / segment_length];
        let normal = [-tangent[1], tangent[0]];
        let receiver_angle_degrees = tangent[1].atan2(tangent[0]).to_degrees();
        let delta_q = current.q[0] - previous.q[0];
        let delta_time = current.travel_time_s - previous.travel_time_s;
        let segment_q = previous.q[0];
        if crosses_caustic(segment_q, q_old) {
            caustic_phase += PI / 2.0;
        }
        q_old = segment_q;
        let wavelength = previous.speed_mps / case.environment.frequency_hz;
        let mut projected_sigma =
            previous.q[0].abs().max(current.q[0].abs()) / q0.abs() / tangent[0].abs();
        // The reference's near-field coefficient is a default-real literal.
        let diffraction_floor =
            (f64::from(0.2_f32) * case.environment.frequency_hz * current.travel_time_s.re)
                .min(PI * wavelength);
        projected_sigma = projected_sigma.max(diffraction_floor);
        let radius = 4.0 * projected_sigma;
        let (minimum_depth, maximum_depth) = if tangent[0].abs() > 0.5 {
            (
                previous.position_m[1].min(current.position_m[1]) - radius,
                previous.position_m[1].max(current.position_m[1]) + radius,
            )
        } else {
            (f64::NEG_INFINITY, f64::INFINITY)
        };

        loop {
            let receiver_range = ranges[receiver_index];
            if receiver_range >= range_a.min(range_b) && receiver_range < range_a.max(range_b) {
                for depth_index in 0..depth_count {
                    let receiver_depth = if rectilinear {
                        depths[depth_index]
                    } else {
                        depths[receiver_index]
                    };
                    if receiver_depth < minimum_depth || receiver_depth > maximum_depth {
                        continue;
                    }
                    let receiver_delta = [
                        receiver_range - previous.position_m[0],
                        receiver_depth - previous.position_m[1],
                    ];
                    let fraction = dot(receiver_delta, tangent) / segment_length;
                    let normal_distance = dot(receiver_delta, normal).abs();
                    let interpolated_q = previous.q[0] + fraction * delta_q;
                    let mut sigma = (interpolated_q / q0).abs();
                    sigma = sigma.max(diffraction_floor);
                    if normal_distance >= 4.0 * sigma {
                        continue;
                    }
                    let spreading = (q0 / interpolated_q).abs();
                    let delay = previous.travel_time_s + fraction * delta_time;
                    let constant = ratio
                        * (current.speed_mps / interpolated_q.abs()).sqrt()
                        * current.amplitude;
                    let gaussian_weight =
                        (-0.5 * (normal_distance / sigma).powi(2)).exp() / (sigma * spreading);
                    let amplitude = constant * gaussian_weight;
                    let mut phase = previous.phase_radians + caustic_phase;
                    if crosses_caustic(interpolated_q, q_old) {
                        phase += PI / 2.0;
                    }
                    let cell_index = receiver_index * depth_count + depth_index;
                    apply_contribution(
                        target,
                        cell_index,
                        state_index,
                        states,
                        ArrivalCandidate {
                            amplitude,
                            phase_radians: phase,
                            delay,
                            source_angle_degrees: launch_angle_degrees,
                            receiver_angle_degrees,
                            top_bounces: current.top_bounces,
                            bottom_bounces: current.bottom_bounces,
                            incoherent_intensity: (2.0 * PI).sqrt()
                                * constant.powi(2)
                                * gaussian_weight,
                        },
                        case.environment.frequency_hz,
                        limits,
                        counts,
                    )?;
                }
            }

            let next_index = if range_b > ranges[receiver_index] {
                if receiver_index + 1 >= ranges.len() {
                    break;
                }
                let next = receiver_index + 1;
                if ranges[next] >= range_b {
                    break;
                }
                next
            } else {
                if receiver_index == 0 {
                    break;
                }
                let next = receiver_index - 1;
                if ranges[next] <= range_b {
                    break;
                }
                next
            };
            receiver_index = next_index;
        }
        range_a = range_b;
    }
    Ok(())
}

#[allow(clippy::cast_possible_truncation, clippy::too_many_arguments)]
fn apply_contribution(
    target: &mut InfluenceTarget<'_>,
    cell_index: usize,
    state_index: usize,
    states: &[RayState],
    candidate: ArrivalCandidate,
    frequency_hz: f64,
    limits: &InfluenceLimits,
    counts: &mut InfluenceCounts,
) -> Result<(), &'static str> {
    if cell_index.is_multiple_of(256) {
        crate::cancellation::check()?;
    }
    match target {
        InfluenceTarget::Eigenrays(receivers) => {
            let point_count = state_index + 1;
            if counts.eigenrays >= limits.max_eigenrays
                || counts.eigenray_points.saturating_add(point_count) > limits.max_eigenray_points
            {
                return Err("eigenray resource limit exceeded");
            }
            receivers[cell_index].eigenrays.push(trajectory_from_states(
                candidate.source_angle_degrees,
                &states[..=state_index],
                RayTermination::ReceiverHit,
            ));
            counts.eigenrays += 1;
            counts.eigenray_points += point_count;
            Ok(())
        }
        InfluenceTarget::Arrivals(receivers) => add_arrival(
            &mut receivers[cell_index],
            candidate,
            frequency_hz,
            limits,
            counts,
        ),
        InfluenceTarget::Field { pressure, coherent } => {
            let contribution = if *coherent {
                let angular_frequency = 2.0 * PI * frequency_hz;
                let exponent = Complex64::new(0.0, -1.0)
                    * (angular_frequency * candidate.delay - candidate.phase_radians);
                candidate.amplitude * exponent.exp()
            } else {
                let attenuation = (2.0 * PI * frequency_hz * candidate.delay.im).exp();
                Complex64::new(candidate.incoherent_intensity * attenuation.powi(2), 0.0)
            };
            pressure[cell_index] += Complex32::new(contribution.re as f32, contribution.im as f32);
            Ok(())
        }
    }
}

#[allow(clippy::cast_possible_truncation)]
pub(super) fn scale_pressure(
    case: &Case,
    angular_spacing_radians: f64,
    source_speed_mps: f64,
    pressure: &mut [Complex32],
) {
    let cerveny_scale = matches!(
        case.environment.run.beam_family,
        Some(BeamFamily::CervenyCartesian | BeamFamily::CervenyRayCentered)
    );
    let constant = if cerveny_scale {
        -angular_spacing_radians * case.environment.frequency_hz.sqrt() / source_speed_mps
    } else {
        -1.0
    };
    if case.environment.run.kind != RunKind::Coherent {
        for sample in pressure.iter_mut() {
            *sample = Complex32::new(sample.re.sqrt(), 0.0);
        }
    }
    let depth_count = match case.environment.run.receiver_grid {
        ReceiverGrid::Rectilinear => case.environment.positions.receiver_depths_m.len(),
        ReceiverGrid::Irregular => 1,
    };
    for (range_index, &range_m) in case
        .environment
        .positions
        .receiver_ranges_m
        .iter()
        .enumerate()
    {
        let factor = match case.environment.run.source_geometry {
            // ScalePressure declares its local pi and square root in single precision.
            SourceGeometry::Line => f64::from(-4.0 * std::f32::consts::PI.sqrt()) * constant,
            SourceGeometry::Point if range_m == 0.0 => 0.0,
            SourceGeometry::Point => constant / range_m.abs().sqrt(),
        } as f32;
        for depth_index in 0..depth_count {
            pressure[range_index * depth_count + depth_index] *= factor;
        }
    }
}

#[allow(clippy::cast_possible_truncation)]
pub(super) fn scale_arrivals(case: &Case, receivers: &mut [ReceiverArrivals]) {
    for receiver in receivers {
        let factor = match case.environment.run.source_geometry {
            SourceGeometry::Line => 4.0 * PI.sqrt(),
            SourceGeometry::Point if receiver.range_m == 0.0 => 1.0e5,
            SourceGeometry::Point => 1.0 / receiver.range_m.sqrt(),
        } as f32;
        for arrival in &mut receiver.arrivals {
            arrival.amplitude *= factor;
        }
    }
}

struct ArrivalCandidate {
    amplitude: f64,
    phase_radians: f64,
    delay: Complex64,
    source_angle_degrees: f64,
    receiver_angle_degrees: f64,
    top_bounces: u32,
    bottom_bounces: u32,
    incoherent_intensity: f64,
}

fn add_arrival(
    receiver: &mut ReceiverArrivals,
    candidate: ArrivalCandidate,
    frequency_hz: f64,
    limits: &InfluenceLimits,
    counts: &mut InfluenceCounts,
) -> Result<(), &'static str> {
    let angular_frequency = 2.0 * PI * frequency_hz;
    let combine = receiver.arrivals.last().is_some_and(|last| {
        angular_frequency
            * (candidate.delay
                - Complex64::new(
                    f64::from(last.travel_time_s),
                    f64::from(last.attenuation_time_s),
                ))
            .norm()
            < PHASE_TOLERANCE
            && (f64::from(last.phase_radians) - candidate.phase_radians).abs() < PHASE_TOLERANCE
    });
    if combine {
        combine_arrival(
            receiver.arrivals.last_mut().expect("arrival was present"),
            &candidate,
        );
        return Ok(());
    }

    let arrival = candidate.into_arrival();
    if receiver.arrivals.len() >= limits.max_arrivals_per_receiver {
        let (weakest_index, weakest) = receiver
            .arrivals
            .iter()
            .enumerate()
            .min_by(|left, right| left.1.amplitude.total_cmp(&right.1.amplitude))
            .expect("arrival limit is positive");
        if arrival.amplitude > weakest.amplitude {
            receiver.arrivals[weakest_index] = arrival;
        }
        return Ok(());
    }
    if counts.arrivals >= limits.max_total_arrivals {
        return Err("total arrival resource limit exceeded");
    }
    receiver.arrivals.push(arrival);
    counts.arrivals += 1;
    Ok(())
}

#[allow(clippy::cast_possible_truncation)]
fn combine_arrival(arrival: &mut Arrival, candidate: &ArrivalCandidate) {
    let added_amplitude = candidate.amplitude as f32;
    let total_amplitude = arrival.amplitude + added_amplitude;
    let old_weight = arrival.amplitude / total_amplitude;
    let new_weight = added_amplitude / total_amplitude;
    let old_delay = Complex32::new(arrival.travel_time_s, arrival.attenuation_time_s);
    let new_delay = Complex32::new(candidate.delay.re as f32, candidate.delay.im as f32);
    let delay = old_weight * old_delay + new_weight * new_delay;
    arrival.amplitude = total_amplitude;
    arrival.travel_time_s = delay.re;
    arrival.attenuation_time_s = delay.im;
    arrival.source_angle_degrees = old_weight * arrival.source_angle_degrees
        + new_weight * candidate.source_angle_degrees as f32;
    arrival.receiver_angle_degrees = old_weight * arrival.receiver_angle_degrees
        + new_weight * candidate.receiver_angle_degrees as f32;
}

impl ArrivalCandidate {
    #[allow(clippy::cast_possible_truncation)]
    fn into_arrival(self) -> Arrival {
        Arrival {
            amplitude: self.amplitude as f32,
            phase_radians: self.phase_radians as f32,
            travel_time_s: self.delay.re as f32,
            attenuation_time_s: self.delay.im as f32,
            source_angle_degrees: self.source_angle_degrees as f32,
            receiver_angle_degrees: self.receiver_angle_degrees as f32,
            top_bounces: self.top_bounces,
            bottom_bounces: self.bottom_bounces,
        }
    }
}

#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss
)]
fn uniform_receiver_index(ranges: &[f64], range: f64) -> usize {
    if ranges.len() == 1 {
        return 0;
    }
    let spacing = ranges[ranges.len() - 1] - ranges[ranges.len() - 2];
    let index = ((range - ranges[0]) / spacing).trunc();
    if !index.is_finite() || index <= 0.0 {
        0
    } else if index >= (ranges.len() - 1) as f64 {
        ranges.len() - 1
    } else {
        index as usize
    }
}

#[allow(clippy::cast_possible_truncation)]
fn add_raw_field_contribution(
    target: &mut InfluenceTarget<'_>,
    cell_index: usize,
    contribution: Complex64,
) -> Result<(), &'static str> {
    if cell_index.is_multiple_of(256) {
        crate::cancellation::check()?;
    }
    let InfluenceTarget::Field { pressure, .. } = target else {
        return Err("simple-Gaussian arrivals are not supported");
    };
    pressure[cell_index] += Complex32::new(contribution.re as f32, contribution.im as f32);
    Ok(())
}

fn crosses_caustic(q: f64, old_q: f64) -> bool {
    (q <= 0.0 && old_q > 0.0) || (q >= 0.0 && old_q < 0.0)
}

fn crosses_simple_caustic(q: f64, old_q: f64) -> bool {
    (q < 0.0 && old_q >= 0.0) || (q > 0.0 && old_q <= 0.0)
}

fn dot(left: [f64; 2], right: [f64; 2]) -> f64 {
    left[0] * right[0] + left[1] * right[1]
}

fn spacing(value: f64) -> f64 {
    let magnitude = value.abs();
    if !magnitude.is_finite() {
        return f64::NAN;
    }
    if magnitude == 0.0 {
        return f64::MIN_POSITIVE * f64::EPSILON;
    }
    let bits = magnitude.to_bits();
    f64::from_bits(bits + 1) - magnitude
}

#[cfg(test)]
mod tests {
    use super::{crosses_caustic, spacing};

    #[test]
    #[allow(clippy::float_cmp)]
    fn spacing_matches_binary64_bins() {
        assert_eq!(spacing(1.0), f64::EPSILON);
        assert_eq!(spacing(-2.0), 2.0 * f64::EPSILON);
        assert_eq!(spacing(0.0), f64::from_bits(1));
    }

    #[test]
    fn caustic_crossing_uses_reference_inclusive_zero_test() {
        assert!(crosses_caustic(0.0, 1.0));
        assert!(crosses_caustic(0.0, -1.0));
        assert!(!crosses_caustic(1.0, 1.0));
    }
}

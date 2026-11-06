use num_complex::Complex64;

use super::boundary::{BoundaryCurve, add, dot, scale, subtract};
use super::ssp::{SegmentState, SoundSpeedModel};

#[derive(Clone, Copy, Debug)]
pub(crate) struct RayState {
    pub position_m: [f64; 2],
    pub tangent: [f64; 2],
    pub p: [f64; 2],
    pub q: [f64; 2],
    pub speed_mps: f64,
    pub travel_time_s: Complex64,
    pub amplitude: f64,
    pub phase_radians: f64,
    pub top_bounces: u32,
    pub bottom_bounces: u32,
}

#[derive(Clone, Copy, Debug)]
#[allow(clippy::struct_field_names)]
pub(crate) struct StepLimits {
    pub base_step_m: f64,
    pub max_range_m: f64,
    pub max_depth_m: f64,
}

#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
pub(crate) fn step_2d(
    initial: RayState,
    sound_speed: &SoundSpeedModel,
    segments: &mut SegmentState,
    top: &BoundaryCurve,
    top_segment: usize,
    bottom: &BoundaryCurve,
    bottom_segment: usize,
    limits: StepLimits,
    consecutive_small_steps: &mut usize,
) -> Result<RayState, &'static str> {
    let sample0 = sound_speed.evaluate(initial.position_m, segments)?;
    let c0_squared = sample0.speed_mps * sample0.speed_mps;
    let curvature0 = normal_curvature(&initial, &sample0);
    let initial_segments = *segments;
    let mut step_m = limits.base_step_m;
    let unit_tangent0 = scale(initial.tangent, sample0.speed_mps);
    reduce_step(
        initial.position_m,
        unit_tangent0,
        initial_segments,
        sound_speed,
        top,
        top_segment,
        bottom,
        bottom_segment,
        limits,
        &mut step_m,
        consecutive_small_steps,
    );
    let half_step = 0.5 * step_m;

    let midpoint = RayState {
        position_m: add(initial.position_m, scale(unit_tangent0, half_step)),
        tangent: subtract(
            initial.tangent,
            scale(sample0.gradient, half_step / c0_squared),
        ),
        p: subtract(initial.p, scale(initial.q, half_step * curvature0)),
        q: add(initial.q, scale(initial.p, half_step * sample0.speed_mps)),
        ..initial
    };

    let sample1 = sound_speed.evaluate(midpoint.position_m, segments)?;
    let c1_squared = sample1.speed_mps * sample1.speed_mps;
    let curvature1 = normal_curvature(&midpoint, &sample1);
    let unit_tangent1 = scale(midpoint.tangent, sample1.speed_mps);
    reduce_step(
        initial.position_m,
        unit_tangent1,
        initial_segments,
        sound_speed,
        top,
        top_segment,
        bottom,
        bottom_segment,
        limits,
        &mut step_m,
        consecutive_small_steps,
    );

    let midpoint_weight = step_m / (2.0 * half_step);
    let initial_weight = 1.0 - midpoint_weight;
    let weighted_initial_step = step_m * initial_weight;
    let weighted_midpoint_step = step_m * midpoint_weight;
    let mut final_state = RayState {
        position_m: add(
            add(
                initial.position_m,
                scale(unit_tangent0, weighted_initial_step),
            ),
            scale(unit_tangent1, weighted_midpoint_step),
        ),
        tangent: subtract(
            subtract(
                initial.tangent,
                scale(sample0.gradient, weighted_initial_step / c0_squared),
            ),
            scale(sample1.gradient, weighted_midpoint_step / c1_squared),
        ),
        p: subtract(
            subtract(
                initial.p,
                scale(initial.q, weighted_initial_step * curvature0),
            ),
            scale(midpoint.q, weighted_midpoint_step * curvature1),
        ),
        q: add(
            add(
                initial.q,
                scale(initial.p, weighted_initial_step * sample0.speed_mps),
            ),
            scale(midpoint.p, weighted_midpoint_step * sample1.speed_mps),
        ),
        travel_time_s: initial.travel_time_s
            + weighted_initial_step
                / Complex64::new(sample0.speed_mps, sample0.imaginary_speed_mps)
            + weighted_midpoint_step
                / Complex64::new(sample1.speed_mps, sample1.imaginary_speed_mps),
        ..initial
    };

    let sample2 = sound_speed.evaluate(final_state.position_m, segments)?;
    final_state.speed_mps = sample2.speed_mps;
    if *segments != initial_segments {
        let gradient_jump = subtract(sample2.gradient, sample0.gradient);
        let ray_normal = [-final_state.tangent[1], final_state.tangent[0]];
        let normal_jump = dot(gradient_jump, ray_normal);
        let tangent_jump = dot(gradient_jump, final_state.tangent);
        let incidence_tangent = if segments.depth == initial_segments.depth {
            -final_state.tangent[1] / final_state.tangent[0]
        } else {
            final_state.tangent[0] / final_state.tangent[1]
        };
        let curvature_jump = incidence_tangent
            * (2.0 * normal_jump - incidence_tangent * tangent_jump)
            / sample2.speed_mps;
        final_state.p = subtract(final_state.p, scale(final_state.q, curvature_jump));
    }

    if state_is_finite(final_state) {
        Ok(final_state)
    } else {
        Err("ray integrator produced a non-finite state")
    }
}

fn normal_curvature(ray: &RayState, sample: &super::ssp::SoundSpeedSample) -> f64 {
    sample.c_rr * ray.tangent[1].powi(2) - 2.0 * sample.c_rz * ray.tangent[0] * ray.tangent[1]
        + sample.c_zz * ray.tangent[0].powi(2)
}

// Measured hot path: called twice in every ray step.
#[allow(clippy::too_many_arguments, clippy::inline_always)]
#[inline(always)]
fn reduce_step(
    initial_position_m: [f64; 2],
    unit_tangent: [f64; 2],
    initial_segments: SegmentState,
    sound_speed: &SoundSpeedModel,
    top: &BoundaryCurve,
    top_segment: usize,
    bottom: &BoundaryCurve,
    bottom_segment: usize,
    limits: StepLimits,
    step_m: &mut f64,
    consecutive_small_steps: &mut usize,
) {
    let trial_position_m = add(initial_position_m, scale(unit_tangent, *step_m));
    let mut interface_step = f64::MAX;
    if unit_tangent[1].abs() > f64::EPSILON {
        if sound_speed.depth_interface(initial_segments.depth) > trial_position_m[1] {
            interface_step = (sound_speed.depth_interface(initial_segments.depth)
                - initial_position_m[1])
                / unit_tangent[1];
        } else if sound_speed.depth_interface(initial_segments.depth + 1) < trial_position_m[1] {
            interface_step = (sound_speed.depth_interface(initial_segments.depth + 1)
                - initial_position_m[1])
                / unit_tangent[1];
        }
    }

    let top_boundary = top.segment(top_segment);
    let mut top_step = f64::MAX;
    if dot(
        top_boundary.normal,
        subtract(trial_position_m, top_boundary.origin_m),
    ) > f64::EPSILON
    {
        top_step = -dot(
            subtract(initial_position_m, top_boundary.origin_m),
            top_boundary.normal,
        ) / dot(unit_tangent, top_boundary.normal);
    }

    let bottom_boundary = bottom.segment(bottom_segment);
    let mut bottom_step = f64::MAX;
    if dot(
        bottom_boundary.normal,
        subtract(trial_position_m, bottom_boundary.origin_m),
    ) > f64::EPSILON
    {
        bottom_step = -dot(
            subtract(initial_position_m, bottom_boundary.origin_m),
            bottom_boundary.normal,
        ) / dot(unit_tangent, bottom_boundary.normal);
    }

    let top_interval = top.range_interval(top_segment);
    let bottom_interval = bottom.range_interval(bottom_segment);
    let mut range_interval = [
        top_interval[0].max(bottom_interval[0]),
        top_interval[1].min(bottom_interval[1]),
    ];
    if sound_speed.is_range_dependent() {
        range_interval[0] = range_interval[0].max(
            sound_speed
                .range_interface(initial_segments.range)
                .expect("range-dependent model"),
        );
        range_interval[1] = range_interval[1].min(
            sound_speed
                .range_interface(initial_segments.range + 1)
                .expect("range-dependent model"),
        );
    }
    let mut segment_step = f64::MAX;
    if unit_tangent[0].abs() > f64::EPSILON {
        if trial_position_m[0] < range_interval[0] {
            segment_step = -(initial_position_m[0] - range_interval[0]) / unit_tangent[0];
        } else if trial_position_m[0] > range_interval[1] {
            segment_step = -(initial_position_m[0] - range_interval[1]) / unit_tangent[0];
        }
    }

    let range_box_step = if trial_position_m[0].abs() > limits.max_range_m {
        (limits.max_range_m - initial_position_m[0].abs()) / unit_tangent[0].abs()
    } else {
        f64::MAX
    };
    let depth_box_step = if trial_position_m[1].abs() > limits.max_depth_m {
        (limits.max_depth_m - initial_position_m[1].abs()) / unit_tangent[1].abs()
    } else {
        f64::MAX
    };

    *step_m = step_m
        .min(interface_step)
        .min(top_step)
        .min(bottom_step)
        .min(segment_step)
        .min(range_box_step)
        .min(depth_box_step);
    if *step_m < 1.0e-4 * limits.base_step_m {
        *step_m = 1.0e-4 * limits.base_step_m;
        *consecutive_small_steps += 1;
    } else {
        *consecutive_small_steps = 0;
    }
}

fn state_is_finite(state: RayState) -> bool {
    state.position_m.iter().all(|value| value.is_finite())
        && state.tangent.iter().all(|value| value.is_finite())
        && state.p.iter().all(|value| value.is_finite())
        && state.q.iter().all(|value| value.is_finite())
        && state.speed_mps.is_finite()
        && state.travel_time_s.re.is_finite()
        && state.travel_time_s.im.is_finite()
        && state.amplitude.is_finite()
        && state.phase_radians.is_finite()
}

mod boundary;
mod influence;
mod integrator;
mod reflection;
mod ssp;

use crate::Case;
use crate::diagnostic::{Diagnostic, DiagnosticReport, SourceLocation};
use crate::model::{BeamFamily, BoundaryCondition, ReceiverGrid, RunKind, SourceBeamPattern};
pub use crate::result::*;

use boundary::{BoundaryGeometry, BoundarySide};
use influence::{
    Effects, InfluenceCounts, InfluenceLimits, InfluenceTarget, Recorder, cerveny_cartesian,
    cerveny_ray_centered, geo_gaussian_cartesian, geo_hat_cartesian, geo_hat_ray_centered,
    scale_arrivals, scale_pressure, simple_gaussian,
};
use integrator::{RayState, StepLimits, step_2d};
use num_complex::{Complex32, Complex64};
use reflection::reflect_2d;
use ssp::{SegmentState, SoundSpeedModel};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

/// Rays evaluated ahead per thread before the calling thread applies them in order.
const RAYS_PER_THREAD: usize = 4;
/// Recorded ray products held at once; a batch ends when it reaches this size.
const BATCH_BYTES: usize = 256 << 20;
/// Influence a worker records for one ray; a larger ray is given up and applied
/// by the calling thread instead.
const RAY_RECORD_BYTES: usize = 32 << 20;

/// Resource limits for deterministic simulation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SimulationLimits {
    pub max_rays: usize,
    pub max_steps_per_ray: usize,
    pub max_total_ray_points: usize,
    pub max_arrivals_per_receiver: usize,
    pub max_total_arrivals: usize,
    pub max_eigenrays: usize,
    pub max_total_eigenray_points: usize,
    pub max_field_cells: usize,
}

impl Default for SimulationLimits {
    fn default() -> Self {
        Self {
            max_rays: 1_000_000,
            max_steps_per_ray: 100_000,
            max_total_ray_points: 20_000_000,
            max_arrivals_per_receiver: 20_000_000,
            max_total_arrivals: 20_000_000,
            max_eigenrays: 1_000_000,
            max_total_eigenray_points: 20_000_000,
            max_field_cells: 20_000_000,
        }
    }
}

/// Runs a loaded case using the deterministic compatibility path.
///
/// Runs ray traces, eigenrays, arrivals, or coherent/semi-coherent/incoherent
/// pressure fields with the supported two-dimensional influence models.
/// Unsupported run-kind and beam-family combinations return a structured
/// diagnostic.
///
/// # Errors
///
/// Returns diagnostics for unsupported modes, exceeded resource limits, or
/// non-finite and otherwise invalid numerical states.
pub fn run(case: &Case, limits: SimulationLimits) -> Result<SimulationResult, DiagnosticReport> {
    let result = crate::cancellation::check()
        .map_err(|message| resource_report(case, message))
        .and_then(|()| run_inner(case, limits, RAY_RECORD_BYTES));
    if crate::cancellation::cancelled() {
        return Err(report(Diagnostic::error(
            "BH0304",
            "simulation cancelled",
            "execution",
            SourceLocation::file(&case.environment.source_path),
        )));
    }
    result
}

#[allow(clippy::cast_precision_loss, clippy::too_many_lines)]
fn run_inner(
    case: &Case,
    limits: SimulationLimits,
    ray_record_bytes: usize,
) -> Result<SimulationResult, DiagnosticReport> {
    let influence_supported = matches!(
        case.environment.run.beam_family,
        Some(
            BeamFamily::GeometricHatCartesian
                | BeamFamily::GeometricHatRayCentered
                | BeamFamily::GeometricGaussianCartesian
        )
    ) || (case.environment.run.kind == RunKind::Eigenrays
        && case.environment.run.beam_family == Some(BeamFamily::SimpleGaussian));
    let field_run = matches!(
        case.environment.run.kind,
        RunKind::Coherent | RunKind::SemiCoherent | RunKind::Incoherent
    );
    let field_influence_supported = matches!(
        case.environment.run.beam_family,
        Some(
            BeamFamily::GeometricHatCartesian
                | BeamFamily::GeometricHatRayCentered
                | BeamFamily::GeometricGaussianCartesian
                | BeamFamily::SimpleGaussian
                | BeamFamily::CervenyCartesian
                | BeamFamily::CervenyRayCentered
        )
    );
    if (matches!(
        case.environment.run.kind,
        RunKind::Eigenrays | RunKind::Arrivals
    ) && !influence_supported)
        || (field_run && !field_influence_supported)
    {
        return Err(report(Diagnostic::error(
            "BH0301",
            "the selected influence model is not yet supported for this run kind",
            "run.beam_family",
            SourceLocation::file(&case.environment.source_path),
        )));
    }
    let ray_count = selected_angles(case).len() * case.environment.positions.source_depths_m.len();
    let requested_field_cells = if field_run {
        receiver_count(case)
            .checked_mul(case.environment.positions.source_depths_m.len())
            .ok_or_else(|| resource_report(case, "field cell count overflowed"))?
    } else {
        0
    };
    if requested_field_cells > limits.max_field_cells {
        return Err(resource_report(
            case,
            &format!(
                "simulation requests {requested_field_cells} field cells; limit is {}",
                limits.max_field_cells
            ),
        ));
    }
    if ray_count > limits.max_rays {
        return Err(report(Diagnostic::error(
            "BH0303",
            format!(
                "simulation requests {ray_count} rays; limit is {}",
                limits.max_rays
            ),
            "simulation_limits.max_rays",
            SourceLocation::file(&case.environment.source_path),
        )));
    }
    if limits.max_steps_per_ray < 2
        || limits.max_total_ray_points < ray_count
        || (case.environment.run.kind == RunKind::Arrivals
            && (limits.max_arrivals_per_receiver == 0 || limits.max_total_arrivals == 0))
        || (case.environment.run.kind == RunKind::Eigenrays
            && (limits.max_eigenrays == 0 || limits.max_total_eigenray_points == 0))
    {
        return Err(report(Diagnostic::error(
            "BH0303",
            "simulation resource limits are too small",
            "simulation_limits",
            SourceLocation::file(&case.environment.source_path),
        )));
    }
    if matches!(
        case.environment.top_boundary.condition,
        BoundaryCondition::WriteReflectionCoefficient
    ) || matches!(
        case.environment.bottom_boundary.condition,
        BoundaryCondition::WriteReflectionCoefficient
    ) {
        return Err(report(Diagnostic::error(
            "BH0301",
            "W boundary reflection-table generation is not supported",
            "boundary.condition",
            SourceLocation::file(&case.environment.source_path),
        )));
    }

    let sound_speed = SoundSpeedModel::new(case).map_err(report)?;
    let boundaries = BoundaryGeometry::new(case);
    let step_m = if case.environment.trace.step_m == 0.0 {
        (case.environment.sound_speed.bottom_depth_m - case.environment.sound_speed.top_depth_m)
            / 10.0
    } else {
        case.environment.trace.step_m
    };
    let step_limits = StepLimits {
        base_step_m: step_m,
        max_range_m: case.environment.trace.max_range_m,
        max_depth_m: case.environment.trace.max_depth_m,
    };

    let receiver_count = receiver_count(case);
    let reference_max_arrivals = (limits.max_total_arrivals / receiver_count.max(1)).max(10);
    let influence_limits = InfluenceLimits {
        max_arrivals_per_receiver: limits.max_arrivals_per_receiver.min(reference_max_arrivals),
        max_total_arrivals: limits.max_total_arrivals,
        max_eigenrays: limits.max_eigenrays,
        max_eigenray_points: limits.max_total_eigenray_points,
    };
    let all_angles = &case.environment.trace.launch_angles_degrees;
    let angular_spacing_radians = if all_angles.len() == 1 {
        0.0
    } else {
        ((all_angles[all_angles.len() - 1] - all_angles[0]) / (all_angles.len() - 1) as f64)
            .to_radians()
    };
    let mut counts = InfluenceCounts {
        arrivals: 0,
        eigenrays: 0,
        eigenray_points: 0,
    };
    let mut total_points = 0_usize;
    let mut sources = Vec::new();
    let mut arrival_sources = Vec::new();
    let mut eigenray_sources = Vec::new();
    let mut field_sources = Vec::new();
    let inputs = RayInputs {
        case,
        sound_speed: &sound_speed,
        boundaries: &boundaries,
        step_limits,
        limits,
        angular_spacing_radians,
        ray_record_bytes,
        buffers: Mutex::new(Vec::new()),
        oversized: AtomicBool::new(false),
    };
    // Workers trace and evaluate a batch of rays; this thread then applies them in
    // launch order, so every sum, merge, limit and error is that of one thread.
    let threads = crate::parallelism::threads();
    let batch_len = match threads {
        1 => 1,
        threads => threads.saturating_mul(RAYS_PER_THREAD),
    };
    let angles = selected_angles(case);
    for &source_depth_m in &case.environment.positions.source_depths_m {
        let mut rays = Vec::with_capacity(angles.len());
        let mut arrival_receivers = if case.environment.run.kind == RunKind::Arrivals {
            make_arrival_receivers(case)
        } else {
            Vec::new()
        };
        let mut eigenray_receivers = if case.environment.run.kind == RunKind::Eigenrays {
            make_eigenray_receivers(case)
        } else {
            Vec::new()
        };
        let mut pressure = if field_run {
            vec![Complex32::new(0.0, 0.0); receiver_count]
        } else {
            Vec::new()
        };
        let mut source_speed_mps = 0.0;
        let mut start = 0;
        while start < angles.len() {
            let batch = &angles[start..angles.len().min(start.saturating_add(batch_len))];
            let points_before = total_points;
            let evaluated = crate::parallelism::map_prefix(
                batch.len(),
                BATCH_BYTES,
                |ray: &Result<EvaluatedRay, _>| ray.as_ref().map_or(0, EvaluatedRay::bytes),
                |index| {
                    evaluate_ray(
                        &inputs,
                        source_depth_m,
                        batch[index],
                        points_before,
                        threads > 1,
                    )
                },
            );
            start += evaluated.len();
            for (&launch_angle_degrees, ray) in batch.iter().zip(evaluated) {
                crate::cancellation::check().map_err(|m| resource_report(case, m))?;
                let ray = ray.map_err(|message| {
                    report(Diagnostic::error(
                        "BH0302",
                        format!(
                            "ray at source depth {source_depth_m} m and launch angle {launch_angle_degrees}° failed: {message}"
                        ),
                        "ray_trace",
                        SourceLocation::file(&case.environment.source_path),
                    ))
                })?;
                source_speed_mps = ray.source_speed_mps;
                total_points = total_points
                    .checked_add(ray.point_count)
                    .ok_or_else(|| resource_report(case, "total ray-point count overflowed"))?;
                if total_points > limits.max_total_ray_points {
                    return Err(resource_report(
                        case,
                        &format!(
                            "simulation traced more than {} ray points",
                            limits.max_total_ray_points
                        ),
                    ));
                }
                match ray.product {
                    RayProduct::OverLimit => unreachable!("the ray-point check rejects this ray"),
                    RayProduct::Trajectory(trajectory) => rays.push(trajectory),
                    RayProduct::Influence { states, recorded } => {
                        let mut target = match case.environment.run.kind {
                            RunKind::Eigenrays => {
                                InfluenceTarget::Eigenrays(&mut eigenray_receivers)
                            }
                            RunKind::Arrivals => InfluenceTarget::Arrivals(&mut arrival_receivers),
                            _ => InfluenceTarget::Field(&mut pressure),
                        };
                        let outcome = match recorded {
                            Some((mut effects, failure)) => {
                                let applied = influence::apply(
                                    &mut effects,
                                    &mut target,
                                    &states,
                                    case.environment.frequency_hz,
                                    &influence_limits,
                                    &mut counts,
                                );
                                inputs.recycle(effects);
                                applied.and_then(|()| failure.map_or(Ok(()), Err))
                            }
                            // Apply each effect as it is found, as a single thread does.
                            None => record_influence(
                                case,
                                &sound_speed,
                                &states,
                                launch_angle_degrees.to_radians(),
                                angular_spacing_radians,
                                &mut Recorder::Apply {
                                    target: &mut target,
                                    coherent: case.environment.run.kind == RunKind::Coherent,
                                    states: &states,
                                    limits: &influence_limits,
                                    counts: &mut counts,
                                },
                            ),
                        };
                        outcome.map_err(|message| resource_report(case, message))?;
                    }
                }
            }
        }
        match case.environment.run.kind {
            RunKind::Rays => sources.push(SourceRaySet {
                source_depth_m,
                rays,
            }),
            RunKind::Eigenrays => eigenray_sources.push(SourceEigenrays {
                source_depth_m,
                receivers: eigenray_receivers,
            }),
            RunKind::Arrivals => {
                scale_arrivals(case, &mut arrival_receivers);
                arrival_sources.push(SourceArrivals {
                    source_depth_m,
                    receivers: arrival_receivers,
                });
            }
            RunKind::Coherent | RunKind::SemiCoherent | RunKind::Incoherent => {
                scale_pressure(
                    case,
                    angular_spacing_radians,
                    source_speed_mps,
                    &mut pressure,
                );
                if pressure
                    .iter()
                    .any(|value| !value.re.is_finite() || !value.im.is_finite())
                {
                    return Err(report(Diagnostic::error(
                        "BH0302",
                        "pressure field scaling produced a non-finite value",
                        "pressure_field",
                        SourceLocation::file(&case.environment.source_path),
                    )));
                }
                field_sources.push(SourceField {
                    source_depth_m,
                    samples: receiver_coordinates(case)
                        .zip(pressure)
                        .map(|((range_m, depth_m), pressure)| FieldSample {
                            range_m,
                            depth_m,
                            pressure,
                        })
                        .collect(),
                });
            }
        }
    }
    Ok(SimulationResult {
        title: case.environment.title.clone(),
        frequency_hz: case.environment.frequency_hz,
        legacy_run_options: case.environment.run.legacy.clone(),
        sources,
        arrival_sources,
        eigenray_sources,
        field_sources,
    })
}

fn selected_angles(case: &Case) -> &[f64] {
    match case.environment.trace.selected_launch_angle {
        Some(index) => &case.environment.trace.launch_angles_degrees[index - 1..index],
        None => &case.environment.trace.launch_angles_degrees,
    }
}

/// Shared inputs of every ray of a run.
struct RayInputs<'a> {
    case: &'a Case,
    sound_speed: &'a SoundSpeedModel,
    boundaries: &'a BoundaryGeometry,
    step_limits: StepLimits,
    limits: SimulationLimits,
    angular_spacing_radians: f64,
    ray_record_bytes: usize,
    /// Applied, empty effect buffers. Reusing them keeps a run from allocating
    /// a fresh influence-sized buffer per ray, which some allocators retain.
    buffers: Mutex<Vec<Effects>>,
    /// Set once a ray outgrows `ray_record_bytes`: the calling thread then
    /// applies every later ray itself instead of evaluating it twice.
    oversized: AtomicBool,
}

impl RayInputs<'_> {
    fn effects(&self) -> Effects {
        let recycled = self.buffers.lock().map(|mut buffers| buffers.pop());
        recycled
            .ok()
            .flatten()
            .unwrap_or_else(|| Effects::new(self.case.environment.run.kind))
    }

    fn recycle(&self, effects: Effects) {
        if let Ok(mut buffers) = self.buffers.lock() {
            buffers.push(effects);
        }
    }
}

/// One traced ray and what it contributes, evaluated independently of other rays.
struct EvaluatedRay {
    source_speed_mps: f64,
    point_count: usize,
    product: RayProduct,
}

impl EvaluatedRay {
    fn bytes(&self) -> usize {
        match &self.product {
            RayProduct::OverLimit => 0,
            RayProduct::Trajectory(trajectory) => {
                trajectory.points.len() * std::mem::size_of::<RayPoint>()
            }
            RayProduct::Influence { states, recorded } => {
                states.len() * std::mem::size_of::<RayState>()
                    + recorded.as_ref().map_or(0, |(effects, _)| effects.bytes())
            }
        }
    }
}

enum RayProduct {
    /// Exceeds the ray-point limit whatever the rays before it hold, so nothing
    /// more is evaluated.
    OverLimit,
    Trajectory(RayTrajectory),
    Influence {
        /// Kept for eigenrays, whose hits copy a prefix of the ray, and for
        /// rays the calling thread evaluates.
        states: Vec<RayState>,
        /// The recorded effects and the influence error raised after them, or
        /// `None` when the calling thread applies the ray as it evaluates it.
        recorded: Option<(Effects, Option<&'static str>)>,
    },
}

/// Trace one ray and, when `record`, record its influence without applying it.
/// `points_before` is the ray-point total of the rays before this batch.
fn evaluate_ray(
    inputs: &RayInputs<'_>,
    source_depth_m: f64,
    launch_angle_degrees: f64,
    points_before: usize,
    record: bool,
) -> Result<EvaluatedRay, &'static str> {
    let case = inputs.case;
    let traced = trace_ray(
        case,
        inputs.sound_speed,
        inputs.boundaries,
        inputs.step_limits,
        source_depth_m,
        launch_angle_degrees,
        inputs.limits,
    )?;
    let kind = case.environment.run.kind;
    let source_speed_mps = traced.states[0].speed_mps;
    let point_count = traced.states.len();
    let product = if points_before.saturating_add(point_count) > inputs.limits.max_total_ray_points
    {
        RayProduct::OverLimit
    } else if kind == RunKind::Rays {
        RayProduct::Trajectory(trajectory_from_states(
            launch_angle_degrees,
            &traced.states,
            traced.termination,
        ))
    } else {
        let recorded = if record && !inputs.oversized.load(Ordering::Relaxed) {
            record_alone(inputs, &traced.states, launch_angle_degrees)
        } else {
            None
        };
        RayProduct::Influence {
            states: if kind == RunKind::Eigenrays || recorded.is_none() {
                traced.states
            } else {
                Vec::new()
            },
            recorded,
        }
    };
    Ok(EvaluatedRay {
        source_speed_mps,
        point_count,
        product,
    })
}

/// Record a ray's influence for later application, or `None` if it outgrows
/// `ray_record_bytes`; the calling thread then applies it, and every later ray.
fn record_alone(
    inputs: &RayInputs<'_>,
    states: &[RayState],
    launch_angle_degrees: f64,
) -> Option<(Effects, Option<&'static str>)> {
    let mut effects = inputs.effects();
    let mut recorder = Recorder::Record {
        effects: &mut effects,
        limit: inputs.ray_record_bytes,
        full: false,
    };
    let failure = record_influence(
        inputs.case,
        inputs.sound_speed,
        states,
        launch_angle_degrees.to_radians(),
        inputs.angular_spacing_radians,
        &mut recorder,
    )
    .err();
    if recorder.is_full() {
        inputs.oversized.store(true, Ordering::Relaxed);
        effects.clear();
        inputs.recycle(effects);
        return None;
    }
    Some((effects, failure))
}

fn record_influence(
    case: &Case,
    sound_speed: &SoundSpeedModel,
    states: &[RayState],
    launch_angle_radians: f64,
    angular_spacing_radians: f64,
    recorder: &mut Recorder<'_, '_>,
) -> Result<(), &'static str> {
    match case.environment.run.beam_family {
        Some(BeamFamily::GeometricHatCartesian) => geo_hat_cartesian(
            case,
            states,
            launch_angle_radians,
            angular_spacing_radians,
            recorder,
        ),
        Some(BeamFamily::GeometricHatRayCentered) => geo_hat_ray_centered(
            case,
            states,
            launch_angle_radians,
            angular_spacing_radians,
            recorder,
        ),
        Some(BeamFamily::GeometricGaussianCartesian) => geo_gaussian_cartesian(
            case,
            states,
            launch_angle_radians,
            angular_spacing_radians,
            recorder,
        ),
        Some(BeamFamily::SimpleGaussian) => simple_gaussian(
            case,
            states,
            launch_angle_radians,
            angular_spacing_radians,
            recorder,
        ),
        Some(BeamFamily::CervenyCartesian) => cerveny_cartesian(
            case,
            sound_speed,
            states,
            launch_angle_radians,
            angular_spacing_radians,
            recorder,
        ),
        Some(BeamFamily::CervenyRayCentered) => cerveny_ray_centered(
            case,
            sound_speed,
            states,
            launch_angle_radians,
            angular_spacing_radians,
            recorder,
        ),
        _ => unreachable!("unsupported beam families were rejected"),
    }
}

fn receiver_count(case: &Case) -> usize {
    match case.environment.run.receiver_grid {
        ReceiverGrid::Rectilinear => {
            case.environment.positions.receiver_ranges_m.len()
                * case.environment.positions.receiver_depths_m.len()
        }
        ReceiverGrid::Irregular => case.environment.positions.receiver_ranges_m.len(),
    }
}

fn make_arrival_receivers(case: &Case) -> Vec<ReceiverArrivals> {
    receiver_coordinates(case)
        .map(|(range_m, depth_m)| ReceiverArrivals {
            range_m,
            depth_m,
            arrivals: Vec::new(),
        })
        .collect()
}

fn make_eigenray_receivers(case: &Case) -> Vec<ReceiverEigenrays> {
    receiver_coordinates(case)
        .map(|(range_m, depth_m)| ReceiverEigenrays {
            range_m,
            depth_m,
            eigenrays: Vec::new(),
        })
        .collect()
}

fn receiver_coordinates(case: &Case) -> impl Iterator<Item = (f64, f64)> + '_ {
    let positions = &case.environment.positions;
    positions
        .receiver_ranges_m
        .iter()
        .enumerate()
        .flat_map(
            move |(range_index, &range_m)| match case.environment.run.receiver_grid {
                ReceiverGrid::Rectilinear => positions
                    .receiver_depths_m
                    .iter()
                    .map(move |&depth_m| (range_m, depth_m))
                    .collect::<Vec<_>>(),
                ReceiverGrid::Irregular => {
                    vec![(range_m, positions.receiver_depths_m[range_index])]
                }
            },
        )
}

struct TracedRay {
    states: Vec<RayState>,
    termination: RayTermination,
}

#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
fn trace_ray(
    case: &Case,
    sound_speed: &SoundSpeedModel,
    boundaries: &BoundaryGeometry,
    step_limits: StepLimits,
    source_depth_m: f64,
    launch_angle_degrees: f64,
    limits: SimulationLimits,
) -> Result<TracedRay, &'static str> {
    let mut sound_segments = SegmentState::default();
    let source_position_m = [0.0, source_depth_m];
    let source_sample = sound_speed.evaluate(source_position_m, &mut sound_segments)?;
    let launch_angle_radians = launch_angle_degrees.to_radians();
    let mut amplitude = source_amplitude(case.source_beam_pattern.as_ref(), launch_angle_degrees);
    if case.environment.run.kind == RunKind::SemiCoherent {
        let angular_frequency = 2.0 * std::f64::consts::PI * case.environment.frequency_hz;
        // The reference evaluates SQRT(2.0) in default (single) precision.
        amplitude *= f64::from(2.0_f32.sqrt())
            * (angular_frequency / source_sample.speed_mps
                * source_depth_m
                * launch_angle_radians.sin())
            .sin()
            .abs();
    }
    // Preserve reference RG initialization: p/q are not exported for ray runs,
    // but still participate in non-finite-state rejection. JSON round-trip ray
    // outcomes are characterized in tests/ray_solver.rs; that is not proof this
    // legacy branch is safe to remove for all inputs.
    let geometric_q = case.environment.run.beam_family == Some(BeamFamily::GeometricHatCartesian)
        || case
            .environment
            .run
            .legacy
            .chars()
            .nth(1)
            .is_some_and(|option| option == 'G');
    let initial = RayState {
        position_m: source_position_m,
        tangent: [
            launch_angle_radians.cos() / source_sample.speed_mps,
            launch_angle_radians.sin() / source_sample.speed_mps,
        ],
        p: [1.0, 0.0],
        q: if geometric_q { [0.0, 0.0] } else { [0.0, 1.0] },
        speed_mps: source_sample.speed_mps,
        travel_time_s: Complex64::new(0.0, 0.0),
        amplitude,
        phase_radians: 0.0,
        top_bounces: 0,
        bottom_bounces: 0,
    };
    let mut states = Vec::with_capacity(limits.max_steps_per_ray.min(4096));
    states.push(initial);
    let mut top_segment = boundaries.top.segment_for_range(0.0);
    let mut bottom_segment = boundaries.bottom.segment_for_range(0.0);
    let mut beginning_top_distance = boundaries
        .top
        .signed_inside_distance(source_position_m, top_segment);
    let mut beginning_bottom_distance = boundaries
        .bottom
        .signed_inside_distance(source_position_m, bottom_segment);
    if beginning_top_distance <= 0.0 || beginning_bottom_distance <= 0.0 {
        return Ok(TracedRay {
            states,
            termination: RayTermination::SourceOutsideBoundaries,
        });
    }

    let mut consecutive_small_steps = 0_usize;
    let mut termination = RayTermination::StepLimit;
    while states.len() < limits.max_steps_per_ray {
        if states.len() % 256 == 0 {
            crate::cancellation::check()?;
        }
        let incident = *states.last().expect("initial state is present");
        let stepped = step_2d(
            incident,
            sound_speed,
            &mut sound_segments,
            &boundaries.top,
            top_segment,
            &boundaries.bottom,
            bottom_segment,
            step_limits,
            &mut consecutive_small_steps,
        )?;
        states.push(stepped);

        let top_interval = boundaries.top.range_interval(top_segment);
        if stepped.position_m[0] < top_interval[0] || stepped.position_m[0] > top_interval[1] {
            top_segment = boundaries.top.segment_for_range(stepped.position_m[0]);
        }
        let bottom_interval = boundaries.bottom.range_interval(bottom_segment);
        if stepped.position_m[0] < bottom_interval[0] || stepped.position_m[0] > bottom_interval[1]
        {
            bottom_segment = boundaries.bottom.segment_for_range(stepped.position_m[0]);
        }

        let mut ending_top_distance = boundaries
            .top
            .signed_inside_distance(stepped.position_m, top_segment);
        let mut ending_bottom_distance = boundaries
            .bottom
            .signed_inside_distance(stepped.position_m, bottom_segment);

        if beginning_top_distance > 0.0 && ending_top_distance <= 0.0 {
            if states.len() >= limits.max_steps_per_ray {
                break;
            }
            let (tangent, normal) = boundaries
                .top
                .reflection_frame(stepped.position_m, top_segment);
            let reflected = reflect_2d(
                stepped,
                BoundarySide::Top,
                boundaries.top.segment(top_segment),
                tangent,
                normal,
                &case.environment.top_boundary.condition,
                case.top_reflection.as_ref(),
                None,
                &case.environment,
                sound_speed,
                &mut sound_segments,
                case.environment.run.beam_shift,
            )?;
            states.push(reflected);
            ending_top_distance = boundaries
                .top
                .signed_inside_distance(reflected.position_m, top_segment);
            ending_bottom_distance = boundaries
                .bottom
                .signed_inside_distance(reflected.position_m, bottom_segment);
        } else if beginning_bottom_distance > 0.0 && ending_bottom_distance <= 0.0 {
            if states.len() >= limits.max_steps_per_ray {
                break;
            }
            let (tangent, normal) = boundaries
                .bottom
                .reflection_frame(stepped.position_m, bottom_segment);
            let reflected = reflect_2d(
                stepped,
                BoundarySide::Bottom,
                boundaries.bottom.segment(bottom_segment),
                tangent,
                normal,
                &case.environment.bottom_boundary.condition,
                case.bottom_reflection.as_ref(),
                case.internal_reflection.as_ref(),
                &case.environment,
                sound_speed,
                &mut sound_segments,
                case.environment.run.beam_shift,
            )?;
            states.push(reflected);
            ending_top_distance = boundaries
                .top
                .signed_inside_distance(reflected.position_m, top_segment);
            ending_bottom_distance = boundaries
                .bottom
                .signed_inside_distance(reflected.position_m, bottom_segment);
        }

        let current = *states.last().expect("step state is present");
        if current.position_m[0].abs() > step_limits.max_range_m
            || current.position_m[1].abs() > step_limits.max_depth_m
        {
            termination = RayTermination::ExitedTraceBox;
            break;
        }
        if current.amplitude < 0.005 {
            termination = RayTermination::LostEnergy;
            break;
        }
        if (beginning_top_distance < 0.0 && ending_top_distance < 0.0)
            || (beginning_bottom_distance < 0.0 && ending_bottom_distance < 0.0)
        {
            termination = RayTermination::EscapedBoundary;
            break;
        }
        beginning_top_distance = ending_top_distance;
        beginning_bottom_distance = ending_bottom_distance;
    }

    Ok(TracedRay {
        states,
        termination,
    })
}

fn source_amplitude(pattern: Option<&SourceBeamPattern>, angle_degrees: f64) -> f64 {
    let Some(pattern) = pattern else {
        return 1.0;
    };
    let points = &pattern.points;
    let insertion = points.partition_point(|point| point.angle_degrees < angle_degrees);
    let left = insertion.saturating_sub(1).min(points.len() - 2);
    let fraction = (angle_degrees - points[left].angle_degrees)
        / (points[left + 1].angle_degrees - points[left].angle_degrees);
    (1.0 - fraction) * points[left].amplitude + fraction * points[left + 1].amplitude
}

fn trajectory_from_states(
    launch_angle_degrees: f64,
    states: &[RayState],
    termination: RayTermination,
) -> RayTrajectory {
    let final_state = *states.last().expect("ray has an initial state");
    RayTrajectory {
        launch_angle_degrees,
        points: states
            .iter()
            .map(|state| RayPoint {
                range_m: state.position_m[0],
                depth_m: state.position_m[1],
                travel_time_s: state.travel_time_s.re,
                attenuation_time_s: state.travel_time_s.im,
                amplitude: state.amplitude,
                phase_radians: state.phase_radians,
            })
            .collect(),
        top_bounces: final_state.top_bounces,
        bottom_bounces: final_state.bottom_bounces,
        termination,
    }
}

fn resource_report(case: &Case, message: &str) -> DiagnosticReport {
    report(Diagnostic::error(
        "BH0303",
        message,
        "simulation_limits",
        SourceLocation::file(&case.environment.source_path),
    ))
}

fn report(diagnostic: Diagnostic) -> DiagnosticReport {
    DiagnosticReport::from_diagnostic(diagnostic)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::num::NonZeroUsize;
    use std::path::{Path, PathBuf};

    fn fixtures() -> Vec<(PathBuf, Case)> {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
        let mut paths: Vec<PathBuf> = [root.clone(), root.join("golden")]
            .iter()
            .flat_map(|directory| std::fs::read_dir(directory).unwrap())
            .map(|entry| entry.unwrap().path())
            .filter(|path| path.extension().is_some_and(|extension| extension == "env"))
            .collect();
        paths.sort();
        paths
            .into_iter()
            .map(|path| {
                let case = crate::legacy::load_case(&path).unwrap().value;
                (path, case)
            })
            .collect()
    }

    /// Small ray records make workers give up rays for the calling thread to
    /// apply; outcomes, failures included, must not change.
    #[test]
    fn record_sizes_never_change_outcomes() {
        let default = SimulationLimits::default();
        let points = SimulationLimits {
            max_total_ray_points: 4_000,
            ..default
        };
        let arrivals = SimulationLimits {
            max_total_arrivals: 30,
            ..default
        };
        let eigenrays = SimulationLimits {
            max_eigenrays: 3,
            ..default
        };
        for (path, case) in fixtures() {
            let mut variants = vec![default, points];
            match case.environment.run.kind {
                RunKind::Arrivals => variants.push(arrivals),
                RunKind::Eigenrays => variants.push(eigenrays),
                _ => {}
            }
            for limits in variants {
                let expected = format!("{:?}", run_inner(&case, limits, RAY_RECORD_BYTES));
                for ray_record_bytes in [1, 4096] {
                    for threads in [1, 4] {
                        let actual = crate::parallelism::with_threads(
                            NonZeroUsize::new(threads).unwrap(),
                            || format!("{:?}", run_inner(&case, limits, ray_record_bytes)),
                        );
                        assert!(
                            actual == expected,
                            "{} differs with {threads} threads, {ray_record_bytes}-byte ray records and {limits:?}",
                            path.display(),
                        );
                    }
                }
            }
        }
    }
}

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
    InfluenceCounts, InfluenceLimits, InfluenceTarget, cerveny_cartesian, cerveny_ray_centered,
    geo_gaussian_cartesian, geo_hat_cartesian, geo_hat_ray_centered, scale_arrivals,
    scale_pressure, simple_gaussian,
};
use integrator::{RayState, StepLimits, step_2d};
use num_complex::{Complex32, Complex64};
use reflection::reflect_2d;
use ssp::{SegmentState, SoundSpeedModel};

/// Resource limits for deterministic single-threaded simulation.
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
#[allow(clippy::cast_precision_loss, clippy::too_many_lines)]
pub fn run(case: &Case, limits: SimulationLimits) -> Result<SimulationResult, DiagnosticReport> {
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
            "run_options.beam_family",
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
    for &source_depth_m in &case.environment.positions.source_depths_m {
        let mut rays = Vec::with_capacity(selected_angles(case).len());
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
        for &launch_angle_degrees in selected_angles(case) {
            let traced = trace_ray(
                case,
                &sound_speed,
                &boundaries,
                step_limits,
                source_depth_m,
                launch_angle_degrees,
                limits,
            )
            .map_err(|message| {
                report(Diagnostic::error(
                    "BH0302",
                    format!(
                        "ray at source depth {source_depth_m} m and launch angle {launch_angle_degrees}° failed: {message}"
                    ),
                    "ray_trace",
                    SourceLocation::file(&case.environment.source_path),
                ))
            })?;
            source_speed_mps = traced.states[0].speed_mps;
            total_points = total_points
                .checked_add(traced.states.len())
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

            match case.environment.run.kind {
                RunKind::Rays => rays.push(trajectory_from_states(
                    launch_angle_degrees,
                    &traced.states,
                    traced.termination,
                )),
                RunKind::Eigenrays => {
                    let mut target = InfluenceTarget::Eigenrays(&mut eigenray_receivers);
                    apply_influence(
                        case,
                        &sound_speed,
                        &traced.states,
                        launch_angle_degrees.to_radians(),
                        angular_spacing_radians,
                        &mut target,
                        &influence_limits,
                        &mut counts,
                    )
                    .map_err(|message| resource_report(case, message))?;
                }
                RunKind::Arrivals => {
                    let mut target = InfluenceTarget::Arrivals(&mut arrival_receivers);
                    apply_influence(
                        case,
                        &sound_speed,
                        &traced.states,
                        launch_angle_degrees.to_radians(),
                        angular_spacing_radians,
                        &mut target,
                        &influence_limits,
                        &mut counts,
                    )
                    .map_err(|message| resource_report(case, message))?;
                }
                RunKind::Coherent | RunKind::SemiCoherent | RunKind::Incoherent => {
                    let mut target = InfluenceTarget::Field {
                        pressure: &mut pressure,
                        coherent: case.environment.run.kind == RunKind::Coherent,
                    };
                    apply_influence(
                        case,
                        &sound_speed,
                        &traced.states,
                        launch_angle_degrees.to_radians(),
                        angular_spacing_radians,
                        &mut target,
                        &influence_limits,
                        &mut counts,
                    )
                    .map_err(|message| resource_report(case, message))?;
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

#[allow(clippy::too_many_arguments)]
fn apply_influence(
    case: &Case,
    sound_speed: &SoundSpeedModel,
    states: &[RayState],
    launch_angle_radians: f64,
    angular_spacing_radians: f64,
    target: &mut InfluenceTarget<'_>,
    limits: &InfluenceLimits,
    counts: &mut InfluenceCounts,
) -> Result<(), &'static str> {
    match case.environment.run.beam_family {
        Some(BeamFamily::GeometricHatCartesian) => geo_hat_cartesian(
            case,
            states,
            launch_angle_radians,
            angular_spacing_radians,
            target,
            limits,
            counts,
        ),
        Some(BeamFamily::GeometricHatRayCentered) => geo_hat_ray_centered(
            case,
            states,
            launch_angle_radians,
            angular_spacing_radians,
            target,
            limits,
            counts,
        ),
        Some(BeamFamily::GeometricGaussianCartesian) => geo_gaussian_cartesian(
            case,
            states,
            launch_angle_radians,
            angular_spacing_radians,
            target,
            limits,
            counts,
        ),
        Some(BeamFamily::SimpleGaussian) => simple_gaussian(
            case,
            states,
            launch_angle_radians,
            angular_spacing_radians,
            target,
            limits,
            counts,
        ),
        Some(BeamFamily::CervenyCartesian) => cerveny_cartesian(
            case,
            sound_speed,
            states,
            launch_angle_radians,
            angular_spacing_radians,
            target,
        ),
        Some(BeamFamily::CervenyRayCentered) => cerveny_ray_centered(
            case,
            sound_speed,
            states,
            launch_angle_radians,
            angular_spacing_radians,
            target,
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
        amplitude *= 2.0_f64.sqrt()
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

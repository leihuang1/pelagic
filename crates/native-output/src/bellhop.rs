//! BELLHOP 2D RAY, ASCII/binary ARR and SHD serialization of existing results.
#![allow(clippy::cast_possible_truncation)]
use crate::native::{self, count, create, doubles, sequential, singles};
use bellhop::model::{LegacyArrivalEncoding, ReceiverGrid, RunKind};
use bellhop::{Case, result::SimulationResult};
use std::io::{self, Write};
use std::path::Path;

/// Serialize an existing result into owned scratch; never runs the solver.
/// # Errors
/// Returns native representation or filesystem errors.
pub fn write(
    root: &Path,
    stem: &str,
    case: &Case,
    result: &SimulationResult,
) -> Result<(), String> {
    match case.environment.run.kind {
        RunKind::Rays | RunKind::Eigenrays => rays(&root.join(format!("{stem}.ray")), case, result),
        RunKind::Arrivals => {
            if case.environment.run.receiver_grid == ReceiverGrid::Irregular {
                // ARR has no irregular-grid flag. Each paired receiver gets a readable 1x1 ARR,
                // rather than invented off-diagonal zero-arrival receivers or quadratic storage.
                // ponytail: one file/pair can hit the 1 MiB manifest cap; use HDF5 for large paired grids.
                (0..case.environment.positions.receiver_ranges_m.len()).try_for_each(|index| {
                    arrivals(
                        &root.join(format!("{stem}.r{index:06}.arr")),
                        case,
                        result,
                        Some(index),
                    )
                })
            } else {
                arrivals(&root.join(format!("{stem}.arr")), case, result, None)
            }
        }
        _ => {
            let pos = &case.environment.positions;
            native::shd(
                &root.join(format!("{stem}.shd")),
                &result.title,
                result.frequency_hz,
                &pos.source_depths_m,
                &pos.receiver_depths_m,
                &pos.receiver_ranges_m,
                case.environment.run.receiver_grid == ReceiverGrid::Irregular,
                result.field_grid(case)?.pressures().map(|p| (p.re, p.im)),
                u64::MAX,
            )
        }
    }
    .map_err(|e| e.to_string())
}

fn rays(path: &Path, case: &Case, result: &SimulationResult) -> io::Result<()> {
    let mut file = create(path)?;
    let env = &case.environment;
    let title: String = env
        .title
        .chars()
        .take(50)
        .collect::<String>()
        .replace('\'', "''")
        .replace(['\n', '\r'], " ");
    writeln!(
        file,
        "'{title}'\n{:.17e}\n1 1 {}\n{} 1\n{:.17e}\n{:.17e}\n'rz'",
        env.frequency_hz,
        env.positions.source_depths_m.len(),
        env.trace
            .selected_launch_angle
            .map_or(env.trace.launch_angles_degrees.len(), |_| 1),
        env.sound_speed.top_depth_m,
        env.sound_speed.bottom_depth_m
    )?;
    let rays = result.sources.iter().flat_map(|s| &s.rays).chain(
        result
            .eigenray_sources
            .iter()
            .flat_map(|s| &s.receivers)
            .flat_map(|r| &r.eigenrays),
    );
    for ray in rays {
        writeln!(
            file,
            "{:.17e}\n{} {} {}",
            ray.launch_angle_degrees,
            ray.points.len(),
            ray.top_bounces,
            ray.bottom_bounces
        )?;
        for point in &ray.points {
            writeln!(file, "{:.17e} {:.17e}", point.range_m, point.depth_m)?;
        }
    }
    file.flush()
}

#[allow(clippy::cast_precision_loss, clippy::too_many_lines)] // Keep both pinned ARR layouts together; bounce counts are f32 in binary.
fn arrivals(
    path: &Path,
    case: &Case,
    result: &SimulationResult,
    selected: Option<usize>,
) -> io::Result<()> {
    let mut file = create(path)?;
    let pos = &case.environment.positions;
    let binary = case.environment.run.arrival_encoding == Some(LegacyArrivalEncoding::Binary);
    if binary {
        sequential(&mut file, b"'2D'")?;
        sequential(&mut file, &native::single(result.frequency_hz)?)?;
    } else {
        writeln!(file, "'2D'\n{:.17e}", result.frequency_hz)?;
    }
    let depths = selected.map_or(pos.receiver_depths_m.as_slice(), |i| {
        &pos.receiver_depths_m[i..=i]
    });
    let ranges = selected.map_or(pos.receiver_ranges_m.as_slice(), |i| {
        &pos.receiver_ranges_m[i..=i]
    });
    for (values, double) in [
        (pos.source_depths_m.as_slice(), false),
        (depths, false),
        (ranges, true),
    ] {
        if binary {
            let mut record = count(values.len())?.to_vec();
            record.extend(if double {
                doubles(values)
            } else {
                singles(values)?
            });
            sequential(&mut file, &record)?;
        } else {
            write!(file, "{}", values.len())?;
            for &value in values {
                write!(
                    file,
                    " {:.17e}",
                    if double {
                        value
                    } else {
                        f64::from(f32::from_le_bytes(native::single(value)?))
                    }
                )?;
            }
            writeln!(file)?;
        }
    }
    for source in &result.arrival_sources {
        let depth_count = if selected.is_some() {
            1
        } else {
            pos.receiver_depths_m.len()
        };
        let expected = depth_count * pos.receiver_ranges_m.len();
        if source.receivers.len() != expected {
            return Err(io::Error::other("incomplete native arrival grid"));
        }
        let indices = (selected.unwrap_or(0)..).take(if selected.is_some() { 1 } else { expected });
        // ARR readers visit depth, then range; solver/HDF5 receivers visit range, then depth.
        let receivers = indices.map(|i| {
            &source.receivers[if selected.is_some() {
                i
            } else {
                (i % pos.receiver_ranges_m.len()) * depth_count + i / pos.receiver_ranges_m.len()
            }]
        });
        let maximum = receivers
            .clone()
            .map(|r| r.arrivals.len())
            .max()
            .unwrap_or(0);
        if binary {
            sequential(&mut file, &count(maximum)?)?;
        } else {
            writeln!(file, "{maximum}")?;
        }
        for receiver in receivers {
            if binary {
                sequential(&mut file, &count(receiver.arrivals.len())?)?;
            } else {
                writeln!(file, "{}", receiver.arrivals.len())?;
            }
            for arrival in &receiver.arrivals {
                // The solver result already includes line/cylindrical spreading. Do not apply it twice.
                let phase_degrees = arrival.phase_radians.to_degrees();
                let values = [
                    arrival.amplitude,
                    phase_degrees,
                    arrival.travel_time_s,
                    arrival.attenuation_time_s,
                    arrival.source_angle_degrees,
                    arrival.receiver_angle_degrees,
                ];
                if values.iter().any(|v| !v.is_finite()) {
                    return Err(io::Error::other("non-finite native arrival"));
                }
                if binary {
                    let mut record: Vec<_> = values.iter().flat_map(|v| v.to_le_bytes()).collect();
                    record.extend((arrival.top_bounces as f32).to_le_bytes());
                    record.extend((arrival.bottom_bounces as f32).to_le_bytes());
                    sequential(&mut file, &record)?;
                } else {
                    for value in values {
                        write!(file, "{value:.9e} ")?;
                    }
                    writeln!(file, "{} {}", arrival.top_bounces, arrival.bottom_bounces)?;
                }
            }
        }
    }
    file.flush()
}

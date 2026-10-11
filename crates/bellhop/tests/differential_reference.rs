use std::fs;
use std::path::{Path, PathBuf};

use bellhop::legacy::load_case;
use bellhop::model::ReceiverGrid;
use bellhop::solver::{Arrival, RayPoint, RayTrajectory, SimulationLimits, run};

#[derive(Debug)]
struct ReferenceRay {
    launch_angle_degrees: f64,
    top_bounces: u32,
    bottom_bounces: u32,
    points: Vec<[f64; 2]>,
}

#[test]
#[ignore = "requires a pinned external Fortran reference run"]
fn ray_trajectories_match_pinned_linux_reference() {
    let environment = required_path("BELLHOP_DIFFERENTIAL_ENV");
    let reference_path = required_path("BELLHOP_DIFFERENTIAL_RAY");
    let tolerance = std::env::var("BELLHOP_DIFFERENTIAL_POSITION_TOLERANCE_M")
        .map_or(Ok(1.0e-5), |value| value.parse::<f64>())
        .expect("position tolerance must be a number");

    let case = load_case(&environment).unwrap().value;
    let base_step_m = if case.environment.trace.step_m == 0.0 {
        (case.environment.sound_speed.bottom_depth_m - case.environment.sound_speed.top_depth_m)
            / 10.0
    } else {
        case.environment.trace.step_m
    };
    let minimum_step_factor = std::env::var("BELLHOP_DIFFERENTIAL_MINIMUM_STEP_FACTOR")
        .map_or(Ok(4.1), |value| value.parse::<f64>())
        .expect("minimum-step tolerance factor must be a number");
    assert!(minimum_step_factor >= 1.0);
    let minimum_step_tolerance = (minimum_step_factor * 1.0e-4 * base_step_m).max(tolerance);
    let result = run(&case, SimulationLimits::default()).unwrap();
    let actual: Vec<&RayTrajectory> = result
        .sources
        .iter()
        .flat_map(|source| &source.rays)
        .chain(
            result
                .eigenray_sources
                .iter()
                .flat_map(|source| &source.receivers)
                .flat_map(|receiver| &receiver.eigenrays),
        )
        .collect();
    let expected = parse_reference_rays(&reference_path);
    assert_eq!(actual.len(), expected.len(), "ray count differs");

    let mut maximum_error = 0.0_f64;
    let mut maximum_minimum_step_error = 0.0_f64;
    let mut skipped_minimum_steps = 0_usize;
    for (ray_index, (actual, expected)) in actual.iter().zip(&expected).enumerate() {
        assert!(
            (actual.launch_angle_degrees - expected.launch_angle_degrees).abs() < 1.0e-10,
            "ray {ray_index} launch angle differs"
        );
        assert_eq!(
            actual.top_bounces, expected.top_bounces,
            "ray {ray_index} top-bounce count differs"
        );
        assert_eq!(
            actual.bottom_bounces, expected.bottom_bounces,
            "ray {ray_index} bottom-bounce count differs"
        );

        if actual.points.len() == expected.points.len()
            && actual
                .points
                .iter()
                .zip(&expected.points)
                .all(|(actual, &expected)| {
                    coordinate_error(actual.range_m, actual.depth_m, expected) <= tolerance
                })
        {
            for (actual, &expected) in actual.points.iter().zip(&expected.points) {
                maximum_error =
                    maximum_error.max(coordinate_error(actual.range_m, actual.depth_m, expected));
            }
            continue;
        }

        let (branch_error, skipped) = align_compiler_sensitive_steps(
            &actual.points,
            &expected.points,
            minimum_step_tolerance,
        )
        .unwrap_or_else(|| {
            panic!(
                "ray {ray_index} cannot be aligned within the {minimum_step_tolerance:e} m minimum-step tolerance (Rust points={}, Fortran points={})",
                actual.points.len(),
                expected.points.len()
            )
        });
        maximum_minimum_step_error = maximum_minimum_step_error.max(branch_error);
        skipped_minimum_steps += skipped;
    }
    eprintln!(
        "compared {} rays; maximum aligned coordinate error {maximum_error:e} m; maximum compiler-sensitive branch error {maximum_minimum_step_error:e} m; accepted {skipped_minimum_steps} minimum-step differences",
        actual.len()
    );
}

fn align_compiler_sensitive_steps(
    actual: &[RayPoint],
    expected: &[[f64; 2]],
    tolerance: f64,
) -> Option<(f64, usize)> {
    let columns = expected.len() + 1;
    let mut parent = vec![0_u8; (actual.len() + 1).checked_mul(columns)?];
    parent[0] = 4;
    for actual_index in 0..=actual.len() {
        for expected_index in 0..=expected.len() {
            if parent[actual_index * columns + expected_index] == 0 {
                continue;
            }
            if actual_index < actual.len()
                && expected_index < expected.len()
                && coordinate_error(
                    actual[actual_index].range_m,
                    actual[actual_index].depth_m,
                    expected[expected_index],
                ) <= tolerance
            {
                let next = (actual_index + 1) * columns + expected_index + 1;
                if parent[next] == 0 {
                    parent[next] = 1;
                }
            }
            if is_short_actual_step(actual, actual_index, tolerance) {
                let next = (actual_index + 1) * columns + expected_index;
                if parent[next] == 0 {
                    parent[next] = 2;
                }
            }
            if is_short_expected_step(expected, expected_index, tolerance) {
                let next = actual_index * columns + expected_index + 1;
                if parent[next] == 0 {
                    parent[next] = 3;
                }
            }
        }
    }

    let mut actual_index = actual.len();
    let mut expected_index = expected.len();
    if parent[actual_index * columns + expected_index] == 0 {
        return None;
    }
    let mut maximum_error = 0.0_f64;
    let mut skipped = 0;
    while actual_index > 0 || expected_index > 0 {
        match parent[actual_index * columns + expected_index] {
            1 => {
                actual_index -= 1;
                expected_index -= 1;
                maximum_error = maximum_error.max(coordinate_error(
                    actual[actual_index].range_m,
                    actual[actual_index].depth_m,
                    expected[expected_index],
                ));
            }
            2 => {
                actual_index -= 1;
                skipped += 1;
            }
            3 => {
                expected_index -= 1;
                skipped += 1;
            }
            _ => return None,
        }
    }
    Some((maximum_error, skipped))
}

fn is_short_actual_step(points: &[RayPoint], index: usize, tolerance: f64) -> bool {
    index > 0
        && index + 1 < points.len()
        && coordinate_error(
            points[index].range_m,
            points[index].depth_m,
            [points[index - 1].range_m, points[index - 1].depth_m],
        ) <= tolerance
}

fn is_short_expected_step(points: &[[f64; 2]], index: usize, tolerance: f64) -> bool {
    index > 0
        && index + 1 < points.len()
        && coordinate_error(points[index][0], points[index][1], points[index - 1]) <= tolerance
}

fn coordinate_error(range_m: f64, depth_m: f64, expected: [f64; 2]) -> f64 {
    (range_m - expected[0])
        .abs()
        .max((depth_m - expected[1]).abs())
}

fn required_path(variable: &str) -> PathBuf {
    std::env::var_os(variable).map_or_else(
        || panic!("{variable} must name an input file"),
        PathBuf::from,
    )
}

fn parse_reference_rays(path: &Path) -> Vec<ReferenceRay> {
    let source = fs::read_to_string(path).expect("reference .ray file must be readable");
    let mut records = source
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty());
    for description in [
        "title",
        "frequency",
        "source counts",
        "launch-angle counts",
        "top depth",
        "bottom depth",
        "coordinate type",
    ] {
        records
            .next()
            .unwrap_or_else(|| panic!("reference .ray file is missing {description}"));
    }

    let mut rays = Vec::new();
    while let Some(angle_record) = records.next() {
        let launch_angle_degrees = parse_values::<f64>(angle_record, "launch angle")[0];
        let counts_record = records.next().expect("ray point-count record is missing");
        let counts = parse_values::<u32>(counts_record, "ray point counts");
        assert_eq!(counts.len(), 3, "ray count record must have three values");
        let point_count = usize::try_from(counts[0]).expect("point count must fit usize");
        let mut points = Vec::with_capacity(point_count);
        for _ in 0..point_count {
            let point_record = records.next().expect("ray coordinate record is missing");
            let point = parse_values::<f64>(point_record, "ray coordinates");
            assert_eq!(point.len(), 2, "2D ray point must have two coordinates");
            points.push([point[0], point[1]]);
        }
        rays.push(ReferenceRay {
            launch_angle_degrees,
            top_bounces: counts[1],
            bottom_bounces: counts[2],
            points,
        });
    }
    rays
}

#[test]
#[ignore = "requires a pinned external Fortran reference run"]
fn arrivals_match_pinned_linux_reference() {
    let environment = required_path("BELLHOP_DIFFERENTIAL_ENV");
    let reference_path = required_path("BELLHOP_DIFFERENTIAL_ARR");

    let case = load_case(&environment).unwrap().value;
    let result = run(&case, SimulationLimits::default()).unwrap();
    let actual = &result.arrival_sources[0].receivers;
    let expected = parse_reference_arrivals(&reference_path);
    let (depth_count, range_count) = receiver_grid_shape(&case);
    assert_eq!(
        expected.len(),
        depth_count * range_count,
        "reference .arr receiver count"
    );
    assert_eq!(
        actual.len(),
        depth_count * range_count,
        "Rust receiver count"
    );

    let mut maximum_amplitude_error = 0.0_f32;
    let mut maximum_angle_error = 0.0_f32;
    let mut maximum_time_error = 0.0_f32;
    let mut maximum_attenuation_error = 0.0_f32;
    let mut maximum_phase_error = 0.0_f32;
    let mut phase_mismatches = 0_usize;
    let mut field_mismatches = 0_usize;
    let mut first_mismatch = None;
    let mut count_mismatches = 0_usize;
    for range_index in 0..range_count {
        for depth_index in 0..depth_count {
            // The reference writes receivers depth-major (``DO id = 1, Nrd;
            // DO ir = 1, Nr``); Rust is range-major.
            let actual_receiver = &actual[range_index * depth_count + depth_index];
            let expected_receiver = &expected[depth_index * range_count + range_index].arrivals;
            if actual_receiver.arrivals.len() != expected_receiver.len() {
                assert_eq!(
                    actual_receiver
                        .arrivals
                        .len()
                        .abs_diff(expected_receiver.len()),
                    1,
                    "receiver (range {range_index}, depth {depth_index}) arrival-count difference exceeds one"
                );
                count_mismatches += 1;
                continue;
            }
            for (arrival_index, (actual, expected)) in actual_receiver
                .arrivals
                .iter()
                .zip(expected_receiver)
                .enumerate()
            {
                let label = format!(
                    "receiver (range {range_index}, depth {depth_index}) arrival {arrival_index}"
                );
                let result = compare_arrival(
                    actual,
                    expected,
                    &label,
                    &mut maximum_amplitude_error,
                    &mut maximum_angle_error,
                    &mut maximum_time_error,
                    &mut maximum_attenuation_error,
                    &mut maximum_phase_error,
                );
                if result.is_some() {
                    field_mismatches += 1;
                    if result == Some(MismatchKind::Phase) {
                        phase_mismatches += 1;
                    }
                    if first_mismatch.is_none() {
                        first_mismatch = result;
                    }
                }
            }
        }
    }
    assert!(
        count_mismatches <= 4,
        "{count_mismatches} receiver arrival counts differ; expected at most four f64-grid edge differences"
    );
    if let Some(kind) = first_mismatch {
        panic!(
            "{field_mismatches} arrival fields exceed tolerance (first: {kind:?}); \
             maximum amplitude error {maximum_amplitude_error:e}; maximum phase error {maximum_phase_error:e}; \
             maximum angle error {maximum_angle_error:e}; maximum travel-time error {maximum_time_error:e}; \
             maximum attenuation-time error {maximum_attenuation_error:e}"
        );
    }
    eprintln!(
        "compared {} receivers; maximum amplitude error {maximum_amplitude_error:e}; \
         maximum angle error {maximum_angle_error:e}; maximum travel-time error {maximum_time_error:e}; \
         maximum attenuation-time error {maximum_attenuation_error:e}; \
         phase mismatches {phase_mismatches}; arrival-count mismatches {count_mismatches}",
        actual.len()
    );
}

struct ReferenceArrivalReceiver {
    arrivals: Vec<Arrival>,
}

fn parse_reference_arrivals(path: &Path) -> Vec<ReferenceArrivalReceiver> {
    let source = fs::read_to_string(path).expect("reference .arr file must be readable");
    let header: Vec<&str> = source.lines().collect();
    for description in [
        "title",
        "frequency",
        "source depths",
        "receiver depths",
        "receiver ranges",
    ] {
        assert!(header.get(description_index(description)).is_some());
    }
    // Line 6 (0-based) is MAXVAL(NArr), written before the receiver blocks.
    let mut line_index = 6;
    let mut receivers = Vec::new();
    while line_index < header.len() {
        let arrival_count: usize = header[line_index].trim().parse().unwrap_or_else(|_| {
            panic!(
                "reference .arr receiver {:?} arrival count is invalid",
                receivers.len()
            )
        });
        line_index += 1;
        let mut arrivals = Vec::with_capacity(arrival_count);
        for _ in 0..arrival_count {
            let fields: Vec<&str> = header[line_index].split_whitespace().collect();
            assert_eq!(fields.len(), 8, "arrival row must have eight fields");
            arrivals.push(Arrival {
                amplitude: fields[0].parse().unwrap(),
                phase_radians: fields[1].parse::<f32>().unwrap().to_radians(),
                travel_time_s: fields[2].parse().unwrap(),
                attenuation_time_s: fields[3].parse().unwrap(),
                source_angle_degrees: fields[4].parse().unwrap(),
                receiver_angle_degrees: fields[5].parse().unwrap(),
                top_bounces: fields[6].parse().unwrap(),
                bottom_bounces: fields[7].parse().unwrap(),
            });
            line_index += 1;
        }
        receivers.push(ReferenceArrivalReceiver { arrivals });
    }
    receivers
}

fn description_index(description: &str) -> usize {
    match description {
        "title" => 0,
        "frequency" => 1,
        "source depths" => 2,
        "receiver depths" => 3,
        "receiver ranges" => 4,
        _ => 5,
    }
}

#[allow(clippy::too_many_arguments)]
fn compare_arrival(
    actual: &Arrival,
    expected: &Arrival,
    _label: &str,
    maximum_amplitude_error: &mut f32,
    maximum_angle_error: &mut f32,
    maximum_time_error: &mut f32,
    maximum_attenuation_error: &mut f32,
    maximum_phase_error: &mut f32,
) -> Option<MismatchKind> {
    *maximum_amplitude_error = maximum_amplitude_error.max(amplitude_error(actual, expected));
    *maximum_angle_error = maximum_angle_error.max(angle_error(actual, expected));
    *maximum_time_error = maximum_time_error.max(time_error(actual, expected));
    *maximum_attenuation_error = maximum_attenuation_error.max(attenuation_error(actual, expected));
    *maximum_phase_error = maximum_phase_error.max(phase_error(actual, expected));
    // The reference stores the phase as single-precision DEGREES
    // (``SNGL( RadDeg ) * Arr(...)%Phase`` in ArrMod.f90), quantizing to
    // ~2.2e-6 rad at large phases; Rust stores single-precision radians.
    // Values within one quantum of either storage grid can therefore differ
    // by up to ~4.2e-6 rad while representing the same computed phase.
    let phase_tolerance = 5.0e-6;
    // Double-precision receiver grids move samples by less than one former
    // single-precision grid quantum, producing sub-micro-unit amplitude shifts.
    let amplitude_tolerance = 5.0e-7;
    if amplitude_error(actual, expected) <= amplitude_tolerance
        && phase_error(actual, expected) <= phase_tolerance
        && time_error(actual, expected) <= time_error_tolerance(actual, expected)
        && attenuation_error(actual, expected) <= attenuation_tolerance(actual, expected)
        && angle_error(actual, expected) <= angle_tolerance(actual, expected)
        && actual.top_bounces == expected.top_bounces
        && actual.bottom_bounces == expected.bottom_bounces
    {
        return None;
    }
    Some(if phase_error(actual, expected) > phase_tolerance {
        MismatchKind::Phase
    } else {
        MismatchKind::Field
    })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum MismatchKind {
    Field,
    Phase,
}

fn amplitude_error(actual: &Arrival, expected: &Arrival) -> f32 {
    (actual.amplitude - expected.amplitude).abs()
}

fn phase_error(actual: &Arrival, expected: &Arrival) -> f32 {
    (actual.phase_radians - expected.phase_radians).abs()
}

fn time_error(actual: &Arrival, expected: &Arrival) -> f32 {
    (actual.travel_time_s - expected.travel_time_s).abs()
}

fn attenuation_error(actual: &Arrival, expected: &Arrival) -> f32 {
    (actual.attenuation_time_s - expected.attenuation_time_s).abs()
}

fn angle_error(actual: &Arrival, expected: &Arrival) -> f32 {
    (actual.source_angle_degrees - expected.source_angle_degrees)
        .abs()
        .max((actual.receiver_angle_degrees - expected.receiver_angle_degrees).abs())
}

fn angle_tolerance(_actual: &Arrival, _expected: &Arrival) -> f32 {
    5.0e-5
}

fn time_error_tolerance(_actual: &Arrival, _expected: &Arrival) -> f32 {
    5.0e-6
}

fn attenuation_tolerance(_actual: &Arrival, _expected: &Arrival) -> f32 {
    1.0e-7
}

#[test]
#[ignore = "requires a pinned external Fortran reference run"]
fn pressure_fields_match_pinned_linux_reference() {
    let environment = required_path("BELLHOP_DIFFERENTIAL_ENV");
    let reference_path = required_path("BELLHOP_DIFFERENTIAL_SHD");
    let tolerance = std::env::var("BELLHOP_DIFFERENTIAL_PRESSURE_TOLERANCE")
        .map_or(Ok(5.0e-8), |value| value.parse::<f64>())
        .expect("pressure tolerance must be a number");
    let relative_tolerance = std::env::var("BELLHOP_DIFFERENTIAL_PRESSURE_RELATIVE_TOLERANCE")
        .map_or(Ok(0.0), |value| value.parse::<f64>())
        .expect("relative pressure tolerance must be a number");

    let case = load_case(&environment).unwrap().value;
    let result = run(&case, SimulationLimits::default()).unwrap();
    let actual = &result.field_sources[0].samples;
    let (depth_count, range_count) = receiver_grid_shape(&case);
    let expected = read_reference_shd(&reference_path, depth_count, range_count);

    let positions = &case.environment.positions;
    assert_eq!(
        expected.receiver_depths_m.len(),
        positions.receiver_depths_m.len(),
        "reference .shd receiver depth count"
    );
    assert_eq!(
        expected.receiver_ranges_m.len(),
        positions.receiver_ranges_m.len(),
        "reference .shd receiver range count"
    );
    for (actual, expected) in positions
        .receiver_depths_m
        .iter()
        .zip(&expected.receiver_depths_m)
    {
        assert!(
            (*actual - *expected).abs() <= 1.0e-6 * expected.abs().max(1.0),
            "receiver depth differs: Rust {actual}, reference {expected}"
        );
    }
    for (actual, expected) in positions
        .receiver_ranges_m
        .iter()
        .zip(&expected.receiver_ranges_m)
    {
        assert!(
            (actual - *expected).abs() <= 1.0e-6 * expected.abs().max(1.0),
            "receiver range differs: Rust {actual}, reference {expected}"
        );
    }

    let mut maximum_error = 0.0_f64;
    for range_index in 0..range_count {
        for depth_index in 0..depth_count {
            // The reference stores one depth row per record: ``P(iRz, :)``;
            // Rust samples are range-major.
            let actual_sample = &actual[range_index * depth_count + depth_index];
            let expected_sample = expected.rows[depth_index][range_index];
            let error = (f64::from(actual_sample.pressure.re) - f64::from(expected_sample.re))
                .abs()
                .max((f64::from(actual_sample.pressure.im) - f64::from(expected_sample.im)).abs());
            maximum_error = maximum_error.max(error);
            let magnitude = f64::from(expected_sample.re)
                .abs()
                .max(f64::from(expected_sample.im).abs());
            assert!(
                error <= tolerance + relative_tolerance * magnitude,
                "sample (range {range_index}, depth {depth_index}): actual ({}, {}), \
                 expected ({}, {}), error {error:e} exceeds {tolerance:e} + {relative_tolerance:e} * magnitude",
                actual_sample.pressure.re,
                actual_sample.pressure.im,
                expected_sample.re,
                expected_sample.im
            );
        }
    }
    eprintln!(
        "compared {} pressure samples; maximum absolute error {maximum_error:e}",
        actual.len()
    );
}

fn receiver_grid_shape(case: &bellhop::Case) -> (usize, usize) {
    let positions = &case.environment.positions;
    match case.environment.run.receiver_grid {
        ReceiverGrid::Rectilinear => (
            positions.receiver_depths_m.len(),
            positions.receiver_ranges_m.len(),
        ),
        ReceiverGrid::Irregular => (1, positions.receiver_ranges_m.len()),
    }
}

struct ReferenceShd {
    receiver_depths_m: Vec<f64>,
    receiver_ranges_m: Vec<f64>,
    rows: Vec<Vec<num_complex::Complex32>>,
}

#[allow(clippy::cast_sign_loss)]
fn read_reference_shd(path: &Path, expected_depths: usize, expected_ranges: usize) -> ReferenceShd {
    let bytes = fs::read(path).expect("reference .shd file must be readable");
    let record_length_words = read_i32(&bytes, 0) as usize;
    assert!(
        (41..=1_000_000).contains(&record_length_words),
        "implausible LRecl"
    );
    let record_bytes = 4 * record_length_words;
    assert_eq!(
        bytes.len() % record_bytes,
        0,
        "reference .shd is not a whole number of records"
    );
    let record = |index: usize| &bytes[index * record_bytes..(index + 1) * record_bytes];
    let _title = std::str::from_utf8(&record(0)[4..84])
        .expect("shd record 1 title must be UTF-8")
        .trim_end();
    let plot_type = std::str::from_utf8(&record(1)[..10])
        .expect("shd record 2 plot type must be UTF-8")
        .trim_end();
    assert_eq!(plot_type, "rectilin", "reference .shd plot type");
    let counts = record(2);
    let frequency_count = read_i32(counts, 0) as usize;
    let theta_count = read_i32(counts, 4) as usize;
    let x_count = read_i32(counts, 8) as usize;
    let y_count = read_i32(counts, 12) as usize;
    let z_count = read_i32(counts, 16) as usize;
    let shd_depth_count = read_i32(counts, 20) as usize;
    let shd_range_count = read_i32(counts, 24) as usize;
    let frequency_hz = read_f64(counts, 28);
    let _attenuation = read_f64(counts, 36);
    assert_eq!(
        shd_depth_count, expected_depths,
        "reference .shd depth count"
    );
    assert_eq!(
        shd_range_count, expected_ranges,
        "reference .shd range count"
    );
    let record_minimum = 4 * 41;
    assert!(
        record_bytes >= record_minimum
            && record_bytes >= 4 * 2 * frequency_count
            && record_bytes >= 4 * 2 * theta_count
            && record_bytes >= 4 * 2 * x_count
            && record_bytes >= 4 * 2 * y_count
            && record_bytes >= 4 * z_count
            && record_bytes >= 4 * shd_depth_count
            && record_bytes >= 4 * 2 * shd_range_count,
        "reference .shd record is too small for the declared counts"
    );
    let mut record_index = 3;
    let frequencies = read_record_double(&bytes, record_bytes, record_index, frequency_count);
    record_index += 1;
    let _bearings = read_record_double(&bytes, record_bytes, record_index, theta_count);
    record_index += 1;
    let _source_x = read_record_double(&bytes, record_bytes, record_index, x_count);
    record_index += 1;
    let _source_y = read_record_double(&bytes, record_bytes, record_index, y_count);
    record_index += 1;
    // The reference stores source depths and receiver depths single
    // precision but receiver ranges double precision.
    let _source_depths = read_record_single(&bytes, record_bytes, record_index, z_count);
    record_index += 1;
    let receiver_depths_single =
        read_record_single(&bytes, record_bytes, record_index, shd_depth_count);
    record_index += 1;
    let receiver_depths: Vec<f64> = receiver_depths_single
        .iter()
        .map(|&depth| f64::from(depth))
        .collect();
    let receiver_ranges = read_record_double(&bytes, record_bytes, record_index, shd_range_count);
    record_index += 1;
    assert!(
        frequencies
            .iter()
            .any(|&f| (f - frequency_hz).abs() <= 1.0e-9 * frequency_hz.abs().max(1.0)),
        "freqVec mismatch"
    );
    let rows = (0..shd_depth_count)
        .map(|depth_index| {
            let record = record(record_index + depth_index);
            (0..shd_range_count)
                .map(|range_index| {
                    let offset = 8 * range_index;
                    num_complex::Complex32::new(
                        read_f32(record, offset),
                        read_f32(record, offset + 4),
                    )
                })
                .collect()
        })
        .collect();
    assert_eq!(
        record_index + shd_depth_count,
        bytes.len() / record_bytes,
        "reference .shd has trailing records"
    );
    ReferenceShd {
        receiver_depths_m: receiver_depths,
        receiver_ranges_m: receiver_ranges,
        rows,
    }
}

fn read_record_double(bytes: &[u8], record_bytes: usize, index: usize, count: usize) -> Vec<f64> {
    let record = &bytes[index * record_bytes..(index + 1) * record_bytes];
    (0..count)
        .map(|value_index| read_f64(record, 8 * value_index))
        .collect()
}

fn read_record_single(bytes: &[u8], record_bytes: usize, index: usize, count: usize) -> Vec<f32> {
    let record = &bytes[index * record_bytes..(index + 1) * record_bytes];
    (0..count)
        .map(|value_index| read_f32(record, 4 * value_index))
        .collect()
}

fn read_i32(bytes: &[u8], offset: usize) -> i32 {
    i32::from_le_bytes(bytes[offset..offset + 4].try_into().expect("i32 at offset"))
}

fn read_f32(bytes: &[u8], offset: usize) -> f32 {
    f32::from_le_bytes(bytes[offset..offset + 4].try_into().expect("f32 at offset"))
}

fn read_f64(bytes: &[u8], offset: usize) -> f64 {
    f64::from_le_bytes(bytes[offset..offset + 8].try_into().expect("f64 at offset"))
}

fn parse_values<T: std::str::FromStr>(record: &str, description: &str) -> Vec<T> {
    record
        .split_whitespace()
        .map(|value| {
            value
                .parse::<T>()
                .unwrap_or_else(|_| panic!("invalid {description} value {value:?}"))
        })
        .collect()
}

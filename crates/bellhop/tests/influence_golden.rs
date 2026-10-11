use std::fs;
use std::path::{Path, PathBuf};

use bellhop::legacy::load_case;
use bellhop::solver::{Arrival, SimulationLimits, run};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/golden")
        .join(name)
}

#[test]
fn geometric_and_simple_gaussian_eigenrays_match_v2023_5_golden() {
    for stem in ["GeoHat_eigen", "SimpleGaussian_eigen"] {
        let case = load_case(&fixture(&format!("{stem}.env"))).unwrap().value;
        let result = run(&case, SimulationLimits::default()).unwrap();
        let actual = &result.eigenray_sources[0].receivers[0].eigenrays;
        let expected = read_reference_rays(&fixture(&format!("{stem}.ray")));

        assert_eq!(actual.len(), expected.len());
        for (ray_index, (actual_ray, expected_ray)) in actual.iter().zip(&expected).enumerate() {
            assert_eq!(
                actual_ray.points.len(),
                expected_ray.len(),
                "ray {ray_index}"
            );
            for (point_index, (actual_point, expected_point)) in
                actual_ray.points.iter().zip(expected_ray).enumerate()
            {
                assert!(
                    (actual_point.range_m - expected_point[0]).abs() <= 1.0e-10,
                    "ray {ray_index} point {point_index} range"
                );
                assert!(
                    (actual_point.depth_m - expected_point[1]).abs() <= 1.0e-10,
                    "ray {ray_index} point {point_index} depth"
                );
            }
        }
    }
}

#[test]
fn geometric_hat_arrivals_match_v2023_5_golden() {
    let case = load_case(&fixture("GeoHat_arrival.env")).unwrap().value;
    let result = run(&case, SimulationLimits::default()).unwrap();
    let actual = &result.arrival_sources[0].receivers[0].arrivals;
    let expected = read_arrival_receivers(&fixture("GeoHat_arrival.arr"), 1);

    assert_eq!(actual, expected[0].as_slice());
}

#[test]
fn ray_centered_geometric_hat_arrivals_match_v2023_5_golden() {
    let case = load_case(&fixture("GeoHatRay_arrival.env")).unwrap().value;
    let result = run(&case, SimulationLimits::default()).unwrap();
    let actual = &result.arrival_sources[0].receivers;
    let expected = read_arrival_receivers(&fixture("GeoHatRay_arrival.arr"), actual.len());

    for (receiver_index, (actual_receiver, expected_receiver)) in
        actual.iter().zip(&expected).enumerate()
    {
        compare_arrivals(
            &actual_receiver.arrivals,
            expected_receiver,
            &format!("receiver {receiver_index}"),
        );
    }
}

#[test]
fn arrival_limit_retains_the_strongest_reference_arrival() {
    let case = load_case(&fixture("GeoHat_arrival.env")).unwrap().value;
    let result = run(
        &case,
        SimulationLimits {
            max_arrivals_per_receiver: 1,
            ..SimulationLimits::default()
        },
    )
    .unwrap();
    let arrivals = &result.arrival_sources[0].receivers[0].arrivals;

    assert_eq!(arrivals.len(), 1);
    assert!(arrivals[0].source_angle_degrees.abs() <= f32::EPSILON);
}

#[test]
fn official_munk_arrival_fan_matches_v2023_5_golden() {
    let case = load_case(&fixture("MunkB_Arr.env")).unwrap().value;
    let result = run(&case, SimulationLimits::default()).unwrap();
    let actual = &result.arrival_sources[0].receivers;
    let expected = read_arrival_receivers(&fixture("MunkB_Arr.arr"), actual.len());

    assert_eq!(actual.len(), expected.len());
    for (receiver_index, (actual_receiver, expected_receiver)) in
        actual.iter().zip(&expected).enumerate()
    {
        compare_arrivals(
            &actual_receiver.arrivals,
            expected_receiver,
            &format!("receiver {receiver_index}"),
        );
    }
}

#[test]
fn official_gaussian_arrival_grid_matches_v2023_5_golden() {
    const DEPTH_COUNT: usize = 4;
    const RANGE_COUNT: usize = 11;

    let case = load_case(&fixture("calibBarr.env")).unwrap().value;
    let result = run(&case, SimulationLimits::default()).unwrap();
    let actual = &result.arrival_sources[0].receivers;
    let expected = read_arrival_receivers(&fixture("calibBarr.arr"), DEPTH_COUNT * RANGE_COUNT);

    for depth_index in 0..DEPTH_COUNT {
        for range_index in 0..RANGE_COUNT {
            compare_arrivals(
                &actual[range_index * DEPTH_COUNT + depth_index].arrivals,
                &expected[depth_index * RANGE_COUNT + range_index],
                &format!("depth {depth_index} range {range_index}"),
            );
        }
    }
}

fn compare_arrivals(actual: &[Arrival], expected: &[Arrival], receiver: &str) {
    assert_eq!(actual.len(), expected.len(), "{receiver}");
    for (arrival_index, (actual, expected)) in actual.iter().zip(expected).enumerate() {
        let label = format!("{receiver} arrival {arrival_index}");
        assert_close(actual.amplitude, expected.amplitude, 1.0e-9, &label);
        assert_close(actual.phase_radians, expected.phase_radians, 1.0e-6, &label);
        assert_close(actual.travel_time_s, expected.travel_time_s, 5.0e-6, &label);
        assert_close(
            actual.attenuation_time_s,
            expected.attenuation_time_s,
            1.0e-7,
            &label,
        );
        assert_close(
            actual.source_angle_degrees,
            expected.source_angle_degrees,
            5.0e-5,
            &label,
        );
        assert_close(
            actual.receiver_angle_degrees,
            expected.receiver_angle_degrees,
            5.0e-5,
            &label,
        );
        assert_eq!(actual.top_bounces, expected.top_bounces, "{label}");
        assert_eq!(actual.bottom_bounces, expected.bottom_bounces, "{label}");
    }
}

fn read_reference_rays(path: &Path) -> Vec<Vec<[f64; 2]>> {
    let text = fs::read_to_string(path).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    let mut line_index = 7;
    let mut rays = Vec::new();
    while line_index < lines.len() {
        line_index += 1;
        let header: Vec<&str> = lines[line_index].split_whitespace().collect();
        line_index += 1;
        let point_count: usize = header[0].parse().unwrap();
        let mut points = Vec::with_capacity(point_count);
        for _ in 0..point_count {
            let fields: Vec<f64> = lines[line_index]
                .split_whitespace()
                .map(|field| field.parse().unwrap())
                .collect();
            points.push([fields[0], fields[1]]);
            line_index += 1;
        }
        rays.push(points);
    }
    rays
}

fn read_arrival_receivers(path: &Path, receiver_count: usize) -> Vec<Vec<Arrival>> {
    let text = fs::read_to_string(path).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    let mut line_index = 6;
    let mut receivers = Vec::with_capacity(receiver_count);
    for _ in 0..receiver_count {
        let arrival_count: usize = lines[line_index].trim().parse().unwrap();
        line_index += 1;
        let mut arrivals = Vec::with_capacity(arrival_count);
        for _ in 0..arrival_count {
            let fields: Vec<&str> = lines[line_index].split_whitespace().collect();
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
        receivers.push(arrivals);
    }
    receivers
}

fn assert_close(actual: f32, expected: f32, tolerance: f32, label: &str) {
    assert!(
        (actual - expected).abs() <= tolerance,
        "{label}: actual {actual}, expected {expected}, tolerance {tolerance}"
    );
}

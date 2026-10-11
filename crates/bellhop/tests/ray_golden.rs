use std::fs;
use std::path::{Path, PathBuf};

use bellhop::legacy::load_case;
use bellhop::solver::{SimulationLimits, run};

fn golden(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/golden")
        .join(name)
}

#[test]
fn all_ssp_models_match_v2023_5_golden_trajectories() {
    for stem in [
        "N2_one_ray",
        "CLinear_one_ray",
        "Spline_one_ray",
        "MunkP_one_ray",
        "Quadrilateral_one_ray",
        "Analytic_one_ray",
        "BeamShift",
    ] {
        let case = load_case(&golden(&format!("{stem}.env"))).unwrap().value;
        let result = run(&case, SimulationLimits::default()).unwrap();
        let actual = &result.sources[0].rays[0];
        let (top_bounces, bottom_bounces, expected) =
            read_single_ray(&golden(&format!("{stem}.ray")));

        if stem == "BeamShift" {
            assert!(case.environment.run.beam_shift && actual.bottom_bounces > 0);
            let mut definition = case.clone().into_definition();
            definition.environment.run.beam_shift = false;
            let unshifted = run(
                &bellhop::Case::from_definition(definition).unwrap().value,
                SimulationLimits::default(),
            )
            .unwrap();
            assert!(
                actual
                    .points
                    .iter()
                    .zip(&unshifted.sources[0].rays[0].points)
                    .any(|(a, b)| (a.range_m - b.range_m).abs() > 1.0e-5)
            );
        }
        assert_eq!(actual.top_bounces, top_bounces, "{stem}");
        assert_eq!(actual.bottom_bounces, bottom_bounces, "{stem}");
        assert_eq!(actual.points.len(), expected.len(), "{stem}");
        for (index, (point, [range_m, depth_m])) in actual.points.iter().zip(expected).enumerate() {
            assert!(
                (point.range_m - range_m).abs() < 1.0e-5,
                "{stem} point {index}: actual={} expected={range_m}",
                point.range_m
            );
            assert!(
                (point.depth_m - depth_m).abs() < 1.0e-5,
                "{stem} point {index}: actual={} expected={depth_m}",
                point.depth_m
            );
        }
    }
}

fn read_single_ray(path: &Path) -> (u32, u32, Vec<[f64; 2]>) {
    let source = fs::read_to_string(path).unwrap();
    let lines: Vec<&str> = source.lines().collect();
    let counts: Vec<u32> = lines[8]
        .split_whitespace()
        .map(|value| value.parse().unwrap())
        .collect();
    let points = lines[9..9 + counts[0] as usize]
        .iter()
        .map(|line| {
            let values: Vec<f64> = line
                .split_whitespace()
                .map(|value| value.parse().unwrap())
                .collect();
            [values[0], values[1]]
        })
        .collect();
    (counts[1], counts[2], points)
}

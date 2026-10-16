use kraken::{Case, ModeSolver, legacy, solve};
use num_complex::Complex64;
use std::f64::consts::{PI, TAU};
use std::path::{Path, PathBuf};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
        .with_extension("env")
}

#[test]
fn roughness_is_validated_and_legacy_smooth_defaults_stay_empty() {
    let path = fixture("Pekeris");
    let smooth = legacy::load_case(&path, path.with_extension("flp")).unwrap();
    assert_eq!(smooth.interface_roughness_m, [] as [f64; 0]);
    for values in [
        vec![1.0],
        vec![0.0, -1.0],
        vec![f64::NAN, 0.0],
        vec![0.0, f64::INFINITY],
    ] {
        let mut definition = smooth.clone().into_definition();
        definition.interface_roughness_m = values;
        assert!(
            Case::from_definition(definition)
                .unwrap_err()
                .to_string()
                .contains("interface_roughness_m")
        );
    }
    for name in ["FiniteElasticTopN", "ElasticHalfBottomN", "TabRefBrcN"] {
        let path = fixture(name);
        let mut definition = legacy::load_complex_case(&path, path.with_extension("flp"))
            .unwrap()
            .into_definition();
        definition.interface_roughness_m = vec![1.0; definition.additional_fluid_layers.len() + 2];
        assert!(
            Case::from_definition(definition)
                .unwrap_err()
                .to_string()
                .contains("without solids or tables")
        );
    }
}

#[test]
fn separated_duct_complex_roots_match_sturm_counts_across_meshes() {
    for name in ["SductTrapped", "SductSpline"] {
        let path = fixture(name);
        let mut definition = legacy::load_case(&path, path.with_extension("flp"))
            .unwrap()
            .into_definition();
        for mesh in [8000, 12000, 16000] {
            definition.mesh_points = mesh;
            definition.mode_solver = ModeSolver::Kraken;
            let real = solve(&Case::from_definition(definition.clone()).unwrap())
                .unwrap()
                .modes;
            definition.mode_solver = ModeSolver::Krakenc;
            let complex =
                kraken::solve_complex_modes(&Case::from_definition(definition.clone()).unwrap())
                    .unwrap();
            assert_eq!(complex.modes.len(), real.modes.len(), "{name}, mesh {mesh}");
            for (a, b) in complex.modes.iter().zip(&real.modes) {
                assert!(
                    (a.horizontal_wavenumber_rad_per_m - b.horizontal_wavenumber_rad_per_m).norm()
                        < 5e-10
                );
            }
        }
    }
}

#[test]
#[allow(
    clippy::too_many_lines,
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::unreadable_literal
)]
fn lossy_pekeris_spectra_match_independent_continuum_roots() {
    // Continuum pressure/velocity matching, independently counted by contour winding.
    for (name, frequency, roots) in [
        (
            "PekerisDenseLoss",
            75.0,
            &[
                (0.3174764792997787, -1.4550259802290756e-5),
                (0.3146628029238493, -5.5193670917995116e-5),
                (0.30988024579452556, -1.1581245718546011e-4),
                (0.3029986994414734, -1.9323971160994485e-4),
                (0.29384436665178615, -2.936495313949363e-4),
                (0.28219514741674945, -4.4573696338399336e-4),
                (0.2677597622920507, -7.655604602763797e-4),
            ] as &[(f64, f64)],
        ),
        (
            "PekerisRigidLoss",
            75.0,
            &[
                (0.31817308026560853, -3.6914656669861474e-6),
                (0.31630936045863567, -3.1987231678736156e-5),
                (0.3125247444079443, -8.334577936881603e-5),
                (0.3067116653885757, -1.5234204021891774e-4),
                (0.29871828160389874, -2.3959076248196594e-4),
                (0.2883476422707467, -3.5965638336788954e-4),
                (0.2753491181321132, -5.68669710798699e-4),
            ],
        ),
        (
            "PekerisRigidLoss",
            50.0,
            &[
                (0.21194601759380474, -7.428039912374515e-6),
                (0.20931762151182673, -6.241083955822664e-5),
                (0.20387111380159803, -0.00015793148780143544),
                (0.19527140086991487, -0.0002967939115802483),
                (0.18306547667161774, -0.0005700251595041456),
            ],
        ),
    ] {
        let path = fixture(name);
        let mut definition = legacy::load_complex_case(&path, path.with_extension("flp"))
            .unwrap()
            .into_definition();
        definition.frequency_hz = frequency;
        let omega = TAU * definition.frequency_hz;
        let depth = definition.water_depth_m;
        let speed = definition.sound_speed_profile[0].sound_speed_mps;
        let water_k2 = (omega / speed).powi(2);
        let bottom_c = Complex64::new(
            definition.bottom_sound_speed_mps,
            definition.bottom_attenuation_db_per_wavelength * definition.bottom_sound_speed_mps
                / (8.685_889_6 * TAU),
        );
        let bottom_k2 = (omega / bottom_c).powi(2);
        let ratio = definition.water_density_g_cm3 / definition.bottom_density_g_cm3;
        let rigid = definition.surface_boundary == kraken::SurfaceBoundary::Rigid;
        let dispersion = |x: Complex64| {
            let q = (water_k2 - x).sqrt();
            let gamma = (x - bottom_k2).sqrt();
            if rigid {
                -q * (q * depth).sin() + ratio * gamma * (q * depth).cos()
            } else {
                (q * depth).cos() + ratio * gamma * (q * depth).sin() / q
            }
        };
        let low = (omega / definition.c_high_mps).powi(2);
        let high = (omega / definition.c_low_mps).powi(2);
        assert!(bottom_k2.re < low); // No half-space branch point inside the contour.
        assert_continuum_root_count(&dispersion, low, high, roots.len() as f64);
        for mesh in [4000, 8000, 12000] {
            definition.mesh_points = mesh;
            let case = Case::from_definition(definition.clone()).unwrap();
            let result = solve(&case).unwrap();
            assert_eq!(result.modes.modes.len(), roots.len(), "{name}, mesh {mesh}");
            let mut analytic = Vec::new();
            for (mode, &(re, im)) in result.modes.modes.iter().zip(roots) {
                let k = Complex64::new(re, im);
                assert!(dispersion(k * k).norm() < 1e-11);
                assert!(
                    (mode.horizontal_wavenumber_rad_per_m - k).norm() < 5e-10,
                    "{name}, mesh {mesh}: {} vs {k}",
                    mode.horizontal_wavenumber_rad_per_m
                );
                let q = (water_k2 - k * k).sqrt();
                let gamma = (k * k - bottom_k2).sqrt();
                let raw = |z: f64| if rigid { (q * z).cos() } else { (q * z).sin() };
                let water_norm = (depth * 0.5
                    + if rigid { 1.0 } else { -1.0 } * (2.0 * q * depth).sin() / (4.0 * q))
                    / case.water_density_g_cm3;
                let bottom_norm = raw(depth).powi(2) / (2.0 * gamma * case.bottom_density_g_cm3);
                let norm = water_norm + bottom_norm;
                let group = (k * norm
                    / (omega * (water_norm / speed.powi(2) + bottom_norm / bottom_c.powi(2))))
                .re;
                assert!((mode.group_speed_mps - group).abs() < 0.005);
                let shapes: Vec<_> = case
                    .mode_sample_depths_m
                    .iter()
                    .map(|&z| raw(z) / norm.sqrt())
                    .collect();
                let overlap: Complex64 = mode
                    .eigenfunction
                    .iter()
                    .zip(&shapes)
                    .map(|(a, b)| a.conj() * b)
                    .sum();
                let sign = if overlap.re < 0.0 { -1.0 } else { 1.0 };
                for (actual, expected) in mode.eigenfunction.iter().zip(&shapes) {
                    assert!(
                        (*actual - sign * expected).norm() < 1e-6,
                        "{name}, mesh {mesh}, k={k}: shape {actual} vs {}",
                        sign * expected
                    );
                }
                // FIELD propagates the MOD-format f32 wavenumbers, not unrounded continuum k.
                analytic.push((
                    Complex64::new(f64::from(re as f32), f64::from(im as f32)),
                    shapes,
                ));
            }
            assert_eq!(case.source_geometry, kraken::SourceGeometry::Line);
            let factor = Complex64::i() * Complex64::from_polar(TAU.sqrt(), PI * 0.25);
            let sample = |z: f64| {
                case.mode_sample_depths_m
                    .binary_search_by(|d| d.total_cmp(&z))
                    .unwrap()
            };
            let mut index = 0;
            for &sd in &case.source_depths_m {
                for &rd in &case.receiver_depths_m {
                    for &range in &case.receiver_ranges_m {
                        let expected: Complex64 = analytic
                            .iter()
                            .map(|(k, phi)| {
                                factor * phi[sample(sd)] * phi[sample(rd)] / k
                                    * (-Complex64::i() * k * range).exp()
                            })
                            .sum();
                        assert!(
                            (result.field.pressure[index] - expected).norm() < 2e-6,
                            "{name}, mesh {mesh}, pressure {index}: {} vs {expected}",
                            result.field.pressure[index]
                        );
                        index += 1;
                    }
                }
            }
            assert_eq!(index, result.field.pressure.len());
        }
    }
}

fn assert_continuum_root_count(
    dispersion: &impl Fn(Complex64) -> Complex64,
    low: f64,
    high: f64,
    expected: f64,
) {
    for bottom in [-0.01, -0.1] {
        let corners = [
            Complex64::new(low, bottom),
            Complex64::new(high, bottom),
            Complex64::new(high, 1e-4),
            Complex64::new(low, 1e-4),
        ];
        for steps in [4096, 8192] {
            let mut previous = dispersion(corners[0]);
            let mut winding = 0.0;
            for edge in 0..4 {
                for i in 1..=steps {
                    let x = corners[edge]
                        + (corners[(edge + 1) % 4] - corners[edge]) * f64::from(i)
                            / f64::from(steps);
                    let value = dispersion(x);
                    let angle = (value / previous).arg();
                    assert!(
                        angle.is_finite() && angle.abs() < 0.3,
                        "contour needs refinement"
                    );
                    winding += angle;
                    previous = value;
                }
            }
            assert!(
                (winding / TAU - expected).abs() < 1e-8,
                "continuum root count {winding}"
            );
        }
    }
}

#[test]
fn vacuum_surface_loss_matches_independent_small_roughness_formula() {
    let path = fixture("Pekeris");
    let mut definition = legacy::load_case(&path, path.with_extension("flp"))
        .unwrap()
        .into_definition();
    definition.bottom_boundary = kraken::BottomBoundary::Rigid;
    definition.bottom_sound_speed_mps = 0.0;
    definition.bottom_density_g_cm3 = 0.0;
    definition.bottom_attenuation_db_per_wavelength = 0.0;
    definition.mesh_points = 2000;
    definition.c_high_mps = 1510.0; // Isolate the first analytic mode.
    definition.max_range_m = 1e6;
    definition.interface_roughness_m = vec![0.05, 0.0];
    let kz = std::f64::consts::PI / (2.0 * definition.water_depth_m);
    let kx =
        ((2.0 * std::f64::consts::PI * definition.frequency_hz / 1500.0).powi(2) - kz * kz).sqrt();
    let expected = 0.05_f64.powi(2) * kz.powi(3) / (definition.water_depth_m * kx);
    for solver in [ModeSolver::Kraken, ModeSolver::Krakenc] {
        definition.mode_solver = solver;
        let result = solve(&Case::from_definition(definition.clone()).unwrap()).unwrap();
        let loss = result.modes.modes[0].attenuation_nepers_per_m;
        assert!(
            (loss / expected - 1.0).abs() < 2e-6,
            "{solver:?}: {loss} vs {expected}"
        );
        let mut twice = definition.clone();
        twice.interface_roughness_m[0] *= 2.0;
        let doubled = solve(&Case::from_definition(twice).unwrap()).unwrap();
        assert!((doubled.modes.modes[0].attenuation_nepers_per_m / loss - 4.0).abs() < 1e-9);
    }
}

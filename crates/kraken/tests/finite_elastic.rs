use kraken::{Boundary, Case, ModeSolver, legacy, solve};
use std::path::{Path, PathBuf};

fn original(name: &str, solver: ModeSolver) -> Case {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name);
    legacy::load_frequency_cases(
        root.with_extension("env"),
        root.with_extension("flp"),
        solver,
    )
    .unwrap()
    .remove(0)
}

#[test]
fn graded_finite_elastic_inputs_keep_complete_material_profiles() {
    for name in [
        "GradedElasticTopN",
        "GradedElasticTopC",
        "GradedElasticTopP",
        "GradedElasticTopS",
        "GradedElasticBottomN",
        "GradedElasticBottomC",
        "GradedElasticBottomP",
        "GradedElasticBottomS",
        "GradedElasticStack",
        "GradedElasticPower",
        "GradedElasticBio",
    ] {
        for solver in [ModeSolver::Kraken, ModeSolver::Krakenc] {
            let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures")
                .join(name);
            let cases = legacy::load_field_cases(
                root.with_extension("env"),
                root.with_extension("flp"),
                solver,
            )
            .unwrap();
            for field in &cases {
                for case in field.profiles() {
                    for layer in case
                        .top_elastic_layers
                        .iter()
                        .chain(&case.bottom_elastic_layers)
                    {
                        assert_eq!(layer.material_profile.len(), 4);
                        assert!(
                            layer.material_profile[3].density_g_cm3
                                > layer.material_profile[0].density_g_cm3
                        );
                    }
                }
            }
            let document = kraken::input::load_legacy(
                &root.with_extension("env"),
                &root.with_extension("flp"),
                solver,
            )
            .unwrap()
            .document()
            .unwrap();
            let restored =
                kraken::json::load_case_document(&serde_json::to_vec(&document).unwrap()).unwrap();
            assert_eq!(restored, cases);
            for result in kraken::solve_frequencies(&cases) {
                let result = result.unwrap();
                assert_ne!(result.modes[0].modes, []);
                assert_eq!(result.field.pressure.len(), 63);
            }
        }
    }
}

#[test]
fn graded_elastic_validation_checks_nodes_and_interpolated_material() {
    let base = original("GradedElasticTopS", ModeSolver::Krakenc).into_definition();
    for change in [
        "depth",
        "duplicate",
        "cp",
        "cs",
        "density",
        "p_loss",
        "s_loss",
        "bulk",
        "scalar",
        "count",
    ] {
        let mut input = base.clone();
        let layer = &mut input.top_elastic_layers[0];
        let point = &mut layer.material_profile[2];
        match change {
            "depth" => point.depth_m = f64::NAN,
            "duplicate" => point.depth_m = 0.0,
            "cp" => point.compressional_sound_speed_mps = f64::INFINITY,
            "cs" => point.shear_sound_speed_mps = 0.0,
            "density" => point.density_g_cm3 = -1.0,
            "p_loss" => point.compressional_attenuation_db_per_wavelength = -1.0,
            "s_loss" => point.shear_attenuation_db_per_wavelength = f64::NAN,
            "bulk" => point.compressional_sound_speed_mps = 100.0,
            "scalar" => layer.density_g_cm3 = 3.0,
            "count" => {
                layer.material_profile.pop();
            }
            _ => unreachable!(),
        }
        assert!(
            Case::from_definition(input)
                .unwrap_err()
                .diagnostics()
                .iter()
                .any(|d| d.field == "top_elastic_layers[0].material_profile"),
            "{change}"
        );
    }
    let mut input = base.clone();
    input.top_elastic_layers[0].material_profile[2].density_g_cm3 = 1e8;
    assert!(
        Case::from_definition(input)
            .unwrap_err()
            .to_string()
            .contains("density must stay")
    );
    let mut input = base.clone();
    for (point, cs) in input.top_elastic_layers[0]
        .material_profile
        .iter_mut()
        .zip([1400.0, 2590.0, 1800.0, 1400.0])
    {
        point.compressional_sound_speed_mps = 3000.0;
        point.shear_sound_speed_mps = cs;
    }
    assert!(
        Case::from_definition(input)
            .unwrap_err()
            .to_string()
            .contains("positive bulk modulus")
    );
    let mut input = base;
    input.top_elastic_layers[0].material_profile[2].compressional_attenuation_db_per_wavelength =
        50.0;
    assert!(
        Case::from_definition(input)
            .unwrap_err()
            .to_string()
            .contains("interpolated complex speed")
    );
}

#[test]
fn graded_elastic_storage_counts_both_node_losses() {
    let mut input = original("GradedElasticTopC", ModeSolver::Kraken).into_definition();
    let layer = &mut input.top_elastic_layers[0];
    let first = layer.material_profile[0];
    layer.material_profile = (0..=50_000)
        .map(|i| kraken::ElasticMaterialPoint {
            depth_m: f64::from(i) * 20.0 / 50_000.0,
            ..first
        })
        .collect();
    assert!(
        Case::from_definition(input)
            .unwrap_err()
            .to_string()
            .contains("total fluid profile storage")
    );
}

#[test]
fn finite_elastic_layers_keep_the_absolute_fluid_interval() {
    let ice = original("OriginalElasticIce", ModeSolver::Krakenc);
    assert_eq!(
        (
            ice.fluid_top_depth_m(),
            ice.fluid_bottom_depth_m(),
            ice.total_depth_m()
        ),
        (30.0, 5000.0, 5000.0)
    );
    assert_eq!(
        ice.sound_speed_profile[0].depth_m.to_bits(),
        30.0_f64.to_bits()
    );
    assert_eq!(ice.top_elastic_layers.len(), 1);
    assert_eq!(ice.mesh_points, 500);
    let mut input = ice.into_definition();
    input.source_depths_m[0] = 10.0;
    assert!(
        Case::from_definition(input)
            .unwrap_err()
            .diagnostics()
            .iter()
            .any(|d| d.field == "source_depths_m")
    );
    let sediment = original("OriginalElasticSediment", ModeSolver::Kraken);
    assert_eq!(
        (
            sediment.fluid_top_depth_m(),
            sediment.fluid_bottom_depth_m(),
            sediment.total_depth_m()
        ),
        (0.0, 5000.0, 5100.0)
    );
    let mut input = sediment.into_definition();
    input.mode_sample_depths_m[0] = 5100.0;
    assert!(
        Case::from_definition(input)
            .unwrap_err()
            .diagnostics()
            .iter()
            .any(|d| d.field == "mode_sample_depths_m")
    );
}

#[test]
#[allow(clippy::cast_possible_truncation)]
fn legacy_fluid_interface_samples_keep_exact_bounds() {
    for top in [10.2_f64, 10.3] {
        let bottom = 100.3_f64;
        let env = format!(
            "'fractional cap'\n50\n2\n'NVN'\n50 0 {top}\n0 3000 1400 1 0 0\n{top} 3000 /\n101 0 {bottom}\n{top} 1500 0 1 0 0\n{bottom} 1500 /\n'A' 0\n{bottom} 2000 0 2 0 0\n1400 1900\n0\n2\n{top} {bottom} /\n2\n{top} {bottom} /\n"
        );
        let flp = format!(
            "'fractional FIELD'\n'RAOC'\n9999\n1\n0\n1\n0.5\n2\n{top} {bottom} /\n2\n{top} {bottom} /\n2\n0 0 /\n"
        );
        for solver in [ModeSolver::Kraken, ModeSolver::Krakenc] {
            let load = |env: &str, flp: &str| {
                legacy::load_frequency_cases_from_sources(
                    env,
                    flp,
                    Path::new("cap.env"),
                    Path::new("cap.flp"),
                    solver,
                )
            };
            let case = load(&env, &flp).unwrap().remove(0);
            for depths in [
                &case.mode_sample_depths_m,
                &case.source_depths_m,
                &case.receiver_depths_m,
            ] {
                assert_eq!(
                    depths.iter().map(|d| d.to_bits()).collect::<Vec<_>>(),
                    [top.to_bits(), bottom.to_bits()]
                );
            }
            // An adjacent f32 value genuinely inside the solid must not be snapped.
            let outside = f32::from_bits((top as f32).to_bits() - 1).to_string();
            for (bad_env, bad_flp, field) in [
                (
                    env.replace(
                        &format!("2\n{top} {bottom} /"),
                        &format!("2\n{outside} {bottom} /"),
                    ),
                    flp.clone(),
                    "mode_sample_depths_m",
                ),
                (
                    env.clone(),
                    flp.replace(
                        &format!("2\n{top} {bottom} /"),
                        &format!("2\n{outside} {bottom} /"),
                    ),
                    "source_depths_m",
                ),
            ] {
                assert!(
                    load(&bad_env, &bad_flp)
                        .unwrap_err()
                        .diagnostics()
                        .iter()
                        .any(|d| d.field == field)
                );
            }
            // The Rust API keeps exact bounds; only legacy f32 interface spellings are normalized.
            let mut input = case.into_definition();
            input.source_depths_m[0] = top - 1e-8;
            assert!(
                Case::from_definition(input)
                    .unwrap_err()
                    .diagnostics()
                    .iter()
                    .any(|d| d.field == "source_depths_m")
            );
        }
    }
}

#[test]
fn finite_elastic_materials_and_unvalidated_combinations_are_rejected() {
    let base = original("OriginalElasticIce", ModeSolver::Krakenc).into_definition();
    for change in ["cp", "cs", "density", "p_loss", "s_loss", "bulk", "depth"] {
        let mut input = base.clone();
        let layer = &mut input.top_elastic_layers[0];
        match change {
            "cp" => layer.compressional_sound_speed_mps = f64::NAN,
            "cs" => layer.shear_sound_speed_mps = f64::INFINITY,
            "density" => layer.density_g_cm3 = 0.0,
            "p_loss" => layer.compressional_attenuation_db_per_wavelength = -1.0,
            "s_loss" => layer.shear_attenuation_db_per_wavelength = f64::NAN,
            "bulk" => layer.compressional_sound_speed_mps = 1500.0,
            "depth" => layer.bottom_depth_m = f64::INFINITY,
            _ => unreachable!(),
        }
        assert!(
            Case::from_definition(input)
                .unwrap_err()
                .diagnostics()
                .iter()
                .any(|d| d.field.starts_with("top_elastic_layers[0]")),
            "{change}"
        );
    }
    let mut input = original("OriginalElasticSediment", ModeSolver::Krakenc).into_definition();
    input.bottom_boundary = Boundary::FluidHalfSpace;
    assert!(
        Case::from_definition(input)
            .unwrap_err()
            .to_string()
            .contains("compound state is undefined")
    );
    let env = include_str!("fixtures/OriginalElasticIce.env")
        .replace("30.0 3000.0 1400.0 1.0", "30.0 1500.0 1400.0 1.0");
    assert!(
        legacy::load_frequency_cases_from_sources(
            &env,
            include_str!("fixtures/OriginalElasticIce.flp"),
            Path::new("ice.env"),
            Path::new("ice.flp"),
            ModeSolver::Krakenc
        )
        .unwrap_err()
        .to_string()
        .contains("positive bulk modulus")
    );
    let mut input = base;
    input.top_elastic_layers[0].mesh_points = 1_000_000;
    assert!(
        solve(&Case::from_definition(input).unwrap())
            .unwrap_err()
            .to_string()
            .contains("mesh limit")
    );
}

#[test]
fn finite_loss_and_mesh_scaling_preserve_frequency_order() {
    let cases = legacy::load_frequency_cases_from_sources(
        include_str!("fixtures/FiniteElasticPower.env"),
        include_str!("fixtures/FiniteElasticPower.flp"),
        Path::new("power.env"),
        Path::new("power.flp"),
        ModeSolver::Krakenc,
    )
    .unwrap();
    assert_eq!(
        cases.iter().map(|c| c.frequency_hz).collect::<Vec<_>>(),
        [75.0, 50.0, 62.5, 50.0]
    );
    for (case, factor) in cases
        .iter()
        .zip([1.1_f64.powf(0.25), 1.0, 1.1_f64.powf(0.25), 1.0])
    {
        assert!(
            (case.bottom_elastic_layers[0].shear_attenuation_db_per_wavelength - 0.3 * factor)
                .abs()
                < 1e-14
        );
    }
    assert_eq!(cases[1], cases[3]);
    let mut input = cases[0].clone().into_definition();
    input.mode_solver = ModeSolver::Kraken;
    Case::from_definition(input).unwrap();
}

#[test]
fn frequency_runs_reset_search_state_and_stop_after_failure() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/FiniteElasticPower");
    let cases = legacy::load_field_cases(
        root.with_extension("env"),
        root.with_extension("flp"),
        ModeSolver::Kraken,
    )
    .unwrap();
    let first = kraken::solve_frequencies(&cases)
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(first[1], first[3]);
    assert_eq!(first[2].modes[0].modes.len(), 6);
    // The same canonical 62.5 Hz case starts a separate pinned run with seven,
    // rather than inheriting the preceding 50 Hz Solve2 bound of six.
    assert_eq!(solve(&cases[2].profiles()[0]).unwrap().modes.modes.len(), 7);
    assert_eq!(
        kraken::solve_frequencies(&cases)
            .collect::<Result<Vec<_>, _>>()
            .unwrap(),
        first
    );

    let mut input = cases[1].profiles()[0].clone().into_definition();
    input.mesh_points = 10;
    let bad = kraken::FieldCase::new(
        vec![Case::from_definition(input).unwrap()],
        vec![0.0],
        kraken::FieldPropagation::RangeIndependent,
    )
    .unwrap();
    let sequence = [cases[0].clone(), bad, cases[3].clone()];
    let mut results = kraken::solve_frequencies(&sequence);
    assert!(results.next().unwrap().is_ok());
    assert_eq!(
        results.next().unwrap().unwrap_err().diagnostics()[0].code,
        "KR0302"
    );
    assert!(results.next().is_none());
    assert!(results.next().is_none());
}

#[test]
fn frequency_runs_reject_mixed_solvers_before_solving_the_mismatched_block() {
    use kraken::{FieldCase, FieldPropagation, solve_field, solve_frequencies};

    let fixtures = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let complex = legacy::load_complex_case(
        fixtures.join("PekerisComplexCLow.env"),
        fixtures.join("Pekeris.flp"),
    )
    .unwrap();
    let real = original("FiniteElasticBothN", ModeSolver::Kraken);
    assert!(solve_frequencies(&[]).next().is_none());
    for ranges in [vec![0.0], vec![0.0, 1000.0]] {
        let propagation = if ranges.len() == 1 {
            FieldPropagation::RangeIndependent
        } else {
            FieldPropagation::Adiabatic
        };
        let fields = [complex.clone(), real.clone()].map(|case| {
            FieldCase::new(vec![case; ranges.len()], ranges.clone(), propagation).unwrap()
        });
        for order in [[0, 1], [1, 0]] {
            let first = &fields[order[0]];
            let expected = solve_field(first).unwrap();
            assert_eq!(
                expected.modes[0].modes.len(),
                if order[0] == 0 { 3 } else { 6 }
            );
            let homogeneous = [first.clone(), first.clone()];
            for result in solve_frequencies(&homogeneous) {
                assert_eq!(result.unwrap(), expected);
            }
            let sequence = [first.clone(), fields[order[1]].clone(), first.clone()];
            let mut results = solve_frequencies(&sequence);
            assert_eq!(results.next().unwrap().unwrap(), expected);
            let Some(Err(report)) = results.next() else {
                panic!(
                    "mixed-solver frequency block must be rejected, not publish a truncated spectrum"
                );
            };
            let diagnostic = &report.diagnostics()[0];
            assert_eq!(diagnostic.code, "KR0201");
            assert_eq!(diagnostic.field, "mode_solver");
            assert_eq!(
                diagnostic.message,
                "frequency blocks must share the same mode solver"
            );
            assert!(results.next().is_none());
            assert!(results.next().is_none());
        }
    }
}

#[test]
fn real_finite_stiffness_is_not_the_lossless_half_space_approximation() {
    let case = original("OriginalElasticIce", ModeSolver::Kraken);
    let with_loss = solve(&case).unwrap();
    assert!(
        with_loss
            .modes
            .modes
            .iter()
            .all(|m| m.attenuation_nepers_per_m == 0.0)
    );
    let mut input = case.into_definition();
    input.top_elastic_layers[0].compressional_attenuation_db_per_wavelength = 0.0;
    input.top_elastic_layers[0].shear_attenuation_db_per_wavelength = 0.0;
    let lossless = solve(&Case::from_definition(input).unwrap()).unwrap();
    assert!(
        with_loss
            .modes
            .modes
            .iter()
            .zip(lossless.modes.modes)
            .any(
                |(a, b)| (a.horizontal_wavenumber_rad_per_m - b.horizontal_wavenumber_rad_per_m)
                    .norm()
                    > 1e-12
            )
    );
    let complex = solve(&original("OriginalElasticIce", ModeSolver::Krakenc)).unwrap();
    assert!(
        complex
            .modes
            .modes
            .iter()
            .all(|m| m.attenuation_nepers_per_m > 0.0)
    );
}

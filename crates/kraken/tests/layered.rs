use kraken::{Case, CaseDefinition, Interpolation, ModeSolver, SoundSpeedPoint, legacy, solve};
use std::path::Path;

fn cases(source: &str, solver: ModeSolver) -> Result<Vec<Case>, kraken::DiagnosticReport> {
    legacy::load_frequency_cases_from_sources(
        source,
        include_str!("fixtures/LayeredFluidN.flp"),
        Path::new("layers.env"),
        Path::new("layers.flp"),
        solver,
    )
}

fn definition() -> CaseDefinition {
    cases(
        include_str!("fixtures/LayeredFluidN.env"),
        ModeSolver::Krakenc,
    )
    .unwrap()
    .remove(0)
    .into_definition()
}

#[test]
fn layered_fluid_legacy_input_is_accepted_by_both_backends() {
    for solver in [ModeSolver::Kraken, ModeSolver::Krakenc] {
        let case = cases(include_str!("fixtures/LayeredFluidN.env"), solver)
            .unwrap()
            .remove(0);
        assert_eq!(case.additional_fluid_layers.len(), 1);
        assert_eq!(case.total_depth_m().to_bits(), 140.0_f64.to_bits());
        // Full modes and all 63 pressures are checked by the layered-fluid goldens.
    }
}

#[test]
fn original_double_keeps_the_complete_pinned_spectrum() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/OriginalLayeredDouble");
    for solver in [ModeSolver::Kraken, ModeSolver::Krakenc] {
        let case = legacy::load_frequency_cases(
            root.with_extension("env"),
            root.with_extension("flp"),
            solver,
        )
        .unwrap()
        .remove(0);
        let result = solve(&case).unwrap();
        assert_eq!(result.modes.modes.len(), 42);
        assert_eq!(result.modes.sampled_depths_m, [500.0, 2500.0]);
        assert_eq!(result.field.pressure.len(), 501);
        let mut first = case.clone().into_definition();
        first.max_range_m = 0.0;
        let first = solve(&Case::from_definition(first).unwrap()).unwrap();
        assert_eq!(first.modes.modes.len(), 43);
        for (mode, initial) in result.modes.modes.iter().zip(&first.modes.modes) {
            assert_eq!(mode.eigenfunction, initial.eigenfunction);
            assert_eq!(
                mode.group_speed_mps.to_bits(),
                initial.group_speed_mps.to_bits()
            );
            assert_ne!(
                mode.horizontal_wavenumber_rad_per_m,
                initial.horizontal_wavenumber_rad_per_m
            );
        }
        // A new leading mode has no first-mesh shape: reductions must not
        // accidentally make an increased count safe to publish.
        let mut increased = case.into_definition();
        increased.c_low_mps = first.modes.modes[0]
            .phase_speed_mps
            .midpoint(result.modes.modes[0].phase_speed_mps);
        increased.c_high_mps = 1600.0;
        let report = solve(&Case::from_definition(increased).unwrap()).unwrap_err();
        assert_eq!(report.diagnostics()[0].code, "KR0303");
        assert!(report.to_string().contains("mode count changed"));
    }
}

#[test]
fn wide_three_layer_refinement_keeps_the_pinned_mode_count() {
    // Pinned mesh 1 finds five roots, mesh 2 only four; surviving first-mesh
    // shapes and group speeds must not change when the search really contracts.
    let input = cases(
        include_str!("fixtures/LayeredFluidThreeWide.env"),
        ModeSolver::Krakenc,
    )
    .unwrap()
    .remove(0)
    .into_definition();
    let first = solve(&Case::from_definition(input.clone()).unwrap()).unwrap();
    assert_eq!(first.modes.modes.len(), 5);
    let result = solve(
        &Case::from_definition(CaseDefinition {
            max_range_m: 1_000_000.0,
            ..input
        })
        .unwrap(),
    )
    .unwrap();
    assert_eq!(result.modes.modes.len(), 4);
    assert_eq!(result.field.pressure.len(), 63);
    for (mode, initial) in result.modes.modes.iter().zip(&first.modes.modes) {
        assert_eq!(mode.eigenfunction, initial.eigenfunction);
        assert_eq!(
            mode.group_speed_mps.to_bits(),
            initial.group_speed_mps.to_bits()
        );
        assert_ne!(
            mode.horizontal_wavenumber_rad_per_m,
            initial.horizontal_wavenumber_rad_per_m
        );
    }
}

#[test]
fn layer_validation_keeps_interfaces_and_materials_explicit() {
    let base = definition();
    for change in [
        "empty",
        "gap",
        "overlap",
        "reversed",
        "zeroSpeed",
        "nanSpeed",
        "infiniteDepth",
        "density",
        "nanDensity",
        "mesh",
        "shortLoss",
        "gain",
        "nanLoss",
        "infiniteLoss",
        "excessLoss",
    ] {
        let mut input = base.clone();
        let layer = &mut input.additional_fluid_layers[0];
        match change {
            "empty" => layer.sound_speed_profile.clear(),
            "gap" => layer.sound_speed_profile[0].depth_m += 0.1,
            "overlap" => layer.sound_speed_profile[0].depth_m -= 0.1,
            "reversed" => layer.bottom_depth_m = 69.0,
            "zeroSpeed" => layer.sound_speed_profile[1].sound_speed_mps = 0.0,
            "nanSpeed" => layer.sound_speed_profile[1].sound_speed_mps = f64::NAN,
            "infiniteDepth" => layer.bottom_depth_m = f64::INFINITY,
            "density" => layer.density_g_cm3 = 0.0,
            "nanDensity" => layer.density_g_cm3 = f64::NAN,
            "mesh" => layer.mesh_points = 9,
            "shortLoss" => {
                layer.attenuation_db_per_wavelength.pop();
            }
            "gain" => layer.attenuation_db_per_wavelength[1] = -0.1,
            "nanLoss" => layer.attenuation_db_per_wavelength[1] = f64::NAN,
            "infiniteLoss" => layer.attenuation_db_per_wavelength[1] = f64::INFINITY,
            _ => layer.attenuation_db_per_wavelength[1] = 55.0,
        }
        let report = Case::from_definition(input).unwrap_err();
        assert!(
            report
                .diagnostics()
                .iter()
                .any(|d| d.field.starts_with("additional_fluid_layers[0].")),
            "{change}: {report}"
        );
    }
    let mut input = base.clone();
    input.additional_fluid_layers = vec![base.additional_fluid_layers[0].clone(); 500];
    assert!(
        Case::from_definition(input)
            .unwrap_err()
            .to_string()
            .contains("500 total finite media")
    );
    let mut input = base;
    input.interpolation = Interpolation::AnalyticMunk;
    assert!(
        Case::from_definition(input)
            .unwrap_err()
            .to_string()
            .contains("analytic Munk remains single-layer")
    );
}

#[test]
fn mesh_and_profile_budgets_are_shared_across_layers() {
    let mut input = definition();
    input.mesh_points = 500_001;
    input.additional_fluid_layers[0].mesh_points = 500_000;
    let report = solve(&Case::from_definition(input).unwrap()).unwrap_err();
    assert_eq!(report.diagnostics()[0].code, "KR0302");
    assert!(report.to_string().contains("total finite mesh exceeds"));
    let mut input = definition();
    input.water_attenuation_db_per_wavelength.clear();
    input.additional_fluid_layers[0]
        .attenuation_db_per_wavelength
        .clear();
    for (points, top) in [
        (&mut input.sound_speed_profile, 0.0),
        (
            &mut input.additional_fluid_layers[0].sound_speed_profile,
            70.0,
        ),
    ] {
        *points = (0..60_001)
            .map(|i| SoundSpeedPoint {
                depth_m: top + f64::from(i) * 70.0 / 60_000.0,
                sound_speed_mps: 1500.0,
            })
            .collect();
    }
    assert!(
        Case::from_definition(input)
            .unwrap_err()
            .to_string()
            .contains("total fluid profile storage")
    );
}

#[test]
fn legacy_layer_diagnostics_point_to_the_offending_medium() {
    let base = include_str!("fixtures/LayeredFluidN.env");
    for (old, new) in [
        ("89.0 1480.0 0.0 1.6", "89.0 1480.0 0.0 1.7"),
        ("70.0 1450.0 0.0", "70.0 1450.0 100.0"),
        ("70.0 1450.0", "70.01 1450.0"),
        ("83 0.0 140.0", "83 1.0 140.0"),
    ] {
        let report = cases(&base.replace(old, new), ModeSolver::Krakenc).unwrap_err();
        let diagnostic = &report.diagnostics()[0];
        assert_eq!(diagnostic.path, Path::new("layers.env"));
        assert!(diagnostic.line >= 9, "{report}");
        assert!(
            diagnostic.field.starts_with("additional_fluid_layers[0]."),
            "{report}"
        );
    }
    assert!(
        cases(&base.replace("50.0\n2", "50.0\n501"), ModeSolver::Krakenc)
            .unwrap_err()
            .to_string()
            .contains("500 total finite media")
    );
    for source in [
        base.replace("'NVW'", "'AVW'"),
        base.replace("'NVW'", "'NFW'"),
        base.replace("'A' 0.0", "'F' 0.0"),
        base.replace("'A' 0.0", "'P' 0.0"),
    ] {
        assert!(
            cases(&source, ModeSolver::Krakenc)
                .unwrap_err()
                .to_string()
                .contains("single-layer")
        );
    }
}

#[test]
fn layer_spline_loss_undershoot_and_cumulative_input_copies_are_rejected() {
    let mut input = definition();
    input.interpolation = Interpolation::Spline;
    input.additional_fluid_layers[0].attenuation_db_per_wavelength = vec![0.02, 0.0, 0.0];
    let report = Case::from_definition(input).unwrap_err();
    assert!(
        report
            .diagnostics()
            .iter()
            .any(|d| d.field == "additional_fluid_layers[0].attenuation_db_per_wavelength")
    );
    assert!(
        report
            .diagnostics()
            .iter()
            .all(|d| d.field != "sound_speed_profile"
                && d.field != "water_attenuation_db_per_wavelength"),
        "{report}"
    );
    let source = include_str!("fixtures/LayeredFluidN.env")
        .replace("'NVW'", "'NVW  B'")
        .replace("\n7\n", "\n3000\n")
        .replace("0.0 69.99 70.0 70.01 105.0 139.99 140.0 /", "0.0 140.0 /")
        + "1000\n"
        + &"50.0 ".repeat(1000)
        + "\n";
    let field = include_str!("fixtures/LayeredFluidN.flp")
        .replace("\n7\n", "\n3000\n")
        .replace("0.0 69.99 70.0 70.01 105.0 139.99 140.0 /", "0.0 140.0 /");
    let report = legacy::load_frequency_cases_from_sources(
        &source,
        &field,
        Path::new("layers.env"),
        Path::new("layers.flp"),
        ModeSolver::Krakenc,
    )
    .unwrap_err();
    assert!(
        report.to_string().contains("input storage limit"),
        "{report}"
    );
}

#[test]
fn loss_is_converted_per_layer_and_frequency_without_reordering() {
    let cases = cases(
        include_str!("fixtures/LayeredFluidPower.env"),
        ModeSolver::Krakenc,
    )
    .unwrap();
    assert_eq!(
        cases.iter().map(|c| c.frequency_hz).collect::<Vec<_>>(),
        [75.0, 50.0, 62.5, 50.0]
    );
    assert_eq!(cases[1], cases[3]);
    assert!(
        (cases[0].water_attenuation_db_per_wavelength[0]
            / cases[1].water_attenuation_db_per_wavelength[0]
            - (60.0_f64 / 50.0).sqrt())
        .abs()
            < 1e-14
    );
    assert!(
        (cases[0].additional_fluid_layers[0].attenuation_db_per_wavelength[0]
            / cases[1].additional_fluid_layers[0].attenuation_db_per_wavelength[0]
            - (70.0_f64 / 50.0).powf(-0.25))
        .abs()
            < 1e-14
    );
    assert!(
        (cases[0].bottom_attenuation_db_per_wavelength
            / cases[1].bottom_attenuation_db_per_wavelength
            - (55.0_f64 / 50.0).powf(0.25))
        .abs()
            < 1e-14
    );
}

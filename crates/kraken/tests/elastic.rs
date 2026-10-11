use std::path::Path;

use kraken::{Boundary, Case, ModeSolver, legacy, solve};

fn cases(source: &str, solver: ModeSolver) -> Result<Vec<Case>, kraken::DiagnosticReport> {
    legacy::load_frequency_cases_from_sources(
        source,
        include_str!("fixtures/ElasticHalfBottomN.flp"),
        Path::new("elastic.env"),
        Path::new("elastic.flp"),
        solver,
    )
}

#[test]
fn elastic_materials_are_validated_at_the_case_boundary() {
    let input = cases(
        include_str!("fixtures/ElasticHalfBottomN.env"),
        ModeSolver::Krakenc,
    )
    .unwrap()
    .remove(0)
    .into_definition();
    for (cs, loss) in [
        (0.0, 0.4),
        (-1.0, 0.4),
        (f64::NAN, 0.4),
        (4000.0, 0.4),
        (2000.0, -1.0),
        (2000.0, f64::NAN),
        (2000.0, 55.0),
    ] {
        let mut invalid = input.clone();
        invalid.bottom_boundary = Boundary::ElasticHalfSpace {
            shear_sound_speed_mps: cs,
            shear_attenuation_db_per_wavelength: loss,
        };
        assert!(
            Case::from_definition(invalid)
                .unwrap_err()
                .diagnostics()
                .iter()
                .any(|d| d.field == "bottom_boundary")
        );
    }
    let invalid =
        include_str!("fixtures/ElasticHalfBottomN.env").replace("4000.0 2000.0", "2000.0 2000.0");
    let report = cases(&invalid, ModeSolver::Krakenc).unwrap_err();
    assert!(
        report
            .diagnostics()
            .iter()
            .any(|d| d.field == "bottom_boundary"
                && d.path == Path::new("elastic.env")
                && d.line == 14)
    );
    let invalid = include_str!("fixtures/ElasticHalfBottomN.env")
        .replace("4000.0 2000.0 2.0 0.25 0.4", "4000.0 0.0 2.0 0.25 0.4");
    assert!(
        cases(&invalid, ModeSolver::Krakenc)
            .unwrap_err()
            .to_string()
            .contains("no shear loss for a fluid")
    );
}

#[test]
fn elastic_top_and_unvalidated_combinations_are_explicit() {
    let source = include_str!("fixtures/ElasticHalfTopN.env");
    for solver in [ModeSolver::Kraken, ModeSolver::Krakenc] {
        assert!(cases(source, solver).is_ok());
    }
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/FiniteElasticTopN");
    let mut finite = legacy::load_case(root.with_extension("env"), root.with_extension("flp"))
        .unwrap()
        .into_definition();
    finite.surface_boundary = Boundary::ElasticHalfSpace {
        shear_sound_speed_mps: 2300.0,
        shear_attenuation_db_per_wavelength: 0.5,
    };
    finite.surface_sound_speed_mps = 4500.0;
    assert!(
        Case::from_definition(finite)
            .unwrap_err()
            .to_string()
            .contains("elastic top half-space with finite solids")
    );
    let mut input = cases(source, ModeSolver::Krakenc)
        .unwrap()
        .remove(0)
        .into_definition();
    input.bottom_boundary = Boundary::Reflection(Vec::new());
    assert!(
        Case::from_definition(input)
            .unwrap_err()
            .to_string()
            .contains("smooth V/R/A")
    );
}

#[test]
fn elastic_top_frequency_runs_preserve_order_duplicates_and_reset() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/ElasticHalfTopBroadband");
    for (solver, counts) in [
        (ModeSolver::Kraken, [4, 3, 3, 3]),
        (ModeSolver::Krakenc, [6, 3, 5, 3]),
    ] {
        let cases = legacy::load_field_cases(
            root.with_extension("env"),
            root.with_extension("flp"),
            solver,
        )
        .unwrap();
        assert_eq!(
            cases
                .iter()
                .map(|case| case.profiles()[0].frequency_hz)
                .collect::<Vec<_>>(),
            [50.0, 25.0, 37.5, 25.0]
        );
        let results = kraken::solve_frequencies(&cases)
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(
            results
                .iter()
                .map(|result| result.modes[0].modes.len())
                .collect::<Vec<_>>(),
            counts
        );
        assert_eq!(results[1], results[3]);
        assert_eq!(
            kraken::solve_frequencies(&cases)
                .collect::<Result<Vec<_>, _>>()
                .unwrap(),
            results
        );
    }
}

#[test]
fn real_elastic_top_keeps_pinned_compressional_but_not_shear_loss() {
    let case = cases(
        include_str!("fixtures/ElasticHalfTopN.env"),
        ModeSolver::Kraken,
    )
    .unwrap()
    .remove(0);
    let expected = solve(&case).unwrap();
    let mut input = case.into_definition();
    let Boundary::ElasticHalfSpace {
        shear_attenuation_db_per_wavelength,
        ..
    } = &mut input.surface_boundary
    else {
        unreachable!()
    };
    *shear_attenuation_db_per_wavelength = 0.0;
    assert_eq!(
        solve(&Case::from_definition(input.clone()).unwrap()).unwrap(),
        expected
    );
    input.surface_attenuation_db_per_wavelength = 0.0;
    let without_loss = solve(&Case::from_definition(input).unwrap()).unwrap();
    assert_eq!(without_loss.modes.modes.len(), expected.modes.modes.len());
    assert!(
        without_loss
            .modes
            .modes
            .iter()
            .zip(&expected.modes.modes)
            .any(|(a, b)| (a.attenuation_nepers_per_m - b.attenuation_nepers_per_m).abs() > 1e-8)
    );
}

#[test]
fn shear_power_loss_is_converted_per_frequency_and_keeps_duplicates() {
    let cases = cases(
        include_str!("fixtures/ElasticHalfPower.env"),
        ModeSolver::Krakenc,
    )
    .unwrap();
    assert_eq!(
        cases
            .iter()
            .map(|case| case.frequency_hz)
            .collect::<Vec<_>>(),
        [75.0, 50.0, 62.5, 50.0]
    );
    for (case, expected) in
        cases
            .iter()
            .zip([0.4 * 1.1_f64.powf(0.25), 0.4, 0.4 * 1.1_f64.powf(0.25), 0.4])
    {
        let Boundary::ElasticHalfSpace {
            shear_sound_speed_mps,
            shear_attenuation_db_per_wavelength,
        } = case.bottom_boundary
        else {
            panic!("elastic bottom lost")
        };
        assert!((shear_sound_speed_mps - 2000.0).abs() < 1e-12);
        assert!((shear_attenuation_db_per_wavelength - expected).abs() < 1e-14);
    }
    assert_eq!(cases[1], cases[3]);
}

#[test]
fn kraken_keeps_the_reference_shear_cutoff_and_ignores_elastic_shear_loss() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/OriginalElasticScholte");
    let case = legacy::load_case(root.with_extension("env"), root.with_extension("flp")).unwrap();
    let expected = solve(&case).unwrap();
    let mut input = case.into_definition();
    input.c_high_mps = 4000.0; // Pinned KRAKEN caps elastic cHigh at the shear speed.
    assert_eq!(
        solve(&Case::from_definition(input).unwrap()).unwrap(),
        expected
    );

    let case = cases(
        include_str!("fixtures/ElasticHalfShearOnly.env"),
        ModeSolver::Kraken,
    )
    .unwrap()
    .remove(0);
    let expected = solve(&case).unwrap();
    assert!(
        expected
            .modes
            .modes
            .iter()
            .all(|mode| mode.attenuation_nepers_per_m == 0.0)
    );
    let mut input = case.into_definition();
    if let Boundary::ElasticHalfSpace {
        shear_attenuation_db_per_wavelength,
        ..
    } = &mut input.bottom_boundary
    {
        *shear_attenuation_db_per_wavelength = 0.0;
    }
    assert_eq!(
        solve(&Case::from_definition(input).unwrap()).unwrap(),
        expected
    );
    let complex = cases(
        include_str!("fixtures/ElasticHalfShearOnly.env"),
        ModeSolver::Krakenc,
    )
    .unwrap()
    .remove(0);
    assert!(
        solve(&complex)
            .unwrap()
            .modes
            .modes
            .iter()
            .all(|mode| mode.attenuation_nepers_per_m > 0.0)
    );
}

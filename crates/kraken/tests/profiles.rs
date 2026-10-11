use kraken::{Case, FieldCase, FieldPropagation, ModeAddition, ModeSolver, legacy, solve_field};
use std::path::Path;

fn load(name: &str) -> FieldCase {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name);
    legacy::load_field_cases(
        root.with_extension("env"),
        root.with_extension("flp"),
        ModeSolver::Kraken,
    )
    .unwrap()
    .remove(0)
}

#[test]
fn profile_validation_and_source_blocks_are_stateless() {
    for name in ["ProfilesAd", "ProfilesCm"] {
        let case = load(name);
        let mut ranges = case.ranges_m().to_vec();
        ranges[2] = ranges[1];
        assert!(FieldCase::new(case.profiles().to_vec(), ranges, case.propagation()).is_err());
        assert!(FieldCase::new(Vec::new(), Vec::new(), FieldPropagation::Adiabatic).is_err());
        let baseline = solve_field(&case).unwrap();
        let profiles = case
            .profiles()
            .iter()
            .map(|c| {
                let mut input = c.clone().into_definition();
                input.source_depths_m = vec![75.0, 75.0];
                input.source_pattern = [-180.0, 180.0]
                    .map(|angle_degrees| kraken::SourcePatternPoint {
                        angle_degrees,
                        amplitude: 0.5,
                    })
                    .to_vec();
                Case::from_definition(input).unwrap()
            })
            .collect();
        let doubled =
            FieldCase::new(profiles, case.ranges_m().to_vec(), case.propagation()).unwrap();
        let result = solve_field(&doubled).unwrap();
        assert_eq!(result.modes, baseline.modes);
        let block = baseline.field.pressure.len();
        assert_eq!(
            result.field.pressure[..block],
            baseline
                .field
                .pressure
                .iter()
                .map(|p| p * 0.5)
                .collect::<Vec<_>>()
        );
        assert_eq!(result.field.pressure[block..], baseline.field.pressure);
    }
    // Incoherent AD pressures are compared pointwise by ProfilesAdIncoherent's golden.
    let case = load("ProfilesCm");
    let mut profiles = case.profiles().to_vec();
    let mut input = profiles[3].clone().into_definition();
    input.mode_addition = ModeAddition::Incoherent;
    profiles[3] = Case::from_definition(input).unwrap();
    assert!(FieldCase::new(profiles, case.ranges_m().to_vec(), case.propagation()).is_err());
    // PLeft leaves decay zero for non-A boundaries; do not divide by their zero material speed.
    let mut profiles = case.profiles().to_vec();
    let mut input = profiles[0].clone().into_definition();
    input.bottom_boundary = kraken::BottomBoundary::Vacuum;
    input.bottom_sound_speed_mps = 0.0;
    input.bottom_density_g_cm3 = 0.0;
    profiles[0] = Case::from_definition(input).unwrap();
    let vacuum = FieldCase::new(profiles, case.ranges_m().to_vec(), case.propagation()).unwrap();
    assert!(solve_field(&vacuum).is_ok());
    let profiles = case
        .profiles()
        .iter()
        .map(|c| {
            let mut input = c.clone().into_definition();
            input.mode_sample_depths_m.remove(0);
            Case::from_definition(input).unwrap()
        })
        .collect();
    assert!(FieldCase::new(profiles, case.ranges_m().to_vec(), case.propagation()).is_err());
}

#[test]
#[allow(clippy::cast_possible_truncation)]
fn coupling_rejects_unsupported_fluid_interface_stencils() {
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    for name in ["LayeredFluidThree", "LayeredFluidFractional"] {
        let root = fixtures.join(name);
        let base =
            legacy::load_case(root.with_extension("env"), root.with_extension("flp")).unwrap();
        let mut input = base.clone().into_definition();
        let interfaces: Vec<_> = std::iter::once(base.water_depth_m)
            .chain(
                base.additional_fluid_layers
                    .iter()
                    .take(base.additional_fluid_layers.len() - 1)
                    .map(|layer| layer.bottom_depth_m),
            )
            .map(|depth| f64::from(depth as f32))
            .collect();
        input.mode_sample_depths_m = std::iter::once(base.fluid_top_depth_m())
            .chain(interfaces.iter().copied())
            .chain(std::iter::once(base.fluid_bottom_depth_m()))
            .collect();
        let complete = Case::from_definition(input).unwrap();
        let supported = FieldCase::new(
            vec![base, complete.clone()],
            vec![0.0, 500.0],
            FieldPropagation::Coupled,
        )
        .unwrap();
        assert!(solve_field(&supported).is_ok());
        let mut grids = vec![vec![
            complete.fluid_top_depth_m(),
            complete.fluid_bottom_depth_m(),
        ]];
        for omitted in 1..=interfaces.len() {
            let mut grid = complete.mode_sample_depths_m.clone();
            grid.remove(omitted);
            grids.push(grid);
        }
        let mut input = complete.clone().into_definition();
        input.mode_sample_depths_m = if interfaces.len() == 2 {
            // Two interfaces in one gap or one interior stencil, including an on-grid interface.
            grids.extend([
                vec![0.0, 30.0, 115.0, 140.0],
                vec![0.0, 50.0, 90.0, 115.0, 140.0],
                vec![0.0, 50.0, 105.0, 140.0],
                vec![0.0, 50.0, 70.0, 115.0, 140.0],
            ]);
            vec![0.0, 50.0, 80.0, 95.0, 115.0, 140.0]
        } else {
            vec![0.0, 35.0, 105.0, complete.fluid_bottom_depth_m()]
        };
        // Like Gulf, isolated off-grid interfaces inside interior stencils remain supported.
        let interior = Case::from_definition(input).unwrap();
        let supported = FieldCase::new(
            vec![complete.clone(), interior],
            vec![0.0, 500.0],
            FieldPropagation::Coupled,
        )
        .unwrap();
        assert!(solve_field(&supported).is_ok());
        for grid in grids {
            let mut input = complete.clone().into_definition();
            input.mode_sample_depths_m = grid;
            let coarse = Case::from_definition(input).unwrap();
            for profile in 0..2 {
                let mut profiles = vec![complete.clone(); 2];
                profiles[profile] = coarse.clone();
                assert!(
                    FieldCase::new(
                        profiles.clone(),
                        vec![0.0, 500.0],
                        FieldPropagation::Adiabatic
                    )
                    .is_ok()
                );
                let report = FieldCase::new(profiles, vec![0.0, 500.0], FieldPropagation::Coupled)
                    .unwrap_err();
                assert_eq!(report.diagnostics()[0].code, "KR0201");
                assert_eq!(report.diagnostics()[0].field, "mode_sample_depths_m");
                assert!(
                    report.diagnostics()[0]
                        .message
                        .contains("fluid-interface quadrature stencil")
                );
            }
        }
    }
}

#[test]
fn snapshots_reject_extra_profiles_and_preserve_frequency_order() {
    let env = include_str!("fixtures/ProfilesAd.env");
    let flp = include_str!("fixtures/ProfilesAd.flp");
    let parse = |source: &str, field: &str| {
        legacy::load_field_cases_with_resources(
            source,
            field,
            Path::new("snapshot.env"),
            Path::new("snapshot.flp"),
            ModeSolver::Kraken,
            [None; 3],
            None,
        )
    };
    assert!(parse(env, &flp.replace("4\n0.0 0.8 1.6 2.4", "3\n0.0 0.8 1.6")).is_err());
    assert!(parse(env, &flp.replace("'RAOC'", "'RCOI'")).is_err());
    assert!(
        parse(
            env,
            &flp.replace("4\n0.0 0.8 1.6 2.4", "5\n0.0 0.8 1.6 2.4 3.2")
        )
        .is_err()
    );
    for index in 0..3 {
        let mut tables = [None; 3];
        tables[index] = Some("unused");
        let report = legacy::load_field_cases_with_resources(
            env,
            flp,
            Path::new("snapshot.env"),
            Path::new("snapshot.flp"),
            ModeSolver::Kraken,
            tables,
            None,
        )
        .unwrap_err();
        assert_eq!(report.diagnostics()[0].code, "KR0202");
        assert_eq!(
            report.diagnostics()[0].message,
            "unexpected boundary table snapshot"
        );
    }
    let mut broadband = String::new();
    for block in env.split("'Derived").skip(1) {
        broadband.push_str("'Derived");
        broadband.push_str(&block.replace("'NVN'", "'NVN  B'"));
        broadband.push_str("3\n60.0 50.0 50.0 /\n");
    }
    let cases = parse(&broadband, flp).unwrap();
    assert_eq!(
        cases
            .iter()
            .map(|c| c.profiles()[0].frequency_hz)
            .collect::<Vec<_>>(),
        [60.0, 50.0, 50.0]
    );
    assert!(
        parse(
            &broadband.replacen("60.0 50.0 50.0", "60.0 50.0 51.0", 1),
            flp
        )
        .is_err()
    );
}

#[test]
fn depth_only_ssp_rows_and_zero_lower_bound_keep_physical_modes() {
    let env = include_str!("fixtures/Pekeris.env");
    let flp = include_str!("fixtures/Pekeris.flp");
    let parse = |source: &str| {
        legacy::load_frequency_cases_from_sources(
            source,
            flp,
            Path::new("snapshot.env"),
            Path::new("snapshot.flp"),
            ModeSolver::Kraken,
        )
        .unwrap()
        .remove(0)
    };
    let baseline = parse(env);
    let case = parse(
        &env.replace("100.0 1500.0 /", "100.0 /")
            .replace("1400.0 1700.0", "0.0 1700.0"),
    );
    assert_eq!(case.sound_speed_profile, baseline.sound_speed_profile);
    assert_eq!(
        kraken::solve(&case).unwrap(),
        kraken::solve(&baseline).unwrap()
    );
}

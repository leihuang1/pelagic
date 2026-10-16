//! Test-only reader for pinned little-endian, single-profile fluid .mod/.shd files.
//! These binary formats are reference artifacts, not a production output contract.
use std::fs;
use std::path::{Path, PathBuf};

use kraken::{
    Case, ModeSet, NormalMode, PressureField, SimulationResult,
    legacy::{load_case, load_complex_case, load_frequency_cases},
    solve, solve_complex_modes,
};
use num_complex::Complex64;

const CASES: &[&str] = &[
    "Pekeris",
    "PekerisFiltered",
    "PekerisDense",
    "PekerisDenseLoss",
    "PekerisRefined",
    "PekerisSpline3",
    "PekerisRigid",
    "PekerisRigidLoss",
    "PekerisHard",
    "PekerisHardBoth",
    "PekerisRigidPlane",
    "MunkLossless",
    "MunkBottomLoss",
    "MunkAnalytic",
    "SductTrapped",
    "SductPchip",
    "SductSpline",
];

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

#[test]
fn stratified_trapped_complex_spectra_are_complete() {
    for name in ["SductTrapped", "SductPchip", "SductSpline"] {
        compare_frequencies(
            &fixtures().join(format!("{name}.env")),
            &fixtures().join("golden").join(format!("{name}-krakenc")),
            kraken::ModeSolver::Krakenc,
        );
    }
}

#[test]
fn rough_fluid_interfaces_match_pinned_goldens() {
    for name in [
        "RoughSurface",
        "RoughBottom",
        "RoughInterface",
        "RoughBroadband",
    ] {
        for (engine, solver) in [
            ("kraken", kraken::ModeSolver::Kraken),
            ("krakenc", kraken::ModeSolver::Krakenc),
        ] {
            compare_frequencies(
                &fixtures().join(format!("{name}.env")),
                &fixtures().join("golden").join(format!("{name}-{engine}")),
                solver,
            );
        }
    }
}

#[test]
fn fluid_modes_and_field_match_pinned_goldens() {
    for name in CASES {
        compare(
            &fixtures().join(name).with_extension("env"),
            &fixtures().join("golden").join(name),
        );
    }
}

#[test]
fn complex_fluid_modes_match_pinned_fortran() {
    for (name, golden) in [
        ("PekerisComplex", "PekerisComplex"),
        ("PekerisComplexBlank", "PekerisComplex"),
        ("PekerisComplexSlow", "PekerisComplexSlow"),
        ("PekerisComplexCLow", "PekerisComplexCLow"),
        ("PekerisComplexRefined", "PekerisComplexRefined"),
        ("PekerisComplexGradient", "PekerisComplexGradient"),
        (
            "PekerisComplexReverseGradient",
            "PekerisComplexReverseGradient",
        ),
        ("MunkLeakyPartial", "MunkLeakyPartial"),
        ("MunkLeakyPartialLoss", "MunkLeakyPartialLoss"),
        ("MunkLeakyPartialC", "MunkLeakyPartialC"),
    ] {
        compare_complex(
            &fixtures().join(format!("{name}.env")),
            &fixtures().join("golden").join(golden),
        );
    }
}

#[test]
fn complex_cubic_and_analytic_modes_and_field_match_pinned_goldens() {
    for name in [
        "MunkLeakyPartialP",
        "MunkLeakyPartialS",
        "PekerisComplexSpline3",
        "MunkAnalyticComplex",
    ] {
        let env = fixtures().join(name).with_extension("env");
        compare_complex_field(
            &env,
            &env.with_extension("flp"),
            &fixtures().join("golden").join(name),
        );
    }
}

#[test]
fn water_material_modes_and_field_match_pinned_goldens() {
    for name in [
        "WaterLossN",
        "WaterLossC",
        "WaterLossP",
        "WaterLossS",
        "WaterLossUnitN",
        "WaterLossUnitM",
        "WaterLossUnitF",
        "WaterLossUnitQ",
        "WaterLossUnitL",
        "WaterLossPower",
        "WaterLossThorp",
        "WaterLossFg",
        "WaterLossBio",
        "WaterLossLeaky",
    ] {
        for (engine, solver) in [
            ("kraken", kraken::ModeSolver::Kraken),
            ("krakenc", kraken::ModeSolver::Krakenc),
        ] {
            if name == "WaterLossLeaky" && engine == "kraken" {
                continue;
            }
            compare_frequencies(
                &fixtures().join(name).with_extension("env"),
                &fixtures().join("golden").join(format!("{name}-{engine}")),
                solver,
            );
        }
    }
}

#[test]
fn smooth_boundaries_and_top_tables_match_pinned_goldens() {
    for name in [
        "FluidBoundaryVV",
        "FluidBoundaryVR",
        "FluidBoundaryVA",
        "FluidBoundaryRV",
        "FluidBoundaryRR",
        "FluidBoundaryRA",
        "FluidBoundaryAV",
        "FluidBoundaryAR",
        "FluidBoundaryAA",
        "FluidRigidPlaneLoss",
        "FluidBoundaryAir",
        "FluidTrcN",
        "FluidTrcC",
        "FluidTrcRigid",
    ] {
        for (engine, solver) in [
            ("kraken", kraken::ModeSolver::Kraken),
            ("krakenc", kraken::ModeSolver::Krakenc),
        ] {
            if engine == "kraken" && (name == "FluidBoundaryAir" || name.starts_with("FluidTrc")) {
                continue;
            }
            compare_frequencies(
                &fixtures().join(name).with_extension("env"),
                &fixtures().join("golden").join(format!("{name}-{engine}")),
                solver,
            );
        }
    }
}

#[test]
fn layered_fluid_modes_and_field_match_pinned_goldens() {
    for name in [
        "LayeredFluidN",
        "LayeredFluidC",
        "LayeredFluidP",
        "LayeredFluidS",
        "LayeredBoundaryVV",
        "LayeredBoundaryVR",
        "LayeredBoundaryRV",
        "LayeredBoundaryRR",
        "LayeredBoundaryRA",
        "LayeredBoundaryAV",
        "LayeredBoundaryAR",
        "LayeredBoundaryAA",
        "LayeredFluidThree",
        "LayeredFluidThreeWide",
        "LayeredFluidThreeWideRefined",
        "LayeredFluidPlane",
        "LayeredFluidPower",
        "LayeredFluidBio",
        "LayeredFluidLeaky",
        "LayeredDoubleRefined",
        "OriginalLayeredDouble",
        "LayeredNormalization",
        "LayeredFluidFractional",
    ] {
        for (engine, solver) in [
            ("kraken", kraken::ModeSolver::Kraken),
            ("krakenc", kraken::ModeSolver::Krakenc),
        ] {
            if name == "LayeredFluidLeaky" && engine == "kraken" {
                continue;
            }
            compare_frequencies(
                &fixtures().join(name).with_extension("env"),
                &fixtures().join("golden").join(format!("{name}-{engine}")),
                solver,
            );
        }
    }
}

#[test]
fn finite_elastic_layers_match_pinned_goldens() {
    for name in [
        "FiniteElasticBothC",
        "FiniteElasticBothN",
        "FiniteElasticBothP",
        "FiniteElasticBothS",
        "FiniteElasticBottomC",
        "FiniteElasticBottomN",
        "FiniteElasticBottomP",
        "FiniteElasticBottomS",
        "FiniteElasticPower",
        "FiniteElasticRigid",
        "FiniteElasticShearOnly",
        "FiniteElasticStack",
        "FiniteElasticTopC",
        "FiniteElasticTopN",
        "FiniteElasticTopP",
        "FiniteElasticTopS",
        "FiniteElasticVacuum",
        "FiniteSingleIceC",
        "FiniteSingleIceP",
        "FiniteSingleIceS",
        "FiniteSingleSedimentC",
        "FiniteSingleSedimentP",
        "FiniteSingleSedimentS",
        "OriginalElasticIce",
        "OriginalElasticSediment",
    ] {
        for (engine, solver) in [
            ("kraken", kraken::ModeSolver::Kraken),
            ("krakenc", kraken::ModeSolver::Krakenc),
        ] {
            compare_frequencies(
                &fixtures().join(name).with_extension("env"),
                &fixtures().join("golden").join(format!("{name}-{engine}")),
                solver,
            );
        }
    }
}

#[test]
fn graded_elastic_layers_match_pinned_goldens() {
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
        for (engine, solver) in [
            ("kraken", kraken::ModeSolver::Kraken),
            ("krakenc", kraken::ModeSolver::Krakenc),
        ] {
            compare_frequencies(
                &fixtures().join(name).with_extension("env"),
                &fixtures().join("golden").join(format!("{name}-{engine}")),
                solver,
            );
        }
    }
    compare_frequencies(
        &fixtures().join("GradedElasticTopNRefined.env"),
        &fixtures().join("golden/GradedElasticTopNRefined-kraken"),
        kraken::ModeSolver::Kraken,
    );
}

#[test]
fn elastic_half_spaces_match_pinned_goldens() {
    for name in [
        "ElasticHalfBottomN",
        "ElasticHalfBottomC",
        "ElasticHalfBottomP",
        "ElasticHalfBottomS",
        "ElasticHalfTopN",
        "ElasticHalfTopC",
        "ElasticHalfTopP",
        "ElasticHalfTopS",
        "ElasticHalfBothN",
        "ElasticHalfBothC",
        "ElasticHalfBothP",
        "ElasticHalfBothS",
        "ElasticHalfLeaky",
        "ElasticHalfTopBroadband",
        "ElasticHalfPower",
        "ElasticHalfShearOnly",
        "OriginalElasticScholte",
        "OriginalElasticNormal",
        "OriginalElasticFlused",
    ] {
        for (engine, solver) in [
            ("kraken", kraken::ModeSolver::Kraken),
            ("krakenc", kraken::ModeSolver::Krakenc),
        ] {
            if engine == "kraken" && name == "ElasticHalfLeaky" {
                continue;
            }
            compare_frequencies(
                &fixtures().join(name).with_extension("env"),
                &fixtures().join("golden").join(format!("{name}-{engine}")),
                solver,
            );
        }
    }
}

#[test]
fn tabulated_bottom_modes_and_field_match_pinned_goldens() {
    for name in ["TabRefBrcN", "TabRefBrcC", "TabRefIrcN", "TabRefIrcC"] {
        let env = fixtures().join(name).with_extension("env");
        compare_complex_field(
            &env,
            &env.with_extension("flp"),
            &fixtures().join("golden").join(name),
        );
    }
}

#[test]
#[ignore = "requires a pinned external Fortran reference run"]
fn complex_fluid_matches_fresh_reference() {
    let name = std::env::var("KRAKEN_COMPLEX_CASE").expect("KRAKEN_COMPLEX_CASE is required");
    let reference = PathBuf::from(
        std::env::var_os("KRAKEN_COMPLEX_REFERENCE_ROOT")
            .expect("KRAKEN_COMPLEX_REFERENCE_ROOT is required"),
    );
    let env = fixtures().join(format!("{name}.env"));
    if env.with_extension("flp").is_file() {
        compare_complex_field(&env, &env.with_extension("flp"), &reference);
    } else {
        compare_complex(&env, &reference);
    }
}

#[test]
#[ignore = "requires external KRAKENC .env/.flp snapshots and pinned Fortran modes/FIELD"]
fn complex_original_fluid_matches_fresh_reference() {
    let env = PathBuf::from(
        std::env::var_os("KRAKEN_ORIGINAL_COMPLEX_ENV")
            .expect("KRAKEN_ORIGINAL_COMPLEX_ENV is required"),
    );
    let root = PathBuf::from(
        std::env::var_os("KRAKEN_ORIGINAL_COMPLEX_ROOT")
            .expect("KRAKEN_ORIGINAL_COMPLEX_ROOT is required"),
    );
    compare_complex_field(&env, &env.with_extension("flp"), &root);
}

#[test]
fn complex_line_field_matches_pinned_fortran() {
    for name in ["PekerisComplexBlank", "PekerisComplexRefined"] {
        let env = fixtures().join(format!("{name}.env"));
        let case = load_complex_case(&env, env.with_extension("flp")).unwrap();
        let actual = solve(&case).unwrap();
        let error = compare_field(
            &case,
            &actual.field,
            &Records::read(&fixtures().join("golden").join(name).with_extension("shd")),
        );
        eprintln!(
            "{}: {} KRAKENC line-source pressures, |dp|={error:e}",
            env.display(),
            actual.field.pressure.len()
        );
    }
}

#[test]
fn complex_point_field_matches_pinned_fortran() {
    for name in ["MunkLeakyPartialLoss", "MunkLeakyPartialC"] {
        let env = fixtures().join(format!("{name}.env"));
        compare_complex_field(
            &env,
            &env.with_extension("flp"),
            &fixtures().join("golden").join(name),
        );
    }
}

fn compare_complex_field(env: &Path, flp: &Path, reference: &Path) {
    let case = load_complex_case(env, flp).unwrap();
    let actual = result_for_comparison(&case, 0, 1);
    let modes = Records::read(&reference.with_extension("mod"));
    let field = Records::read(&reference.with_extension("shd"));
    let printed = fs::read_to_string(reference.with_extension("prt")).unwrap();
    let errors = compare_modes(&case, &actual.modes, &modes, &printed);
    let pressure_error = compare_field(&case, &actual.field, &field);
    eprintln!(
        "{}: {} KRAKENC modes, {} pressure samples; errors {errors:?}, |dp|={pressure_error:e}",
        env.display(),
        actual.modes.modes.len(),
        actual.field.pressure.len()
    );
}

fn compare_complex(env: &Path, reference: &Path) {
    compare_complex_with_flp(env, &fixtures().join("Pekeris.flp"), reference);
}

fn compare_complex_with_flp(env: &Path, flp: &Path, reference: &Path) {
    let case = load_complex_case(env, flp).unwrap();
    let actual = solve_complex_modes(&case).unwrap();
    let errors = compare_modes(
        &case,
        &actual,
        &Records::read(&reference.with_extension("mod")),
        &fs::read_to_string(reference.with_extension("prt")).unwrap(),
    );
    eprintln!(
        "{}: {} KRAKENC modes, errors {errors:?}",
        env.display(),
        actual.modes.len()
    );
}

#[test]
fn profile_fields_match_pinned_goldens() {
    for (name, environment) in [
        ("ProfilesAd", "ProfilesAd"),
        ("ProfilesCm", "ProfilesCm"),
        ("ProfilesAdIncoherent", "ProfilesAd"),
    ] {
        compare_profiles(
            &fixtures().join(environment).with_extension("env"),
            &fixtures().join(name).with_extension("flp"),
            &fixtures().join("golden").join(format!("{name}-kraken")),
            kraken::ModeSolver::Kraken,
        );
    }
}

#[test]
fn incoherent_profile_comparator_rejects_zero_pressure() {
    let cases = kraken::legacy::load_field_cases(
        fixtures().join("ProfilesAd.env"),
        fixtures().join("ProfilesAdIncoherent.flp"),
        kraken::ModeSolver::Kraken,
    )
    .unwrap();
    let case = &cases[0].profiles()[0];
    let mut field = kraken::solve_field(&cases[0]).unwrap().field;
    assert!(field.pressure.iter().all(|p| p.im == 0.0 && p.re >= 0.0));
    let reference = Records::read(&fixtures().join("golden/ProfilesAdIncoherent-kraken.shd"));
    compare_field(case, &field, &reference);
    field.pressure.fill(Complex64::new(0.0, 0.0));
    assert!(std::panic::catch_unwind(|| compare_field(case, &field, &reference)).is_err());
}

#[test]
#[ignore = "requires pinned multi-profile modes and FIELD"]
fn profile_fields_match_fresh_reference() {
    let env = PathBuf::from(std::env::var_os("KRAKEN_PROFILE_ENV").unwrap());
    let flp = PathBuf::from(
        std::env::var_os("KRAKEN_PROFILE_FLP")
            .unwrap_or_else(|| env.with_extension("flp").into_os_string()),
    );
    let root = PathBuf::from(std::env::var_os("KRAKEN_PROFILE_ROOT").unwrap());
    let solver = match std::env::var("KRAKEN_PROFILE_SOLVER")
        .as_deref()
        .unwrap_or("kraken")
    {
        "kraken" => kraken::ModeSolver::Kraken,
        "krakenc" => kraken::ModeSolver::Krakenc,
        other => panic!("unknown reference solver: {other}"),
    };
    compare_profiles(&env, &flp, &root, solver);
}

#[test]
fn elastic_warnings_do_not_split_json_profile_headers() {
    let env = fixtures().join("ElasticHalfBothS.env");
    let cases = kraken::legacy::load_field_cases(
        &env,
        env.with_extension("flp"),
        kraken::ModeSolver::Kraken,
    )
    .unwrap();
    let document = kraken::json::export_case_document(&cases).unwrap();
    let restored =
        kraken::json::load_case_document(&serde_json::to_vec(&document).unwrap()).unwrap();
    assert_eq!(restored, cases);
    compare_profile_cases(
        &restored,
        &env,
        &fixtures().join("golden/ElasticHalfBothS-kraken"),
    );
}

#[test]
#[ignore = "requires exported JSON, legacy inputs and pinned Fortran modes/FIELD"]
fn json_fields_match_fresh_reference() {
    let input = PathBuf::from(std::env::var_os("KRAKEN_JSON_INPUT").unwrap());
    let env = PathBuf::from(std::env::var_os("KRAKEN_JSON_ENV").unwrap());
    let root = PathBuf::from(std::env::var_os("KRAKEN_JSON_REFERENCE_ROOT").unwrap());
    let cases = kraken::json::load_case_document_named(&fs::read(&input).unwrap(), &input).unwrap();
    let legacy = kraken::legacy::load_field_cases(
        &env,
        env.with_extension("flp"),
        cases[0].profiles()[0].mode_solver,
    )
    .unwrap();
    assert_eq!(
        cases, legacy,
        "JSON must preserve the entire accepted legacy definition"
    );
    compare_profile_cases(&cases, &input, &root);
}

fn compare_profiles(env: &Path, flp: &Path, root: &Path, solver: kraken::ModeSolver) {
    let cases = kraken::legacy::load_field_cases(env, flp, solver).unwrap();
    compare_profile_cases(&cases, env, root);
}

fn compare_profile_cases(cases: &[kraken::FieldCase], input: &Path, root: &Path) {
    let solver = cases[0].profiles()[0].mode_solver;
    let file = Records::read(&root.with_extension("mod"));
    assert_eq!(
        count(file.record(0), 84),
        cases.len(),
        "mode frequency count"
    );
    let printed = fs::read_to_string(root.with_extension("prt")).unwrap();
    let blocks: Vec<_> = printed
        .split(if solver == kraken::ModeSolver::Kraken {
            "\n KRAKEN-"
        } else {
            "\n KRAKENC-"
        })
        .skip(1)
        .collect();
    assert_eq!(blocks.len(), cases[0].profiles().len());
    let mut offset = 0;
    let mut references = Vec::new();
    for block in &blocks {
        let mut after_profile = offset + 5;
        for _ in 0..cases.len() {
            let m = count(file.record(after_profile), 0);
            after_profile += 2 + m + m.div_ceil(file.record_bytes / 8);
        }
        references.push((
            Records {
                bytes: file.bytes[offset * file.record_bytes..after_profile * file.record_bytes]
                    .to_vec(),
                record_bytes: file.record_bytes,
            },
            *block,
        ));
        offset = after_profile;
    }
    // A shrinking real-elastic spectrum can leave old first-mesh shape records
    // after the last declared frequency; they are not additional modes.
    if cases[0].profiles().len() == 1
        && solver == kraken::ModeSolver::Kraken
        && (!cases[0].profiles()[0].top_elastic_layers.is_empty()
            || !cases[0].profiles()[0].bottom_elastic_layers.is_empty()
            || matches!(
                cases[0].profiles()[0].surface_boundary,
                kraken::Boundary::ElasticHalfSpace { .. }
            ))
    {
        references[0]
            .0
            .bytes
            .extend_from_slice(&file.bytes[offset * file.record_bytes..]);
    } else {
        assert_eq!(offset, file.len());
    }
    let shd = Records::read(&root.with_extension("shd"));
    assert_eq!(
        count(shd.record(2), 0),
        cases.len(),
        "FIELD frequency count"
    );
    let mut api = kraken::solve_frequencies(cases);
    let hdf5 = std::env::var_os("KRAKEN_HDF5_RESULT").is_some();
    for (frequency, case) in cases.iter().enumerate() {
        let result = if hdf5 {
            profile_result_for_comparison(case, frequency, cases.len())
        } else {
            api.next().unwrap().unwrap()
        };
        for (index, (profile, mode)) in case.profiles().iter().zip(&result.modes).enumerate() {
            let errors = compare_modes_at(
                profile,
                mode,
                &references[index].0,
                references[index].1,
                frequency,
            );
            eprintln!("profile {index}: {} modes; {errors:?}", mode.modes.len());
        }
        let dp = compare_field_at(&case.profiles()[0], &result.field, &shd, frequency);
        eprintln!(
            "{}: {} profiles / {} pressures; |dp|={dp:e}",
            input.display(),
            result.modes.len(),
            result.field.pressure.len()
        );
    }
}

fn profile_result_for_comparison(
    case: &kraken::FieldCase,
    index: usize,
    frequencies: usize,
) -> kraken::ProfileSimulationResult {
    let Some(path) = std::env::var_os("KRAKEN_HDF5_RESULT") else {
        return kraken::solve_field(case).unwrap();
    };
    let first = result_for_comparison(&case.profiles()[0], index, frequencies);
    if case.profiles().len() == 1 {
        return kraken::ProfileSimulationResult {
            modes: vec![first.modes],
            field: first.field,
        };
    }
    let file = hdf5::File::open(path).unwrap();
    let group = file.group(&format!("frequencies/{index}")).unwrap();
    assert_eq!(
        group
            .attr("profile_count")
            .unwrap()
            .read_scalar::<u64>()
            .unwrap(),
        case.profiles().len() as u64
    );
    assert_eq!(
        hdf5_attribute(&group, "field_propagation"),
        match case.propagation() {
            kraken::FieldPropagation::RangeIndependent => "range_independent",
            kraken::FieldPropagation::Adiabatic => "adiabatic",
            kraken::FieldPropagation::Coupled => "coupled",
        }
    );
    assert_eq!(
        hdf5_data::<f64>(&group, "profile_range_m", &[case.profiles().len()]),
        case.ranges_m()
    );
    let profiles = group.group("profiles").unwrap();
    assert_eq!(
        profiles.member_names().unwrap().len(),
        case.profiles().len()
    );
    let modes = case
        .profiles()
        .iter()
        .enumerate()
        .map(|(i, profile)| {
            let child = profiles.group(&i.to_string()).unwrap();
            assert_eq!(hdf5_attribute(&child, "title"), profile.title);
            close(
                child.attr("range_m").unwrap().read_scalar::<f64>().unwrap(),
                case.ranges_m()[i],
                0.0,
                "profile range",
            );
            compare_finite_elastic_hdf5(&child, profile);
            let m = hdf5_modes(
                &child.group("modes").unwrap(),
                profile.frequency_hz,
                profile.mode_sample_depths_m.len(),
            );
            if i == 0 {
                assert_eq!(m, first.modes);
            }
            m
        })
        .collect();
    kraken::ProfileSimulationResult {
        modes,
        field: first.field,
    }
}

#[test]
fn single_profile_field_extensions_match_pinned_goldens() {
    for (name, solver) in [
        ("FieldScaled", kraken::ModeSolver::Kraken),
        ("FieldPattern", kraken::ModeSolver::Krakenc),
        ("FieldIncoherent", kraken::ModeSolver::Krakenc),
    ] {
        compare_frequencies(
            &fixtures().join(name).with_extension("env"),
            &fixtures().join("golden").join(format!(
                "{name}-{}",
                if solver == kraken::ModeSolver::Kraken {
                    "kraken"
                } else {
                    "krakenc"
                }
            )),
            solver,
        );
    }
}

#[test]
fn multifrequency_modes_and_field_match_pinned_goldens() {
    for (name, solver) in [
        ("PekerisBroadband", kraken::ModeSolver::Kraken),
        ("PekerisComplexBroadband", kraken::ModeSolver::Krakenc),
        ("MunkLeakyPchipBroadband", kraken::ModeSolver::Krakenc),
    ] {
        compare_frequencies(
            &fixtures().join(name).with_extension("env"),
            &fixtures().join("golden").join(name),
            solver,
        );
    }
}

#[test]
#[ignore = "requires pinned multi-frequency Fortran modes and FIELD"]
fn multifrequency_fluid_matches_fresh_reference() {
    let env = PathBuf::from(
        std::env::var_os("KRAKEN_DIFFERENTIAL_ENV").expect("KRAKEN_DIFFERENTIAL_ENV is required"),
    );
    let reference = PathBuf::from(
        std::env::var_os("KRAKEN_DIFFERENTIAL_ROOT").expect("KRAKEN_DIFFERENTIAL_ROOT is required"),
    );
    let solver = if std::env::var("KRAKEN_FREQUENCY_SOLVER").as_deref() == Ok("krakenc") {
        kraken::ModeSolver::Krakenc
    } else {
        kraken::ModeSolver::Kraken
    };
    compare_frequencies(&env, &reference, solver);
}

#[test]
#[ignore = "requires a pinned external Fortran reference run"]
fn fluid_matches_fresh_reference() {
    let env =
        std::env::var_os("KRAKEN_DIFFERENTIAL_ENV").expect("KRAKEN_DIFFERENTIAL_ENV is required");
    let root =
        std::env::var_os("KRAKEN_DIFFERENTIAL_ROOT").expect("KRAKEN_DIFFERENTIAL_ROOT is required");
    compare(Path::new(&env), Path::new(&root));
}

fn compare(env: &Path, reference: &Path) {
    let case = load_case(env, env.with_extension("flp")).unwrap();
    let actual = result_for_comparison(&case, 0, 1);
    let modes = Records::read(&reference.with_extension("mod"));
    let field = Records::read(&reference.with_extension("shd"));
    let printed = fs::read_to_string(reference.with_extension("prt")).unwrap();
    let (k_error, loss_error, binary_loss_error, shape_error) =
        compare_modes(&case, &actual.modes, &modes, &printed);
    let pressure_error = compare_field(&case, &actual.field, &field);
    eprintln!(
        "{}: {} modes, {} pressure samples; max |dk|={k_error:e}, |dalpha(.prt)|={loss_error:e}, |dalpha(.mod)|={binary_loss_error:e}, |dphi|={shape_error:e}, |dp|={pressure_error:e}",
        env.display(),
        actual.modes.modes.len(),
        actual.field.pressure.len()
    );
}

fn compare_frequencies(env: &Path, reference: &Path, solver: kraken::ModeSolver) {
    let cases = kraken::legacy::load_field_cases(env, env.with_extension("flp"), solver).unwrap();
    let modes = Records::read(&reference.with_extension("mod"));
    let field = Records::read(&reference.with_extension("shd"));
    let printed = fs::read_to_string(reference.with_extension("prt")).unwrap();
    assert_eq!(
        count(modes.record(0), 84),
        cases.len(),
        "mode frequency count"
    );
    assert_eq!(
        count(field.record(2), 0),
        cases.len(),
        "FIELD frequency count"
    );
    let mut api = kraken::solve_frequencies(&cases);
    let hdf5 = std::env::var_os("KRAKEN_HDF5_RESULT").is_some();
    for (index, sequence) in cases.iter().enumerate() {
        assert_eq!(sequence.profiles().len(), 1);
        let case = &sequence.profiles()[0];
        let result = if hdf5 {
            result_for_comparison(case, index, cases.len())
        } else {
            let result = api.next().unwrap().unwrap();
            SimulationResult {
                modes: result.modes.into_iter().next().unwrap(),
                field: result.field,
            }
        };
        let errors = compare_modes_at(case, &result.modes, &modes, &printed, index);
        let pressure_error = compare_field_at(case, &result.field, &field, index);
        eprintln!(
            "{}: {} Hz, {} modes, {} pressures; errors {errors:?}, |dp|={pressure_error:e}",
            env.display(),
            case.frequency_hz,
            result.modes.modes.len(),
            result.field.pressure.len()
        );
    }
}

// Optional CLI-produced HDF5 input exercises the same strict Fortran comparator,
// rather than trusting only the solver result before serialization.
#[allow(clippy::too_many_lines)]
fn result_for_comparison(case: &Case, index: usize, frequencies: usize) -> SimulationResult {
    let Some(path) = std::env::var_os("KRAKEN_HDF5_RESULT") else {
        return solve(case).unwrap();
    };
    let file = hdf5::File::open(path).unwrap();
    assert_eq!(hdf5_attribute(&file, "schema_name"), "kraken");
    assert_eq!(
        file.attr("schema_version")
            .unwrap()
            .read_scalar::<u32>()
            .unwrap(),
        1
    );
    assert_eq!(
        file.attr("frequency_count")
            .unwrap()
            .read_scalar::<u64>()
            .unwrap(),
        frequencies as u64
    );
    assert_eq!(
        hdf5_attribute(&file, "solver"),
        match case.mode_solver {
            kraken::ModeSolver::Kraken => "kraken",
            kraken::ModeSolver::Krakenc => "krakenc",
        }
    );
    assert_eq!(
        file.group("frequencies")
            .unwrap()
            .member_names()
            .unwrap()
            .len(),
        frequencies
    );
    let frequency_vector = hdf5_data::<f64>(&file, "frequency_hz", &[frequencies]);
    close(
        frequency_vector[index],
        case.frequency_hz,
        0.0,
        "HDF5 frequency",
    );
    let group = file.group(&format!("frequencies/{index}")).unwrap();
    compare_elastic_hdf5_materials(&group, case);
    compare_finite_elastic_hdf5(&group, case);
    for interface in 0..=case.additional_fluid_layers.len() {
        let layer = group.group(&format!("media/{interface}")).unwrap();
        for (name, index) in [
            ("top_roughness_m", interface),
            ("bottom_roughness_m", interface + 1),
        ] {
            assert_eq!(
                layer.attr(name).unwrap().read_scalar::<f64>().unwrap(),
                case.interface_roughness_m
                    .get(index)
                    .copied()
                    .unwrap_or(0.0)
            );
        }
    }
    assert_eq!(
        hdf5_attribute(&group, "source_geometry"),
        match case.source_geometry {
            kraken::SourceGeometry::Line => "line",
            kraken::SourceGeometry::Point => "point",
            kraken::SourceGeometry::ScaledCylindrical => "scaled_cylindrical",
        }
    );
    assert_eq!(
        hdf5_attribute(&group, "mode_addition"),
        match case.mode_addition {
            kraken::ModeAddition::Coherent => "coherent",
            kraken::ModeAddition::Incoherent => "incoherent",
        }
    );
    assert_eq!(
        hdf5_attribute(&group, "source_pattern"),
        if case.source_pattern.is_empty() {
            "omnidirectional"
        } else {
            "tabulated"
        }
    );
    assert_eq!(
        group
            .attr("source_pattern_point_count")
            .unwrap()
            .read_scalar::<u64>()
            .unwrap(),
        case.source_pattern.len() as u64
    );
    let frequency_hz = group
        .attr("frequency_hz")
        .unwrap()
        .read_scalar::<f64>()
        .unwrap();
    let modes = hdf5_modes(
        &group.group("modes").unwrap(),
        frequency_hz,
        case.mode_sample_depths_m.len(),
    );
    let field = group.group("field").unwrap();
    assert_eq!(
        hdf5_attribute(&field, "pressure_axis_order"),
        "source_depth,receiver_depth,receiver_range"
    );
    let sources = case.source_depths_m.len();
    let depths = case.receiver_depths_m.len();
    let ranges = case.receiver_ranges_m.len();
    let real = hdf5_data::<f32>(&field, "pressure_real", &[sources, depths, ranges]);
    let imaginary = hdf5_data::<f32>(&field, "pressure_imaginary", &[sources, depths, ranges]);
    SimulationResult {
        modes,
        field: PressureField {
            source_depths_m: hdf5_data(&field, "source_depth_m", &[sources]),
            receiver_depths_m: hdf5_data(&field, "receiver_depth_m", &[depths]),
            receiver_ranges_m: hdf5_data(&field, "receiver_range_m", &[ranges]),
            receiver_offsets_m: hdf5_data(&field, "receiver_offset_m", &[depths]),
            pressure: real
                .into_iter()
                .zip(imaginary)
                .map(|(r, i)| Complex64::new(f64::from(r), f64::from(i)))
                .collect(),
        },
    }
}

fn compare_elastic_hdf5_materials(group: &hdf5::Group, case: &Case) {
    for (name, boundary, cp, density, loss) in [
        (
            "surface",
            &case.surface_boundary,
            case.surface_sound_speed_mps,
            case.surface_density_g_cm3,
            case.surface_attenuation_db_per_wavelength,
        ),
        (
            "bottom",
            &case.bottom_boundary,
            case.bottom_sound_speed_mps,
            case.bottom_density_g_cm3,
            case.bottom_attenuation_db_per_wavelength,
        ),
    ] {
        if let kraken::Boundary::ElasticHalfSpace {
            shear_sound_speed_mps,
            shear_attenuation_db_per_wavelength,
        } = boundary
        {
            assert_eq!(
                hdf5_attribute(group, &format!("{name}_half_space_material")),
                "elastic"
            );
            assert_eq!(
                hdf5_attribute(group, &format!("{name}_elastic_attenuation_model")),
                if case.mode_solver == kraken::ModeSolver::Kraken {
                    "reference_real"
                } else {
                    "complex"
                }
            );
            for (attribute, value) in [
                ("sound_speed_mps", cp),
                ("density_g_cm3", density),
                ("attenuation_db_per_wavelength", loss),
                ("shear_sound_speed_mps", *shear_sound_speed_mps),
                (
                    "shear_attenuation_db_per_wavelength",
                    *shear_attenuation_db_per_wavelength,
                ),
            ] {
                close(
                    group
                        .attr(&format!("{name}_{attribute}"))
                        .unwrap()
                        .read_scalar::<f64>()
                        .unwrap(),
                    value,
                    0.0,
                    "HDF5 elastic half-space material",
                );
            }
        }
    }
}

#[allow(clippy::too_many_lines)] // Compare the complete finite-elastic product together.
fn compare_finite_elastic_hdf5(group: &hdf5::Group, case: &Case) {
    let count = case.top_elastic_layers.len() + case.bottom_elastic_layers.len();
    if count == 0 {
        return;
    }
    assert_eq!(
        group
            .attr("finite_elastic_layer_count")
            .unwrap()
            .read_scalar::<u64>()
            .unwrap(),
        count as u64
    );
    let media = group.group("elastic_media").unwrap();
    for (name, materials, mut top) in [
        ("top", &case.top_elastic_layers, 0.0),
        (
            "bottom",
            &case.bottom_elastic_layers,
            case.fluid_bottom_depth_m(),
        ),
    ] {
        let side = media.group(name).unwrap();
        assert_eq!(side.member_names().unwrap().len(), materials.len());
        for (index, material) in materials.iter().enumerate() {
            let layer = side.group(&index.to_string()).unwrap();
            assert_eq!(hdf5_attribute(&layer, "material"), "elastic");
            assert_eq!(
                hdf5_attribute(&layer, "attenuation_model"),
                if case.mode_solver == kraken::ModeSolver::Kraken {
                    "reference_real_stiffness"
                } else {
                    "complex"
                }
            );
            assert_eq!(
                layer
                    .attr("requested_mesh_points")
                    .unwrap()
                    .read_scalar::<u64>()
                    .unwrap(),
                material.mesh_points as u64
            );
            for (attribute, value) in [
                ("top_depth_m", top),
                ("bottom_depth_m", material.bottom_depth_m),
                (
                    "compressional_sound_speed_mps",
                    material.compressional_sound_speed_mps,
                ),
                ("shear_sound_speed_mps", material.shear_sound_speed_mps),
                ("density_g_cm3", material.density_g_cm3),
                (
                    "compressional_attenuation_db_per_wavelength",
                    material.compressional_attenuation_db_per_wavelength,
                ),
                (
                    "shear_attenuation_db_per_wavelength",
                    material.shear_attenuation_db_per_wavelength,
                ),
            ] {
                let attr = layer.attr(attribute).unwrap();
                assert!(attr.dtype().unwrap().is::<f64>());
                close(
                    attr.read_scalar::<f64>().unwrap(),
                    value,
                    0.0,
                    "HDF5 finite elastic material",
                );
            }
            let points = &material.material_profile;
            assert_eq!(
                layer
                    .attr("material_profile_point_count")
                    .unwrap()
                    .read_scalar::<u64>()
                    .unwrap(),
                points.len() as u64
            );
            if points.is_empty() {
                assert!(!layer.link_exists("material_profile"));
            } else {
                let profile = layer.group("material_profile").unwrap();
                for (name, expected) in [
                    (
                        "depth_m",
                        points.iter().map(|p| p.depth_m).collect::<Vec<_>>(),
                    ),
                    (
                        "compressional_sound_speed_mps",
                        points
                            .iter()
                            .map(|p| p.compressional_sound_speed_mps)
                            .collect(),
                    ),
                    (
                        "shear_sound_speed_mps",
                        points.iter().map(|p| p.shear_sound_speed_mps).collect(),
                    ),
                    (
                        "density_g_cm3",
                        points.iter().map(|p| p.density_g_cm3).collect(),
                    ),
                    (
                        "compressional_attenuation_db_per_wavelength",
                        points
                            .iter()
                            .map(|p| p.compressional_attenuation_db_per_wavelength)
                            .collect(),
                    ),
                    (
                        "shear_attenuation_db_per_wavelength",
                        points
                            .iter()
                            .map(|p| p.shear_attenuation_db_per_wavelength)
                            .collect(),
                    ),
                ] {
                    assert_eq!(hdf5_data::<f64>(&profile, name, &[points.len()]), expected);
                }
            }
            top = material.bottom_depth_m;
        }
    }
    close(
        group
            .group("media/0")
            .unwrap()
            .attr("top_depth_m")
            .unwrap()
            .read_scalar::<f64>()
            .unwrap(),
        case.fluid_top_depth_m(),
        0.0,
        "HDF5 first fluid top",
    );
}

fn hdf5_modes(group: &hdf5::Group, frequency_hz: f64, depths: usize) -> ModeSet {
    assert_eq!(
        hdf5_attribute(group, "eigenfunction_axis_order"),
        "mode,sample_depth"
    );
    let count = group.dataset("horizontal_wavenumber_real").unwrap().size();
    let real = hdf5_data::<f64>(group, "horizontal_wavenumber_real", &[count]);
    let imaginary = hdf5_data::<f64>(group, "horizontal_wavenumber_imaginary", &[count]);
    let phase = hdf5_data::<f64>(group, "phase_speed_mps", &[count]);
    let speed = hdf5_data::<f64>(group, "group_speed_mps", &[count]);
    let attenuation = hdf5_data::<f64>(group, "attenuation_nepers_per_m", &[count]);
    let shape_real = hdf5_data::<f64>(group, "eigenfunction_real", &[count, depths]);
    let shape_imaginary = hdf5_data::<f64>(group, "eigenfunction_imaginary", &[count, depths]);
    ModeSet {
        frequency_hz,
        sampled_depths_m: hdf5_data(group, "sample_depth_m", &[depths]),
        modes: (0..count)
            .map(|index| NormalMode {
                horizontal_wavenumber_rad_per_m: Complex64::new(real[index], imaginary[index]),
                phase_speed_mps: phase[index],
                group_speed_mps: speed[index],
                attenuation_nepers_per_m: attenuation[index],
                eigenfunction: (index * depths..(index + 1) * depths)
                    .map(|sample| Complex64::new(shape_real[sample], shape_imaginary[sample]))
                    .collect(),
            })
            .collect(),
    }
}

fn hdf5_data<T: hdf5::H5Type>(group: &hdf5::Group, name: &str, shape: &[usize]) -> Vec<T> {
    let dataset = group.dataset(name).unwrap();
    assert_eq!(dataset.shape(), shape, "HDF5 {name} shape");
    assert!(dataset.dtype().unwrap().is::<T>(), "HDF5 {name} datatype");
    dataset.read_raw().unwrap()
}

fn hdf5_attribute(location: &hdf5::Location, name: &str) -> String {
    location
        .attr(name)
        .unwrap()
        .read_scalar::<hdf5::types::VarLenUnicode>()
        .unwrap()
        .as_str()
        .to_owned()
}

fn close(actual: f64, expected: f64, tolerance: f64, field: &str) -> f64 {
    let error = (actual - expected).abs();
    assert!(
        actual.is_finite() && expected.is_finite() && error <= tolerance,
        "{field}: actual {actual:e}, reference {expected:e}, error {error:e} > {tolerance:e}"
    );
    error
}

fn complex_close(actual: Complex64, expected: Complex64, tolerance: f64, field: &str) -> f64 {
    close((actual - expected).norm(), 0.0, tolerance, field)
}

fn compare_modes(
    case: &Case,
    actual: &ModeSet,
    file: &Records,
    printed: &str,
) -> (f64, f64, f64, f64) {
    assert_eq!(count(file.record(0), 84), 1, "mode frequency count");
    compare_modes_at(case, actual, file, printed, 0)
}

#[allow(clippy::cast_possible_truncation, clippy::too_many_lines)]
fn compare_modes_at(
    case: &Case,
    actual: &ModeSet,
    file: &Records,
    printed: &str,
    frequency_index: usize,
) -> (f64, f64, f64, f64) {
    let header = file.record(0);
    assert!(frequency_index < count(header, 84));
    close(
        actual.frequency_hz,
        case.frequency_hz,
        0.0,
        "result frequency",
    );
    let layer_count = 1 + case.additional_fluid_layers.len();
    assert_eq!(count(header, 88), layer_count, "medium count");
    assert_eq!(
        count(header, 92),
        actual.sampled_depths_m.len(),
        "mode depth count"
    );
    assert_eq!(
        count(header, 96),
        actual.sampled_depths_m.len(),
        "fluid mode shape size"
    );
    let mut top = case.fluid_top_depth_m();
    for medium in 0..layer_count {
        assert!(
            (10..=1_000_000).contains(&count(file.record(1), 12 * medium)),
            "medium mesh intervals"
        );
        assert_eq!(
            &file.record(1)[12 * medium + 4..12 * medium + 12],
            b"ACOUSTIC",
            "fluid material"
        );
        let density = if medium == 0 {
            case.water_density_g_cm3
        } else {
            case.additional_fluid_layers[medium - 1].density_g_cm3
        };
        close(
            single(file.record(2), 8 * medium),
            f64::from(top as f32),
            0.0,
            "medium top depth",
        );
        close(
            single(file.record(2), 8 * medium + 4),
            f64::from(density as f32),
            0.0,
            "medium density",
        );
        top = if medium == 0 {
            case.water_depth_m
        } else {
            case.additional_fluid_layers[medium - 1].bottom_depth_m
        };
    }
    close(
        double(file.record(3), 8 * frequency_index),
        case.frequency_hz,
        1e-12,
        "mode frequency",
    );
    for (index, &depth) in actual.sampled_depths_m.iter().enumerate() {
        close(
            f64::from(depth as f32),
            single(file.record(4), 4 * index),
            0.0,
            "mode depth",
        );
    }
    let modes_per_record = file.record_bytes / 8;
    let mut first = 5;
    for _ in 0..frequency_index {
        let modes = count(file.record(first), 0);
        first += 2 + modes + modes.div_ceil(modes_per_record);
    }
    let mode_count = count(file.record(first), 0);
    assert_eq!(mode_count, actual.modes.len(), "mode count");
    for (offset, boundary, cp, density, loss, depth) in [
        (
            0,
            &case.surface_boundary,
            case.surface_sound_speed_mps,
            case.surface_density_g_cm3,
            case.surface_attenuation_db_per_wavelength,
            0.0,
        ),
        (
            25,
            &case.bottom_boundary,
            case.bottom_sound_speed_mps,
            case.bottom_density_g_cm3,
            case.bottom_attenuation_db_per_wavelength,
            case.total_depth_m(),
        ),
    ] {
        if let kraken::Boundary::ElasticHalfSpace {
            shear_sound_speed_mps,
            shear_attenuation_db_per_wavelength,
        } = boundary
        {
            let record = file.record(first + 1);
            assert_eq!(record[offset], b'A', "elastic half-space boundary");
            for (at, speed, attenuation) in [
                (offset + 1, cp, loss),
                (
                    offset + 9,
                    *shear_sound_speed_mps,
                    *shear_attenuation_db_per_wavelength,
                ),
            ] {
                complex_close(
                    complex(record, at),
                    Complex64::new(
                        f64::from(speed as f32),
                        f64::from(
                            (attenuation * speed / (8.685_889_6 * 2.0 * std::f64::consts::PI))
                                as f32,
                        ),
                    ),
                    0.0,
                    "elastic half-space complex speed",
                );
            }
            close(
                single(record, offset + 17),
                f64::from(density as f32),
                0.0,
                "elastic half-space density",
            );
            close(
                single(record, offset + 21),
                f64::from(depth as f32),
                0.0,
                "elastic half-space depth",
            );
        }
    }
    if frequency_index + 1 == count(header, 84) {
        let records = first + 2 + mode_count + mode_count.div_ceil(modes_per_record);
        if case.mode_solver == kraken::ModeSolver::Kraken
            && (!case.top_elastic_layers.is_empty()
                || !case.bottom_elastic_layers.is_empty()
                || matches!(
                    case.surface_boundary,
                    kraken::Boundary::ElasticHalfSpace { .. }
                ))
        {
            // WriteMode rewrites M/k after refinement but does not truncate
            // unused first-mesh shapes when Solve2 or MINLOC reduces M.
            let limit =
                if case.top_elastic_layers.is_empty() && case.bottom_elastic_layers.is_empty() {
                    // Solve1 can count at most one sign change per acoustic interval,
                    // two elastic boundaries and the final dispersion sign.
                    (0..layer_count)
                        .map(|i| count(file.record(1), 12 * i))
                        .sum::<usize>()
                        + 3
                } else {
                    3000 // Solve2's initial bound when finite solids bypass Solve1.
                };
            assert!(file.len() >= records, "missing declared mode records");
            assert!(
                file.len() <= first + 2 + limit + limit.div_ceil(modes_per_record),
                "first-mesh search record limit"
            );
        } else {
            assert_eq!(file.len(), records, "mode file record count");
        }
    }

    // .prt preserves extrapolated k to ten decimal places; .mod stores complex32.
    let table = printed
        .split("Group Speed\n")
        .nth(frequency_index + 1)
        .expect("modal print table");
    let rows: Vec<_> = table
        .lines()
        .skip(1)
        .take_while(|line| {
            line.split_whitespace()
                .next()
                .is_some_and(|word| word.parse::<usize>().is_ok())
        })
        .collect();
    let print_stride = (mode_count / 30).max(1);
    assert_eq!(
        rows.len(),
        mode_count.div_ceil(print_stride),
        "printed mode count"
    );
    let mut max_k = 0.0_f64;
    let mut max_loss = 0.0_f64;
    let mut max_binary_loss = 0.0_f64;
    let mut max_shape = 0.0_f64;
    for (row_index, row) in rows.into_iter().enumerate() {
        let index = row_index * print_stride;
        let mode = &actual.modes[index];
        let columns: Vec<_> = row.split_whitespace().collect();
        assert_eq!(columns.len(), 5, "modal print columns");
        assert_eq!(columns[0].parse::<usize>().unwrap(), index + 1);
        let values: Vec<f64> = columns[1..]
            .iter()
            .map(|value| value.parse().unwrap())
            .collect();
        max_k = max_k.max(close(
            mode.horizontal_wavenumber_rad_per_m.re,
            values[0],
            5e-10,
            "wavenumber (.prt)",
        ));
        // Fortran prints signed Im(k) at G10.2; the public attenuation is -Im(k).
        // Check to half of the last printed decimal place, not an arbitrary loose tolerance.
        let attenuation_tolerance = if mode.attenuation_nepers_per_m == 0.0 {
            1e-12
        } else {
            printed_loss_tolerance(columns[2])
        };
        max_loss = max_loss.max(close(
            -mode.attenuation_nepers_per_m,
            values[1],
            attenuation_tolerance.max(1e-12),
            "attenuation (.prt)",
        ));
        close(mode.phase_speed_mps, values[2], 5e-6, "phase speed");
        close(mode.group_speed_mps, values[3], 0.005, "group speed");
    }
    for (index, mode) in actual.modes.iter().enumerate() {
        let k_record = file.record(first + 2 + mode_count + index / modes_per_record);
        let stored = complex(k_record, 8 * (index % modes_per_record));
        close(
            f64::from(mode.horizontal_wavenumber_rad_per_m.re as f32),
            stored.re,
            0.0,
            &format!("real wavenumber (.mod) mode {}", index + 1),
        );
        // Loss depends on the first-mesh root; a different root rounding can
        // shift small imaginary parts even when the extrapolated real k agrees.
        max_binary_loss = max_binary_loss.max(close(
            f64::from(mode.horizontal_wavenumber_rad_per_m.im as f32),
            stored.im,
            if case.top_elastic_layers.iter().chain(&case.bottom_elastic_layers).any(|layer|
                layer.compressional_attenuation_db_per_wavelength > 0.0 || layer.shear_attenuation_db_per_wavelength > 0.0)
                || case.interface_roughness_m.iter().any(|&sigma| sigma > 0.0)
                || case.bottom_attenuation_db_per_wavelength > 0.0
                || case.surface_attenuation_db_per_wavelength > 0.0
                || [&case.surface_boundary, &case.bottom_boundary].iter().any(|boundary| matches!(boundary,
                    kraken::Boundary::ElasticHalfSpace { shear_attenuation_db_per_wavelength, .. } if *shear_attenuation_db_per_wavelength > 0.0))
                || case
                    .water_attenuation_db_per_wavelength
                    .iter()
                    .any(|&a| a > 0.0)
            {
                1e-10
            } else if stored.im == 0.0 {
                // Reuse the printed attenuation floor for residuals around an exact reference zero.
                1e-12
            } else {
                0.0
            },
            "imaginary wavenumber (.mod)",
        ));
        let shape = file.record(first + 2 + index);
        assert_eq!(mode.eigenfunction.len(), actual.sampled_depths_m.len());
        let reference: Vec<_> = (0..mode.eigenfunction.len())
            .map(|i| complex(shape, 8 * i))
            .collect();
        // Align only the arbitrary unit phase, never rescale amplitude to hide normalization errors.
        let overlap: Complex64 = reference
            .iter()
            .zip(&mode.eigenfunction)
            .map(|(a, b)| a.conj() * b)
            .sum();
        assert!(
            overlap.norm().is_finite() && overlap.norm() > 0.0,
            "mode phase cannot be aligned"
        );
        let phase = overlap / overlap.norm();
        for (&value, expected) in mode.eigenfunction.iter().zip(reference) {
            max_shape = max_shape.max(complex_close(value, expected * phase, 1e-6, "mode shape"));
        }
    }
    (max_k, max_loss, max_binary_loss, max_shape)
}

fn printed_loss_tolerance(text: &str) -> f64 {
    let (mantissa, exponent) = text.split_once(['E', 'e']).unwrap_or((text, "0"));
    let digits = mantissa.split_once('.').unwrap().1.len();
    0.5 * 10_f64.powi(exponent.parse::<i32>().unwrap() - i32::try_from(digits).unwrap())
}

#[test]
fn printed_loss_precision_accepts_both_fortran_general_formats() {
    assert_eq!(printed_loss_tolerance("-0.46E-09"), 5e-12);
    assert_eq!(printed_loss_tolerance("-0.13"), 0.005);
    assert_eq!(printed_loss_tolerance("-1.3"), 0.05);
}

fn compare_field(case: &Case, actual: &PressureField, file: &Records) -> f64 {
    assert_eq!(count(file.record(2), 0), 1, "FIELD frequency count");
    compare_field_at(case, actual, file, 0)
}

#[allow(clippy::cast_possible_truncation)]
fn compare_field_at(
    case: &Case,
    actual: &PressureField,
    file: &Records,
    frequency_index: usize,
) -> f64 {
    assert_eq!(&file.record(1)[..10], b"          ", "FIELD plot type");
    let header = file.record(2);
    let frequencies = count(header, 0);
    assert!(frequency_index < frequencies);
    for offset in [4, 8, 12] {
        assert_eq!(count(header, offset), 1, "frequency/bearing/x/y count");
    }
    assert_eq!(count(header, 16), actual.source_depths_m.len());
    assert_eq!(count(header, 20), actual.receiver_depths_m.len());
    assert_eq!(count(header, 24), actual.receiver_ranges_m.len());
    // FIELD leaves header freq0 unset; freqVec is the authoritative frequency.
    close(
        double(file.record(3), 8 * frequency_index),
        case.frequency_hz,
        1e-12,
        "field frequency",
    );
    for (record, depths) in [(7, &actual.source_depths_m), (8, &actual.receiver_depths_m)] {
        for (index, &depth) in depths.iter().enumerate() {
            close(
                f64::from(depth as f32),
                single(file.record(record), 4 * index),
                0.0,
                "field depth",
            );
        }
    }
    for (index, &range) in actual.receiver_ranges_m.iter().enumerate() {
        close(
            range,
            double(file.record(9), 8 * index),
            1e-9,
            "field range",
        );
    }
    assert_eq!(actual.receiver_offsets_m, case.receiver_offsets_m);
    let rows = actual.source_depths_m.len() * actual.receiver_depths_m.len();
    let ranges = actual.receiver_ranges_m.len();
    assert_eq!(file.len(), 10 + frequencies * rows, "field record count");
    assert_eq!(actual.pressure.len(), rows * ranges);
    let mut maximum = 0.0_f64;
    for (index, &pressure) in actual.pressure.iter().enumerate() {
        let expected = complex(
            file.record(10 + frequency_index * rows + index / ranges),
            8 * (index % ranges),
        );
        maximum = maximum.max(complex_close(
            pressure,
            expected,
            2e-6,
            &format!("pressure[{index}]"),
        ));
    }
    maximum
}

#[derive(Clone)]
struct Records {
    bytes: Vec<u8>,
    record_bytes: usize,
}

impl Records {
    fn read(path: &Path) -> Self {
        let bytes = fs::read(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let words = count(&bytes, 0);
        assert!((25..=1_000_000).contains(&words), "invalid record size");
        let record_bytes = words * 4;
        assert_eq!(bytes.len() % record_bytes, 0, "truncated reference file");
        Self {
            bytes,
            record_bytes,
        }
    }

    fn len(&self) -> usize {
        self.bytes.len() / self.record_bytes
    }

    fn record(&self, index: usize) -> &[u8] {
        self.bytes
            .get(index * self.record_bytes..(index + 1) * self.record_bytes)
            .expect("missing reference record")
    }
}

fn count(bytes: &[u8], offset: usize) -> usize {
    usize::try_from(i32::from_le_bytes(
        bytes[offset..offset + 4].try_into().unwrap(),
    ))
    .expect("negative count")
}

fn single(bytes: &[u8], offset: usize) -> f64 {
    f64::from(f32::from_le_bytes(
        bytes[offset..offset + 4].try_into().unwrap(),
    ))
}

fn double(bytes: &[u8], offset: usize) -> f64 {
    f64::from_le_bytes(bytes[offset..offset + 8].try_into().unwrap())
}

fn complex(bytes: &[u8], offset: usize) -> Complex64 {
    Complex64::new(single(bytes, offset), single(bytes, offset + 4))
}

#[test]
fn layered_comparator_detects_corruption_in_the_last_medium() {
    let root = fixtures().join("LayeredFluidThree");
    let case = load_complex_case(root.with_extension("env"), root.with_extension("flp")).unwrap();
    let result = solve(&case).unwrap();
    let root = fixtures().join("golden/LayeredFluidThree-krakenc");
    let modes = Records::read(&root.with_extension("mod"));
    let printed = fs::read_to_string(root.with_extension("prt")).unwrap();
    for offset in [
        modes.record_bytes + 24,
        modes.record_bytes + 28,
        2 * modes.record_bytes + 16,
        2 * modes.record_bytes + 20,
    ] {
        let mut corrupted = modes.clone();
        corrupted.bytes[offset..offset + 4].copy_from_slice(&f32::NAN.to_le_bytes());
        assert!(
            std::panic::catch_unwind(|| compare_modes(&case, &result.modes, &corrupted, &printed))
                .is_err()
        );
    }
}

#[test]
fn multifrequency_comparator_detects_later_frequency_corruption() {
    let root = fixtures().join("PekerisBroadband");
    let cases = load_frequency_cases(
        root.with_extension("env"),
        root.with_extension("flp"),
        kraken::ModeSolver::Kraken,
    )
    .unwrap();
    let case = &cases[1];
    let result = solve(case).unwrap();
    let root = fixtures().join("golden/PekerisBroadband");
    let mut modes = Records::read(&root.with_extension("mod"));
    let printed = fs::read_to_string(root.with_extension("prt")).unwrap();
    let first_count = count(modes.record(5), 0);
    let first = 7 + first_count + first_count.div_ceil(modes.record_bytes / 8);
    let offset = (first + 2 + result.modes.modes.len()) * modes.record_bytes;
    modes.bytes[offset..offset + 4].copy_from_slice(&f32::NAN.to_le_bytes());
    assert!(
        std::panic::catch_unwind(|| compare_modes_at(case, &result.modes, &modes, &printed, 1))
            .is_err()
    );
    let mut field = Records::read(&root.with_extension("shd"));
    let rows = case.source_depths_m.len() * case.receiver_depths_m.len();
    let offset = (10 + rows) * field.record_bytes;
    field.bytes[offset..offset + 4].copy_from_slice(&f32::NAN.to_le_bytes());
    assert!(std::panic::catch_unwind(|| compare_field_at(case, &result.field, &field, 1)).is_err());
}

#[test]
fn zero_reference_loss_accepts_roundoff_but_rejects_changed_attenuation() {
    let root = fixtures().join("Pekeris");
    let case = load_case(root.with_extension("env"), root.with_extension("flp")).unwrap();
    let mut result = solve(&case).unwrap();
    let root = fixtures().join("golden/Pekeris");
    let modes = Records::read(&root.with_extension("mod"));
    let printed = fs::read_to_string(root.with_extension("prt")).unwrap();
    for residual in [-3.34e-17, 3.34e-17, -5e-13, 5e-13] {
        result.modes.modes[0].horizontal_wavenumber_rad_per_m.im = residual;
        result.modes.modes[0].attenuation_nepers_per_m = -residual;
        compare_modes(&case, &result.modes, &modes, &printed);
    }
    for changed in [-2e-12, 2e-12, 1e-7, f64::NAN] {
        result.modes.modes[0].horizontal_wavenumber_rad_per_m.im = changed;
        result.modes.modes[0].attenuation_nepers_per_m = -changed;
        assert!(
            std::panic::catch_unwind(|| compare_modes(&case, &result.modes, &modes, &printed))
                .is_err()
        );
    }
}

#[test]
fn comparator_detects_corruption_but_accepts_eigenvector_sign_changes() {
    let root = fixtures().join("Pekeris");
    let case = load_case(root.with_extension("env"), root.with_extension("flp")).unwrap();
    let result = solve(&case).unwrap();
    let root = fixtures().join("golden/Pekeris");
    let modes = Records::read(&root.with_extension("mod"));
    let printed = fs::read_to_string(root.with_extension("prt")).unwrap();
    let field = Records::read(&root.with_extension("shd"));

    let mut flipped = modes.clone();
    for i in 0..case.mode_sample_depths_m.len() {
        let offset = 7 * flipped.record_bytes + 8 * i;
        let value = f32::from_le_bytes(flipped.bytes[offset..offset + 4].try_into().unwrap());
        flipped.bytes[offset..offset + 4].copy_from_slice(&(-value).to_le_bytes());
    }
    compare_modes(&case, &result.modes, &flipped, &printed);
    let lossy = fixtures().join("MunkBottomLoss");
    let lossy_case = load_case(lossy.with_extension("env"), lossy.with_extension("flp")).unwrap();
    let lossy_modes = Records::read(&fixtures().join("golden/MunkBottomLoss.mod"));
    let lossy_printed = fs::read_to_string(fixtures().join("golden/MunkBottomLoss.prt")).unwrap();
    let lossy_result = solve(&lossy_case).unwrap();
    let mut corrupted_loss = lossy_modes.clone();
    let imag = (7 + lossy_result.modes.modes.len()) * corrupted_loss.record_bytes + 4;
    corrupted_loss.bytes[imag..imag + 4].copy_from_slice(&1e-7_f32.to_le_bytes());
    assert!(
        std::panic::catch_unwind(|| compare_modes(
            &lossy_case,
            &lossy_result.modes,
            &corrupted_loss,
            &lossy_printed
        ))
        .is_err()
    );

    for value in [1.0_f32, f32::NAN] {
        for record in [4, 7, 7 + result.modes.modes.len()] {
            let mut corrupted = modes.clone();
            let offset = record * corrupted.record_bytes;
            corrupted.bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
            assert!(
                std::panic::catch_unwind(|| compare_modes(
                    &case,
                    &result.modes,
                    &corrupted,
                    &printed
                ))
                .is_err()
            );
        }
        let mut corrupted = field.clone();
        let offset = 10 * corrupted.record_bytes;
        corrupted.bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        assert!(
            std::panic::catch_unwind(|| compare_field(&case, &result.field, &corrupted)).is_err()
        );
    }
}

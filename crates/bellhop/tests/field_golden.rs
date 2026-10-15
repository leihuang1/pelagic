use std::fs;
use std::path::{Path, PathBuf};

use bellhop::Case;
use bellhop::legacy::load_case;
use bellhop::model::BeamComponent;
use bellhop::solver::{SimulationLimits, run};

const PRESSURE_TOLERANCE: f32 = 5.0e-8;

#[test]
fn field_modes_match_v2023_5_golden_pressures() {
    for stem in [
        "Field_G",
        "Field_B_incoherent",
        "Field_B_semi",
        "Field_S",
        "Field_CervenyC",
        "Field_CervenyR",
        "Field_CervenyF",
        "Field_CervenyW",
        "ShadedField",
        "FreeFGB_grid",
        "Field_ThorpLine",
        "Field_BioLine",
        "Field_WkbGradientC",
        "Field_WkbGradientR",
        "Field_LineS",
        "Field_LineCervenyC",
        "Field_LineCervenyR",
        "Field_LineSemiG",
        "Field_LineSemiB",
        "Field_NearGaussian",
    ] {
        let case = load_case(&fixture(&format!("{stem}.env"))).unwrap().value;
        let result = run(&case, SimulationLimits::default()).unwrap();
        let actual: Vec<_> = result
            .field_sources
            .iter()
            .flat_map(|source| &source.samples)
            .collect();
        let expected = read_pressure(&fixture(&format!("{stem}.csv")));
        assert_eq!(actual.len(), expected.len(), "{stem}");
        if matches!(stem, "Field_ThorpLine" | "Field_BioLine") {
            assert_eq!(
                case.environment.run.source_geometry,
                bellhop::model::SourceGeometry::Line
            );
            // Neither line scaling nor volume loss may become a dormant fixture option.
            for remove_loss in [false, true] {
                let mut definition = case.clone().into_definition();
                if remove_loss {
                    definition.environment.top_options.volume_attenuation =
                        bellhop::model::VolumeAttenuation::None;
                } else {
                    definition.environment.run.source_geometry =
                        bellhop::model::SourceGeometry::Point;
                }
                let control = run(
                    &Case::from_definition(definition).unwrap().value,
                    SimulationLimits::default(),
                )
                .unwrap();
                assert!(
                    control.field_sources[0]
                        .samples
                        .iter()
                        .zip(&actual)
                        .any(|(a, b)| (a.pressure.re - b.pressure.re)
                            .abs()
                            .max((a.pressure.im - b.pressure.im).abs())
                            > PRESSURE_TOLERANCE),
                    "{stem}: inactive option, remove_loss={remove_loss}"
                );
            }
        }

        for (sample_index, (sample, (real, imaginary))) in actual.iter().zip(expected).enumerate() {
            assert!(
                (sample.pressure.re - real).abs() <= PRESSURE_TOLERANCE,
                "{stem} sample {sample_index} real: actual={}, expected={real}",
                sample.pressure.re
            );
            assert!(
                (sample.pressure.im - imaginary).abs() <= PRESSURE_TOLERANCE,
                "{stem} sample {sample_index} imaginary: actual={}, expected={imaginary}",
                sample.pressure.im
            );
        }
    }
}

#[test]
fn cerveny_velocity_components_match_v2023_5_samples() {
    let mut definition = load_case(&fixture("Field_CervenyR.env"))
        .unwrap()
        .value
        .into_definition();
    definition
        .environment
        .trace
        .cerveny
        .as_mut()
        .unwrap()
        .component = BeamComponent::Vertical;
    let case = Case::from_definition(definition.clone()).unwrap().value;
    let vertical = run(&case, SimulationLimits::default())
        .unwrap()
        .field_sources[0]
        .samples[16]
        .pressure;
    assert!((vertical.re - 1.022_244_3e-3).abs() <= PRESSURE_TOLERANCE);
    assert!((vertical.im - 2.047_689e-3).abs() <= PRESSURE_TOLERANCE);

    definition
        .environment
        .trace
        .cerveny
        .as_mut()
        .unwrap()
        .component = BeamComponent::Horizontal;
    let case = Case::from_definition(definition).unwrap().value;
    let horizontal = run(&case, SimulationLimits::default())
        .unwrap()
        .field_sources[0]
        .samples[16]
        .pressure;
    assert!((horizontal.re - 6.562_056_5e-3).abs() <= PRESSURE_TOLERANCE);
    assert!((horizontal.im + 1.197_408_8e-2).abs() <= PRESSURE_TOLERANCE);
}

#[test]
fn nonfinite_pressure_scaling_is_rejected_before_publication() {
    for kind in [
        bellhop::model::RunKind::SemiCoherent,
        bellhop::model::RunKind::Incoherent,
    ] {
        let mut definition = load_case(&fixture("Field_LineS.env"))
            .unwrap()
            .value
            .into_definition();
        definition.environment.run.kind = kind;
        let case = Case::from_definition(definition).unwrap().value;
        let error = run(&case, SimulationLimits::default()).unwrap_err();
        assert_eq!(error.diagnostics()[0].code, "BH0302");
        assert!(error.diagnostics()[0].message.contains("non-finite"));
    }
}

#[test]
fn field_grid_obeys_the_configured_resource_limit() {
    let case = load_case(&fixture("Field_G.env")).unwrap().value;
    let error = run(
        &case,
        SimulationLimits {
            max_field_cells: 32,
            ..SimulationLimits::default()
        },
    )
    .unwrap_err();

    assert_eq!(error.diagnostics()[0].code, "BH0303");
}

fn read_pressure(path: &Path) -> Vec<(f32, f32)> {
    fs::read_to_string(path)
        .unwrap()
        .lines()
        .skip(1)
        .map(|line| {
            let values: Vec<&str> = line.split(',').collect();
            (values[2].parse().unwrap(), values[3].parse().unwrap())
        })
        .collect()
}

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/golden")
        .join(name)
}

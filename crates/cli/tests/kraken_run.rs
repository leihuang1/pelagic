// Serialization round-trips deliberately require exact floating-point equality.
#![allow(clippy::float_cmp)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use hdf5::types::VarLenUnicode;
use hdf5::{File, Group, H5Type};
use kraken::{Case, ModeSolver, SimulationResult};
use sha2::{Digest, Sha256};
#[path = "support/publication.rs"]
mod publication;
use publication::{h5, old_output, read_output};
#[path = "support/native.rs"]
#[allow(dead_code)]
mod native;

fn directory(name: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!("kraken-cli-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&path);
    fs::create_dir_all(&path).unwrap();
    path
}

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../kraken/tests/fixtures")
        .join(name)
}

fn run(env: &Path, output: &Path, solver: &str, extra: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_pelagic"))
        .args(["kraken", "run", "--format", "both"])
        .arg(env)
        .args(["--solver", solver, "--output"])
        .arg(output)
        .args(extra)
        .output()
        .unwrap()
}

fn attribute(location: &hdf5::Location, name: &str) -> String {
    location
        .attr(name)
        .unwrap()
        .read_scalar::<VarLenUnicode>()
        .unwrap()
        .as_str()
        .to_owned()
}

fn data<T: H5Type>(group: &Group, name: &str, shape: &[usize], unit: &str) -> Vec<T> {
    let dataset = group.dataset(name).unwrap();
    assert_eq!(dataset.shape(), shape, "{name}");
    assert!(dataset.dtype().unwrap().is::<T>(), "{name} datatype");
    assert_eq!(attribute(&dataset, "unit"), unit, "{name}");
    dataset.read_raw().unwrap()
}

#[test]
fn cli_round_trips_single_and_multifrequency_results() {
    let root = directory("round-trip");
    // CLI representatives cover serialization branches, not the solver's golden matrix.
    // TRC/BRC/IRC/SBP round-trips live in the resource-protection test below.
    for (name, solver) in [
        ("PekerisFiltered", "kraken"),
        ("PekerisComplexBlank", "krakenc"),
        ("PekerisDenseLoss", "krakenc"),
        ("PekerisRigidLoss", "krakenc"),
        ("MunkLeakyPchipBroadband", "krakenc"),
        ("FieldScaled", "kraken"),
        ("FieldIncoherent", "krakenc"),
        ("PekerisBroadband", "kraken"),
    ] {
        let env = fixture(name).with_extension("env");
        let output = root.join(name).with_extension("h5");
        let process = run(&env, &output, solver, &[]);
        assert!(
            process.status.success(),
            "{}",
            String::from_utf8_lossy(&process.stderr)
        );
        assert_product(&output, &env, &env.with_extension("flp"), solver);
        assert!(!output.join(".pelagic-stage").exists());
    }
    // Duplicate frequencies must have distinct index groups, never Hz-named keys.
    let env = root.join("duplicate.env");
    fs::write(
        &env,
        fs::read_to_string(fixture("PekerisBroadband.env"))
            .unwrap()
            .replace("75.0 50.0 62.5 /", "75.0 75.0 50.0 /"),
    )
    .unwrap();
    fs::copy(fixture("PekerisBroadband.flp"), env.with_extension("flp")).unwrap();
    let output = root.join("duplicate.h5");
    let process = run(&env, &output, "kraken", &[]);
    assert!(
        process.status.success(),
        "{}",
        String::from_utf8_lossy(&process.stderr)
    );
    assert_product(&output, &env, &env.with_extension("flp"), "kraken");
    fs::remove_dir_all(root).unwrap();
}

#[test]
#[ignore = "requires unmodified upstream BroadBand/MunkK input snapshots"]
fn original_complex_spline_broadband_is_complete_and_preserves_late_failure() {
    let env = PathBuf::from(std::env::var_os("KRAKEN_DIFFERENTIAL_ENV").unwrap());
    let cases =
        kraken::legacy::load_frequency_cases(&env, env.with_extension("flp"), ModeSolver::Krakenc)
            .unwrap();
    assert_eq!(cases.len(), 2);
    assert_eq!(cases[1].frequency_hz, 500.0);
    assert_eq!(cases[1].interpolation, kraken::Interpolation::Spline);
    let root = directory("complex-spline-broadband");
    let output = root.join("previous.h5");
    old_output(&output, b"previous output");
    let process = run(&env, &output, "krakenc", &[]);
    assert!(
        process.status.success(),
        "{}",
        String::from_utf8_lossy(&process.stderr)
    );
    assert_product(&output, &env, &env.with_extension("flp"), "krakenc");
    assert!(!output.join(".pelagic-stage").exists());
    // Derived failure: both original frequencies succeed before 7500 Hz hits
    // the unchanged 300M root budget. Never publish the successful prefix.
    let late = root.join("late.env");
    fs::write(
        &late,
        fs::read_to_string(&env)
            .unwrap()
            .replace("2              ! Nfreq", "3              ! Nfreq")
            .replace(" 50 500 /", " 50 500 7500 /"),
    )
    .unwrap();
    fs::copy(env.with_extension("flp"), late.with_extension("flp")).unwrap();
    old_output(&output, b"previous output");
    let process = run(&late, &output, "krakenc", &[]);
    assert_failure(&process, 3, &output, b"previous output");
    let message = String::from_utf8_lossy(&process.stderr);
    assert!(
        message.contains("frequency[2] (7500 Hz)")
            && message.contains("complex root work limit exceeded")
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn cli_round_trips_water_material_results() {
    let root = directory("water-materials");
    for name in ["WaterLossPower"] {
        for engine in ["kraken", "krakenc"] {
            let env = fixture(name).with_extension("env");
            let output = root.join(format!("{name}-{engine}.h5"));
            let process = run(&env, &output, engine, &[]);
            assert!(
                process.status.success(),
                "{}",
                String::from_utf8_lossy(&process.stderr)
            );
            assert_product(&output, &env, &env.with_extension("flp"), engine);
        }
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn cli_round_trips_smooth_boundaries_and_top_tables() {
    let root = directory("fluid-boundaries");
    // Each end exercises V/R/A through each solver; Air adds the radiating top.
    for name in [
        "FluidBoundaryVR",
        "FluidBoundaryRA",
        "FluidBoundaryAV",
        "FluidBoundaryAir",
    ] {
        for engine in ["kraken", "krakenc"] {
            if engine == "kraken" && name == "FluidBoundaryAir" {
                continue;
            }
            let env = fixture(name).with_extension("env");
            let output = root.join(format!("{name}-{engine}.h5"));
            let process = run(&env, &output, engine, &[]);
            assert!(
                process.status.success(),
                "{}",
                String::from_utf8_lossy(&process.stderr)
            );
            assert_product(&output, &env, &env.with_extension("flp"), engine);
        }
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn cli_round_trips_layered_fluids() {
    let root = directory("layered-fluids");
    for name in ["LayeredFluidPower"] {
        for engine in ["kraken", "krakenc"] {
            let env = fixture(name).with_extension("env");
            let output = root.join(format!("{name}-{engine}.h5"));
            let process = run(&env, &output, engine, &[]);
            assert!(
                process.status.success(),
                "{}",
                String::from_utf8_lossy(&process.stderr)
            );
            assert_product(&output, &env, &env.with_extension("flp"), engine);
        }
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn cli_round_trips_finite_elastic_layers() {
    let root = directory("finite-elastic-products");
    // Empty top/bottom groups, homogeneous/graded stacks, per-frequency material state.
    for name in [
        "FiniteElasticBottomN",
        "FiniteElasticTopN",
        "FiniteElasticPower",
        "FiniteElasticStack",
        "GradedElasticStack",
        "GradedElasticPower",
    ] {
        for solver in ["kraken", "krakenc"] {
            let env = fixture(&format!("{name}.env"));
            let output = root.join(format!("{name}-{solver}.h5"));
            let process = run(&env, &output, solver, &[]);
            assert!(
                process.status.success(),
                "{}",
                String::from_utf8_lossy(&process.stderr)
            );
            assert_product(&output, &env, &env.with_extension("flp"), solver);
        }
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn finite_elastic_failures_preserve_output_and_remove_scratch() {
    let root = directory("finite-elastic-failures");
    let output = root.join("old.h5");
    let old = b"old finite elastic output";
    old_output(&output, old);
    let env = root.join("bad-material.env");
    fs::write(
        &env,
        fs::read_to_string(fixture("FiniteElasticBothN.env"))
            .unwrap()
            .replace("20.0 3000.0 1400.0", "20.0 1500.0 1400.0"),
    )
    .unwrap();
    fs::copy(fixture("FiniteElasticBothN.flp"), env.with_extension("flp")).unwrap();
    let process = run(&env, &output, "kraken", &[]);
    assert_failure(&process, 2, &output, old);
    assert!(String::from_utf8_lossy(&process.stderr).contains("positive bulk modulus"));
    let env = root.join("bad.env");
    fs::write(
        &env,
        fs::read_to_string(fixture("OriginalElasticSediment.env"))
            .unwrap()
            .replace("1300.0  2000.0", "2500.0  3000.0"),
    )
    .unwrap();
    fs::copy(
        fixture("OriginalElasticSediment.flp"),
        env.with_extension("flp"),
    )
    .unwrap();
    assert_failure(&run(&env, &output, "kraken", &[]), 3, &output, old);
    assert_failure(
        &run(
            &fixture("OriginalElasticIce.env"),
            &output,
            "krakenc",
            &["--max-output-bytes", "1"],
        ),
        4,
        &output,
        old,
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn cli_round_trips_elastic_half_spaces() {
    let root = directory("elastic-half-spaces");
    for name in ["ElasticHalfBothS", "ElasticHalfTopBroadband"] {
        for engine in ["kraken", "krakenc"] {
            let env = fixture(name).with_extension("env");
            let output = root.join(format!("{name}-{engine}.h5"));
            let process = run(&env, &output, engine, &[]);
            assert!(
                process.status.success(),
                "{}",
                String::from_utf8_lossy(&process.stderr)
            );
            assert_product(&output, &env, &env.with_extension("flp"), engine);
        }
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn finite_elastic_metadata_checks_the_last_frequency() {
    let root = directory("finite-elastic-metadata-corruption");
    let env = fixture("FiniteElasticPower.env");
    let output = root.join("power.h5");
    let process = run(&env, &output, "krakenc", &[]);
    assert!(
        process.status.success(),
        "{}",
        String::from_utf8_lossy(&process.stderr)
    );
    // Retain one RW handle; parallel children must not force a native lock upgrade.
    let file = File::open_rw(h5(&output, &env)).unwrap();
    assert_product(&output, &env, &env.with_extension("flp"), "krakenc");
    file.group("frequencies/3/elastic_media/bottom/0")
        .unwrap()
        .attr("shear_attenuation_db_per_wavelength")
        .unwrap()
        .write_scalar(&0.25_f64)
        .unwrap();
    file.flush().unwrap();
    assert!(
        std::panic::catch_unwind(|| assert_product(
            &output,
            &env,
            &env.with_extension("flp"),
            "krakenc"
        ))
        .is_err()
    );
    drop(file);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn graded_elastic_metadata_detects_corruption_in_each_profile_column() {
    let root = directory("graded-metadata-corruption");
    let env = fixture("GradedElasticStack.env");
    let output = root.join("stack.h5");
    assert!(run(&env, &output, "krakenc", &[]).status.success());
    let case = kraken::legacy::load_complex_case(&env, env.with_extension("flp")).unwrap();
    let file = File::open_rw(h5(&output, &env)).unwrap();
    let group = file.group("frequencies/0").unwrap();
    assert_finite_elastic_metadata(&group, &case);
    let profile = group
        .group("elastic_media/bottom/1/material_profile")
        .unwrap();
    for name in profile.member_names().unwrap() {
        let dataset = profile.dataset(&name).unwrap();
        let original = dataset.read_raw::<f64>().unwrap();
        let mut damaged = original.clone();
        *damaged.last_mut().unwrap() += 1.0;
        dataset.write_raw(&damaged).unwrap();
        assert!(
            std::panic::catch_unwind(|| assert_finite_elastic_metadata(&group, &case)).is_err(),
            "{name}"
        );
        dataset.write_raw(&original).unwrap();
    }
    drop(profile);
    drop(group);
    file.close().unwrap();
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn elastic_metadata_checks_the_last_frequency() {
    let root = directory("elastic-metadata-corruption");
    let env = fixture("ElasticHalfPower.env");
    let output = root.join("power.h5");
    let process = run(&env, &output, "krakenc", &[]);
    assert!(
        process.status.success(),
        "{}",
        String::from_utf8_lossy(&process.stderr)
    );
    // Avoid a read-to-write reopen: parallel CLI children can inherit native read handles.
    let file = File::open_rw(h5(&output, &env)).unwrap();
    assert_product(&output, &env, &env.with_extension("flp"), "krakenc");
    file.group("frequencies/3")
        .unwrap()
        .attr("bottom_shear_sound_speed_mps")
        .unwrap()
        .write_scalar(&1999.0_f64)
        .unwrap();
    file.flush().unwrap();
    assert!(
        std::panic::catch_unwind(|| assert_product(
            &output,
            &env,
            &env.with_extension("flp"),
            "krakenc"
        ))
        .is_err()
    );
    drop(file);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn elastic_failures_preserve_output_and_remove_scratch() {
    let root = directory("elastic-failures");
    let output = root.join("previous.h5");
    old_output(&output, b"old elastic output");
    let invalid = root.join("invalid.env");
    fs::write(
        &invalid,
        fs::read_to_string(fixture("ElasticHalfTopN.env"))
            .unwrap()
            .replace("4500.0 2300.0", "2000.0 2300.0"),
    )
    .unwrap();
    fs::copy(
        fixture("ElasticHalfTopN.flp"),
        invalid.with_extension("flp"),
    )
    .unwrap();
    let process = run(&invalid, &output, "kraken", &[]);
    assert_failure(&process, 2, &output, b"old elastic output");
    assert!(String::from_utf8_lossy(&process.stderr).contains("cp² > 4/3 cs²"));

    let env = root.join("outside.env");
    fs::write(
        &env,
        fs::read_to_string(fixture("ElasticHalfBottomN.env"))
            .unwrap()
            .replace("1400.0 1800.0", "2500.0 3000.0"),
    )
    .unwrap();
    fs::copy(fixture("ElasticHalfBottomN.flp"), env.with_extension("flp")).unwrap();
    let process = run(&env, &output, "kraken", &[]);
    assert_failure(&process, 3, &output, b"old elastic output");
    assert!(String::from_utf8_lossy(&process.stderr).contains("phase-speed limits"));

    let process = run(
        &fixture("ElasticHalfBothN.env"),
        &output,
        "krakenc",
        &["--max-output-bytes", "1"],
    );
    assert_failure(&process, 4, &output, b"old elastic output");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn layered_failures_preserve_output_and_remove_scratch() {
    let root = directory("layered-failures");
    let output = root.join("previous.h5");
    let increased = root.join("increased.env");
    fs::write(
        &increased,
        fs::read_to_string(fixture("OriginalLayeredDouble.env"))
            .unwrap()
            .replace("1400.0  2000.0", "1506.3905 1600.0"),
    )
    .unwrap();
    fs::copy(
        fixture("OriginalLayeredDouble.flp"),
        increased.with_extension("flp"),
    )
    .unwrap();
    for engine in ["kraken", "krakenc"] {
        old_output(&output, b"old layered output");
        let process = run(&increased, &output, engine, &[]);
        assert_failure(&process, 3, &output, b"old layered output");
        assert!(String::from_utf8_lossy(&process.stderr).contains("mode count changed"));
    }
    let env = root.join("later.env");
    fs::write(
        &env,
        fs::read_to_string(fixture("LayeredFluidPower.env"))
            .unwrap()
            .replace("75.0 50.0 62.5 50.0 /", "75.0 50.0 62.5 7500.0 /"),
    )
    .unwrap();
    fs::copy(fixture("LayeredFluidPower.flp"), env.with_extension("flp")).unwrap();
    let process = run(&env, &output, "krakenc", &[]);
    assert_failure(&process, 3, &output, b"old layered output");
    let message = String::from_utf8_lossy(&process.stderr);
    assert!(
        message.contains("frequency[3] (7500 Hz)")
            && message.contains("complex root work limit exceeded"),
        "{message}"
    );
    let env = fixture("LayeredFluidN.env");
    let process = run(&env, &output, "krakenc", &["--max-output-bytes", "1"]);
    assert_failure(&process, 4, &output, b"old layered output");
    fs::remove_dir_all(root).unwrap();
}

#[allow(clippy::too_many_lines)]
fn assert_finite_elastic_metadata(group: &hdf5::Group, case: &kraken::Case) {
    let count = case.top_elastic_layers.len() + case.bottom_elastic_layers.len();
    assert_eq!(
        group
            .attr("finite_elastic_layer_count")
            .unwrap()
            .read_scalar::<u64>()
            .unwrap(),
        count as u64
    );
    if count == 0 {
        return;
    }
    for (side, layers, mut top) in [
        ("top", &case.top_elastic_layers, 0.0),
        (
            "bottom",
            &case.bottom_elastic_layers,
            case.fluid_bottom_depth_m(),
        ),
    ] {
        let media = group.group(&format!("elastic_media/{side}")).unwrap();
        assert_eq!(media.member_names().unwrap().len(), layers.len());
        for (index, layer) in layers.iter().enumerate() {
            let material = media.group(&index.to_string()).unwrap();
            assert_eq!(attribute(&material, "material"), "elastic");
            assert_eq!(
                attribute(&material, "attenuation_model"),
                if case.mode_solver == ModeSolver::Kraken {
                    "reference_real_stiffness"
                } else {
                    "complex"
                }
            );
            assert_eq!(
                material
                    .attr("requested_mesh_points")
                    .unwrap()
                    .read_scalar::<u64>()
                    .unwrap(),
                layer.mesh_points as u64
            );
            for (name, expected) in [
                ("top_depth_m", top),
                ("bottom_depth_m", layer.bottom_depth_m),
                (
                    "compressional_sound_speed_mps",
                    layer.compressional_sound_speed_mps,
                ),
                ("shear_sound_speed_mps", layer.shear_sound_speed_mps),
                ("density_g_cm3", layer.density_g_cm3),
                (
                    "compressional_attenuation_db_per_wavelength",
                    layer.compressional_attenuation_db_per_wavelength,
                ),
                (
                    "shear_attenuation_db_per_wavelength",
                    layer.shear_attenuation_db_per_wavelength,
                ),
            ] {
                let attr = material.attr(name).unwrap();
                assert!(attr.dtype().unwrap().is::<f64>());
                assert_eq!(
                    attr.read_scalar::<f64>().unwrap().to_bits(),
                    expected.to_bits()
                );
            }
            let points = &layer.material_profile;
            assert_eq!(
                material
                    .attr("material_profile_point_count")
                    .unwrap()
                    .read_scalar::<u64>()
                    .unwrap(),
                points.len() as u64
            );
            if points.is_empty() {
                assert!(!material.link_exists("material_profile"));
            } else {
                let profile = material.group("material_profile").unwrap();
                assert_eq!(profile.member_names().unwrap().len(), 6);
                for (name, unit, expected) in [
                    (
                        "depth_m",
                        "m",
                        points.iter().map(|p| p.depth_m).collect::<Vec<_>>(),
                    ),
                    (
                        "compressional_sound_speed_mps",
                        "m/s",
                        points
                            .iter()
                            .map(|p| p.compressional_sound_speed_mps)
                            .collect(),
                    ),
                    (
                        "shear_sound_speed_mps",
                        "m/s",
                        points.iter().map(|p| p.shear_sound_speed_mps).collect(),
                    ),
                    (
                        "density_g_cm3",
                        "g/cm^3",
                        points.iter().map(|p| p.density_g_cm3).collect(),
                    ),
                    (
                        "compressional_attenuation_db_per_wavelength",
                        "dB/wavelength",
                        points
                            .iter()
                            .map(|p| p.compressional_attenuation_db_per_wavelength)
                            .collect(),
                    ),
                    (
                        "shear_attenuation_db_per_wavelength",
                        "dB/wavelength",
                        points
                            .iter()
                            .map(|p| p.shear_attenuation_db_per_wavelength)
                            .collect(),
                    ),
                ] {
                    assert_eq!(
                        data::<f64>(&profile, name, &[points.len()], unit),
                        expected,
                        "{side}/{index}/{name}"
                    );
                }
            }
            top = layer.bottom_depth_m;
        }
    }
}

#[allow(clippy::too_many_lines)]
fn assert_product(output: &Path, env: &Path, flp: &Path, solver: &str) {
    native::kraken(output, env);
    let engine = if solver == "krakenc" {
        ModeSolver::Krakenc
    } else {
        ModeSolver::Kraken
    };
    let cases = kraken::legacy::load_frequency_cases(env, flp, engine).unwrap();
    let file = File::open(h5(output, env)).unwrap();
    assert_eq!(
        file.attr("schema_version")
            .unwrap()
            .read_scalar::<u32>()
            .unwrap(),
        1
    );
    assert_eq!(attribute(&file, "schema_name"), "kraken");
    assert_eq!(attribute(&file, "solver"), solver);
    assert_eq!(attribute(&file, "title"), cases[0].title);
    assert!(
        attribute(&file, "compatibility_reference")
            .contains("475108519289c6fb488b58980c644ea14eccc604")
    );
    assert_eq!(
        file.attr("frequency_count")
            .unwrap()
            .read_scalar::<u64>()
            .unwrap(),
        cases.len() as u64
    );
    assert_eq!(
        data::<f64>(&file, "frequency_hz", &[cases.len()], "Hz"),
        cases
            .iter()
            .map(|case| case.frequency_hz)
            .collect::<Vec<_>>()
    );
    assert_eq!(
        file.group("frequencies")
            .unwrap()
            .member_names()
            .unwrap()
            .len(),
        cases.len()
    );
    let mut inputs = vec![("env", env.to_path_buf()), ("flp", flp.to_path_buf())];
    if matches!(
        cases[0].surface_boundary,
        kraken::SurfaceBoundary::Reflection(_)
    ) {
        inputs.push(("trc", env.with_extension("trc")));
    }
    match &cases[0].bottom_boundary {
        kraken::BottomBoundary::Reflection(_) => inputs.push(("brc", env.with_extension("brc"))),
        kraken::BottomBoundary::Impedance { .. } => inputs.push(("irc", env.with_extension("irc"))),
        _ => {}
    }
    if !cases[0].source_pattern.is_empty() {
        inputs.push(("sbp", flp.with_extension("sbp")));
    }
    assert_eq!(
        file.group("inputs").unwrap().member_names().unwrap().len(),
        inputs.len()
    );
    for (name, path) in inputs {
        let bytes = fs::read(&path).unwrap();
        let group = file.group(&format!("inputs/{name}")).unwrap();
        assert_eq!(attribute(&group, "filename"), path.to_string_lossy());
        assert_eq!(
            group
                .attr("size_bytes")
                .unwrap()
                .read_scalar::<u64>()
                .unwrap(),
            bytes.len() as u64
        );
        assert_eq!(
            attribute(&group, "sha256"),
            format!("{:x}", Sha256::digest(bytes))
        );
    }
    let mut mode_count = 0_u64;
    let mut pressure_count = 0_u64;
    let fields = kraken::legacy::load_field_cases(env, flp, engine).unwrap();
    for ((index, case), result) in cases
        .iter()
        .enumerate()
        .zip(kraken::solve_frequencies(&fields))
    {
        let result = result.unwrap();
        let expected = kraken::SimulationResult {
            modes: result.modes.into_iter().next().unwrap(),
            field: result.field,
        };
        let group = file.group(&format!("frequencies/{index}")).unwrap();
        assert_eq!(
            group
                .attr("frequency_hz")
                .unwrap()
                .read_scalar::<f64>()
                .unwrap(),
            case.frequency_hz
        );
        assert_eq!(
            group
                .attr("mesh_reference_frequency_hz")
                .unwrap()
                .read_scalar::<f64>()
                .unwrap(),
            case.mesh_reference_frequency_hz
                .unwrap_or(case.frequency_hz)
        );
        assert_eq!(
            group
                .attr("requested_mesh_points")
                .unwrap()
                .read_scalar::<u64>()
                .unwrap(),
            case.mesh_points as u64
        );
        assert_eq!(
            group
                .attr("max_range_m")
                .unwrap()
                .read_scalar::<f64>()
                .unwrap(),
            case.max_range_m
        );
        assert_eq!(
            group
                .attr("field_mode_limit")
                .unwrap()
                .read_scalar::<u64>()
                .unwrap(),
            case.mode_limit as u64
        );
        assert_eq!(
            attribute(&group, "source_geometry"),
            match case.source_geometry {
                kraken::SourceGeometry::Line => "line",
                kraken::SourceGeometry::Point => "point",
                kraken::SourceGeometry::ScaledCylindrical => "scaled_cylindrical",
            }
        );
        assert_eq!(
            attribute(&group, "mode_addition"),
            match case.mode_addition {
                kraken::ModeAddition::Coherent => "coherent",
                kraken::ModeAddition::Incoherent => "incoherent",
            }
        );
        assert_eq!(
            attribute(&group, "source_pattern"),
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
        assert_eq!(
            attribute(&group, "surface_boundary"),
            match &case.surface_boundary {
                kraken::SurfaceBoundary::Vacuum => "V",
                kraken::SurfaceBoundary::Rigid => "R",
                kraken::SurfaceBoundary::FluidHalfSpace
                | kraken::SurfaceBoundary::ElasticHalfSpace { .. } => "A",
                kraken::SurfaceBoundary::Reflection(_) => "F",
                kraken::SurfaceBoundary::Impedance { .. } => "P",
            }
        );
        assert_eq!(
            attribute(&group, "bottom_boundary"),
            match &case.bottom_boundary {
                kraken::BottomBoundary::Vacuum => "V",
                kraken::BottomBoundary::FluidHalfSpace
                | kraken::BottomBoundary::ElasticHalfSpace { .. } => "A",
                kraken::BottomBoundary::Rigid => "R",
                kraken::BottomBoundary::Reflection(_) => "F",
                kraken::BottomBoundary::Impedance { .. } => "P",
            }
        );
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
                    attribute(&group, &format!("{name}_half_space_material")),
                    "elastic"
                );
                assert_eq!(
                    attribute(&group, &format!("{name}_elastic_attenuation_model")),
                    if engine == ModeSolver::Kraken {
                        "reference_real"
                    } else {
                        "complex"
                    }
                );
                for (name, value) in [
                    (format!("{name}_sound_speed_mps"), cp),
                    (format!("{name}_density_g_cm3"), density),
                    (format!("{name}_attenuation_db_per_wavelength"), loss),
                    (
                        format!("{name}_shear_sound_speed_mps"),
                        *shear_sound_speed_mps,
                    ),
                    (
                        format!("{name}_shear_attenuation_db_per_wavelength"),
                        *shear_attenuation_db_per_wavelength,
                    ),
                ] {
                    assert_eq!(
                        group.attr(&name).unwrap().read_scalar::<f64>().unwrap(),
                        value
                    );
                }
            }
        }
        assert_finite_elastic_metadata(&group, case);
        let layer_count = 1 + case.additional_fluid_layers.len();
        assert_eq!(
            group
                .attr("finite_fluid_layer_count")
                .unwrap()
                .read_scalar::<u64>()
                .unwrap(),
            layer_count as u64
        );
        let media = group.group("media").unwrap();
        assert_eq!(media.member_names().unwrap().len(), layer_count);
        let mut top_depth = case.fluid_top_depth_m();
        for index in 0..layer_count {
            let (bottom_depth, density, mesh) = if index == 0 {
                (
                    case.water_depth_m,
                    case.water_density_g_cm3,
                    case.mesh_points,
                )
            } else {
                let layer = &case.additional_fluid_layers[index - 1];
                (layer.bottom_depth_m, layer.density_g_cm3, layer.mesh_points)
            };
            let layer = media.group(&index.to_string()).unwrap();
            assert_eq!(
                layer
                    .attr("top_depth_m")
                    .unwrap()
                    .read_scalar::<f64>()
                    .unwrap(),
                top_depth
            );
            assert_eq!(
                layer
                    .attr("bottom_depth_m")
                    .unwrap()
                    .read_scalar::<f64>()
                    .unwrap(),
                bottom_depth
            );
            assert_eq!(
                layer
                    .attr("density_g_cm3")
                    .unwrap()
                    .read_scalar::<f64>()
                    .unwrap(),
                density
            );
            assert_eq!(
                layer
                    .attr("requested_mesh_points")
                    .unwrap()
                    .read_scalar::<u64>()
                    .unwrap(),
                mesh as u64
            );
            top_depth = bottom_depth;
        }
        assert_modes(&group.group("modes").unwrap(), &expected);
        assert_field(&group.group("field").unwrap(), case, &expected);
        mode_count += expected.modes.modes.len() as u64;
        pressure_count += expected.field.pressure.len() as u64;
    }
    assert_eq!(
        file.attr("mode_count")
            .unwrap()
            .read_scalar::<u64>()
            .unwrap(),
        mode_count
    );
    assert_eq!(
        file.attr("pressure_count")
            .unwrap()
            .read_scalar::<u64>()
            .unwrap(),
        pressure_count
    );
}

fn assert_modes(group: &Group, expected: &SimulationResult) {
    let depths = &expected.modes.sampled_depths_m;
    let count = expected.modes.modes.len();
    assert_eq!(
        attribute(group, "eigenfunction_axis_order"),
        "mode,sample_depth"
    );
    assert_eq!(
        data::<f64>(group, "sample_depth_m", &[depths.len()], "m"),
        *depths
    );
    for (name, unit, values) in [
        (
            "horizontal_wavenumber_real",
            "rad/m",
            expected
                .modes
                .modes
                .iter()
                .map(|m| m.horizontal_wavenumber_rad_per_m.re)
                .collect::<Vec<_>>(),
        ),
        (
            "horizontal_wavenumber_imaginary",
            "rad/m",
            expected
                .modes
                .modes
                .iter()
                .map(|m| m.horizontal_wavenumber_rad_per_m.im)
                .collect(),
        ),
        (
            "phase_speed_mps",
            "m/s",
            expected
                .modes
                .modes
                .iter()
                .map(|m| m.phase_speed_mps)
                .collect(),
        ),
        (
            "group_speed_mps",
            "m/s",
            expected
                .modes
                .modes
                .iter()
                .map(|m| m.group_speed_mps)
                .collect(),
        ),
        (
            "attenuation_nepers_per_m",
            "neper/m",
            expected
                .modes
                .modes
                .iter()
                .map(|m| m.attenuation_nepers_per_m)
                .collect(),
        ),
    ] {
        assert_eq!(data::<f64>(group, name, &[count], unit), values, "{name}");
    }
    for (name, imaginary) in [
        ("eigenfunction_real", false),
        ("eigenfunction_imaginary", true),
    ] {
        let values: Vec<_> = expected
            .modes
            .modes
            .iter()
            .flat_map(|mode| &mode.eigenfunction)
            .map(|value| if imaginary { value.im } else { value.re })
            .collect();
        assert_eq!(
            data::<f64>(group, name, &[count, depths.len()], "reference_normalized"),
            values
        );
    }
}

fn assert_field(group: &Group, case: &Case, expected: &SimulationResult) {
    assert_eq!(
        attribute(group, "pressure_axis_order"),
        "source_depth,receiver_depth,receiver_range"
    );
    for (name, values) in [
        ("source_depth_m", &case.source_depths_m),
        ("receiver_depth_m", &case.receiver_depths_m),
        ("receiver_range_m", &case.receiver_ranges_m),
        ("receiver_offset_m", &case.receiver_offsets_m),
    ] {
        assert_eq!(data::<f64>(group, name, &[values.len()], "m"), *values);
    }
    let shape = [
        case.source_depths_m.len(),
        case.receiver_depths_m.len(),
        case.receiver_ranges_m.len(),
    ];
    for (name, imaginary) in [("pressure_real", false), ("pressure_imaginary", true)] {
        let values: Vec<_> = expected
            .field
            .pressure
            .iter()
            .map(|p| if imaginary { p.im } else { p.re })
            .collect();
        let stored: Vec<_> = data::<f32>(group, name, &shape, "1")
            .into_iter()
            .map(f64::from)
            .collect();
        assert_eq!(stored, values, "{name}");
    }
}

#[test]
#[allow(clippy::too_many_lines)]
fn cli_round_trips_profile_fields_and_preserves_outputs_on_later_failure() {
    let root = directory("profiles");
    for (name, propagation) in [("ProfilesAd", "adiabatic"), ("ProfilesCm", "coupled")] {
        let env = root.join(name).with_extension("env");
        let flp = env.with_extension("flp");
        for ext in ["env", "flp"] {
            fs::copy(fixture(name).with_extension(ext), env.with_extension(ext)).unwrap();
        }
        let output = env.with_extension("h5");
        assert!(run(&env, &output, "kraken", &[]).status.success());
        let cases =
            kraken::legacy::load_field_cases(&env, &flp, kraken::ModeSolver::Kraken).unwrap();
        let expected = kraken::solve_field(&cases[0]).unwrap();
        let file = hdf5::File::open(h5(&output, &env)).unwrap();
        for (kind, path) in [("env", &env), ("flp", &flp)] {
            let input = file.group(&format!("inputs/{kind}")).unwrap();
            assert_eq!(
                attribute(&input, "sha256"),
                format!("{:x}", Sha256::digest(fs::read(path).unwrap()))
            );
        }
        let group = file.group("frequencies/0").unwrap();
        assert_eq!(attribute(&group, "field_propagation"), propagation);
        assert_eq!(
            data::<f64>(&group, "profile_range_m", &[4], "m"),
            cases[0].ranges_m()
        );
        let profiles = group.group("profiles").unwrap();
        assert_eq!(profiles.member_names().unwrap().len(), 4);
        let mode_count: usize = expected.modes.iter().map(|m| m.modes.len()).sum();
        assert_eq!(
            file.attr("mode_count")
                .unwrap()
                .read_scalar::<u64>()
                .unwrap(),
            mode_count as u64
        );
        for (index, modes) in expected.modes.into_iter().enumerate() {
            let product = SimulationResult {
                modes,
                field: expected.field.clone(),
            };
            let profile = profiles.group(&index.to_string()).unwrap();
            assert_eq!(
                attribute(&profile, "title"),
                cases[0].profiles()[index].title
            );
            assert_modes(&profile.group("modes").unwrap(), &product);
            if index == 0 {
                assert_modes(&group.group("modes").unwrap(), &product);
                assert_field(
                    &group.group("field").unwrap(),
                    &cases[0].profiles()[0],
                    &product,
                );
            }
        }
        drop(profiles);
        drop(group);
        file.close().unwrap();
        let old = read_output(&output);
        let source = fs::read_to_string(&env).unwrap();
        // The fourth profile validates, but has no root in its selected spectral interval.
        let split = source.rfind("1400.0 1700.0").unwrap();
        fs::write(
            &env,
            format!(
                "{}{}",
                &source[..split],
                source[split..].replacen("1400.0 1700.0", "1699.0 1700.0", 1)
            ),
        )
        .unwrap();
        let process = run(&env, &output, "kraken", &[]);
        assert_failure(&process, 3, &output, &old);
        assert!(String::from_utf8_lossy(&process.stderr).contains("profiles[3]"));
        fs::write(&env, &source).unwrap();
        assert_failure(
            &run(&env, &output, "kraken", &["--max-output-bytes", "100"]),
            4,
            &output,
            &old,
        );
        fs::write(
            &flp,
            fs::read_to_string(&flp)
                .unwrap()
                .replace("4\n0.0 0.8 1.6 2.4", "3\n0.0 0.8 1.6"),
        )
        .unwrap();
        assert_failure(&run(&env, &output, "kraken", &[]), 2, &output, &old);
    }
    let env = root.join("layered.env");
    let flp = env.with_extension("flp");
    let output = env.with_extension("h5");
    let source = fs::read_to_string(fixture("LayeredFluidThree.env")).unwrap();
    let field = fs::read_to_string(fixture("LayeredFluidThree.flp")).unwrap();
    fs::write(&env, format!("{source}{source}")).unwrap();
    fs::write(
        &flp,
        field
            .replace("'RAOC'", "'RCOC'")
            .replace("1\n0.0\n", "2\n0.0 0.5 /\n"),
    )
    .unwrap();
    let process = run(&env, &output, "kraken", &[]);
    assert!(
        process.status.success(),
        "{}",
        String::from_utf8_lossy(&process.stderr)
    );
    let old = read_output(&output);
    // Only the second profile omits internal interfaces; the single-profile case is otherwise valid.
    let coarse = source.replace(
        "3\n31.0 70.0 105.0 /\n7\n0.0 69.99 70.0 70.01 105.0 139.99 140.0 /",
        "1\n31.0 /\n2\n0.0 140.0 /",
    );
    assert_ne!(coarse, source);
    fs::write(&env, format!("{source}{coarse}")).unwrap();
    let process = run(&env, &output, "kraken", &[]);
    assert_failure(&process, 2, &output, &old);
    assert!(
        String::from_utf8_lossy(&process.stderr).contains("fluid-interface quadrature stencil")
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn cli_discovers_and_routes_profile_boundary_resources() {
    let root = directory("profile-resources");
    let env = root.join("mixed.env");
    let flp = env.with_extension("flp");
    let field = fs::read_to_string(fixture("TabRefBrcC.flp")).unwrap();
    fs::write(
        &flp,
        field
            .replace("'R OC'", "'RAOC'")
            .replace("1\n0.0 /", "4\n0.0 0.4 0.8 1.2 /"),
    )
    .unwrap();
    for (name, ext) in [
        ("TabRefBrcC", "brc"),
        ("TabRefIrcC", "irc"),
        ("FluidTrcC", "trc"),
    ] {
        fs::copy(fixture(name).with_extension(ext), env.with_extension(ext)).unwrap();
    }
    for names in [
        [
            "PekerisComplexBlank",
            "TabRefBrcC",
            "TabRefIrcC",
            "FluidTrcC",
        ],
        [
            "FluidTrcC",
            "TabRefIrcC",
            "TabRefBrcC",
            "PekerisComplexBlank",
        ],
    ] {
        let source: String = names
            .iter()
            .map(|name| fs::read_to_string(fixture(name).with_extension("env")).unwrap())
            .collect();
        fs::write(&env, source).unwrap();
        let cases = kraken::legacy::load_field_cases(&env, &flp, ModeSolver::Krakenc).unwrap();
        for (case, name) in cases[0].profiles().iter().zip(names) {
            let expected = kraken::legacy::load_complex_case(
                fixture(name).with_extension("env"),
                fixture(name).with_extension("flp"),
            )
            .unwrap();
            assert_eq!(case.bottom_boundary, expected.bottom_boundary);
            assert_eq!(case.surface_boundary, expected.surface_boundary);
        }
        let output = env.with_extension("h5");
        let process = run(&env, &output, "krakenc", &[]);
        assert!(
            process.status.success(),
            "{}",
            String::from_utf8_lossy(&process.stderr)
        );
        let file = File::open(h5(&output, &env)).unwrap();
        for ext in ["brc", "irc", "trc"] {
            let input = file.group(&format!("inputs/{ext}")).unwrap();
            assert_eq!(
                attribute(&input, "sha256"),
                format!(
                    "{:x}",
                    Sha256::digest(fs::read(env.with_extension(ext)).unwrap())
                )
            );
        }
        let expected = kraken::solve_field(&cases[0]).unwrap();
        let product = SimulationResult {
            modes: expected.modes[0].clone(),
            field: expected.field,
        };
        assert_field(
            &file.group("frequencies/0/field").unwrap(),
            &cases[0].profiles()[0],
            &product,
        );
        file.close().unwrap();
        let old = read_output(&output);
        for ext in ["brc", "irc", "trc"] {
            let path = env.with_extension(ext);
            let bytes = fs::read(&path).unwrap();
            assert_failure(&run(&env, &path, "krakenc", &[]), 4, &path, &bytes);
            fs::remove_file(&path).unwrap();
            assert_failure(&run(&env, &output, "krakenc", &[]), 2, &output, &old);
            fs::write(path, bytes).unwrap();
        }
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn cli_protects_consumed_resources_and_preserves_outputs_on_resource_errors() {
    let root = directory("resources");
    for (name, extension) in [
        ("TabRefBrcC", "brc"),
        ("TabRefIrcC", "irc"),
        ("FluidTrcC", "trc"),
        ("FieldPattern", "sbp"),
    ] {
        let env = root.join(name).with_extension("env");
        let table = env.with_extension(extension);
        for ext in ["env", "flp", extension] {
            fs::copy(fixture(name).with_extension(ext), env.with_extension(ext)).unwrap();
        }
        let bytes = fs::read(&table).unwrap();
        assert_failure(&run(&env, &table, "krakenc", &[]), 4, &table, &bytes);
        #[cfg(unix)]
        {
            let alias = root.join(format!("{name}-alias.h5"));
            std::os::unix::fs::symlink(&table, &alias).unwrap();
            assert_failure(&run(&env, &alias, "krakenc", &[]), 4, &alias, &bytes);
        }
        let output = root.join(name).with_extension("h5");
        let old = b"old complete output";
        old_output(&output, old);
        for bad in [b"0\n".to_vec(), vec![0xff], vec![b' '; 1_048_577]] {
            fs::write(&table, bad).unwrap();
            assert_failure(&run(&env, &output, "krakenc", &[]), 2, &output, old);
        }
        fs::remove_file(&table).unwrap();
        assert_failure(&run(&env, &output, "krakenc", &[]), 2, &output, old);
        if matches!(extension, "brc" | "trc") {
            // Valid finite rows, singular F impedance: failure after HDF5 creation.
            fs::write(&table, "2\n0.0 1.0 0.0\n90.0 1.0 0.0\n").unwrap();
            assert_failure(&run(&env, &output, "krakenc", &[]), 3, &output, old);
        }
        fs::write(&table, &bytes).unwrap();
        assert!(run(&env, &output, "krakenc", &[]).status.success());
        assert_product(&output, &env, &env.with_extension("flp"), "krakenc");
    }
    fs::remove_dir_all(root).unwrap();
}

fn assert_failure(process: &Output, code: i32, output: &Path, old: &[u8]) {
    assert_eq!(
        process.status.code(),
        Some(code),
        "{}",
        String::from_utf8_lossy(&process.stderr)
    );
    assert_eq!(read_output(output), old);
    assert!(!output.join(".pelagic-stage").exists());
    assert!(!output.join(".pelagic-backup").exists());
    assert!(!output.join(".pelagic-lock").exists());
}

#[test]
fn cli_preserves_old_outputs_and_inputs_on_failure() {
    let root = directory("failure");
    let env = root.join("case.env");
    let flp = env.with_extension("flp");
    let source = fs::read_to_string(fixture("PekerisBroadband.env")).unwrap();
    fs::write(&env, &source).unwrap();
    fs::copy(fixture("PekerisBroadband.flp"), &flp).unwrap();
    let output = root.join("result.h5");
    let old = b"previous complete result";
    old_output(&output, old);
    let conflict = h5(&output, &env);
    fs::write(&conflict, old).unwrap();
    assert_failure(&run(&env, &output, "kraken", &[]), 4, &output, old);
    fs::remove_file(conflict).unwrap();
    assert_failure(
        &run(&env, &output, "kraken", &["--max-output-bytes", "100"]),
        4,
        &output,
        old,
    );
    assert_failure(
        &run(&env, &output, "kraken", &["--max-output-bytes", "4096"]),
        4,
        &output,
        old,
    );

    // First frequency succeeds; 5 Hz has no trapped mode. No partial publication.
    fs::write(&env, source.replace("75.0 50.0 62.5 /", "75.0 5.0 62.5 /")).unwrap();
    let process = run(&env, &output, "kraken", &[]);
    assert_failure(&process, 3, &output, old);
    assert!(String::from_utf8_lossy(&process.stderr).contains("frequency[1] (5 Hz)"));
    fs::write(&env, &source).unwrap();
    for input in [&env, &flp] {
        let bytes = fs::read(input).unwrap();
        let process = run(&env, input, "kraken", &[]);
        assert_eq!(process.status.code(), Some(4));
        assert_eq!(fs::read(input).unwrap(), bytes);
    }
    let temporary = output.join(".pelagic-stage");
    fs::write(&temporary, b"someone else's scratch file").unwrap();
    let process = run(&env, &output, "kraken", &[]);
    assert_eq!(process.status.code(), Some(4));
    assert_eq!(
        fs::read(&temporary).unwrap(),
        b"someone else's scratch file"
    );
    assert_eq!(read_output(&output), old);
    fs::remove_file(temporary).unwrap();

    // A quota just below the complete file size also catches cumulative/metadata growth.
    let process = run(&env, &output, "kraken", &[]);
    assert!(
        process.status.success(),
        "{}",
        String::from_utf8_lossy(&process.stderr)
    );
    let complete = read_output(&output);
    let quota = (complete.len() - 1).to_string();
    assert_failure(
        &run(&env, &output, "kraken", &["--max-output-bytes", &quota]),
        4,
        &output,
        &complete,
    );
    fs::write(&env, "unsupported input").unwrap();
    assert_failure(&run(&env, &output, "kraken", &[]), 2, &output, &complete);
    fs::write(&env, [0xff]).unwrap();
    assert_failure(&run(&env, &output, "kraken", &[]), 2, &output, &complete);
    fs::write(&env, vec![b' '; 1_048_577]).unwrap();
    assert_failure(&run(&env, &output, "kraken", &[]), 2, &output, &complete);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn explicit_field_input_and_default_output_work() {
    let root = directory("explicit-flp");
    let env = fixture("PekerisBroadband.env");
    let flp = fixture("PekerisBroadband.flp");
    let process = Command::new(env!("CARGO_BIN_EXE_pelagic"))
        .current_dir(&root)
        .args(["kraken", "run", "--format", "both"])
        .arg(&env)
        .arg("--flp")
        .arg(&flp)
        .output()
        .unwrap();
    assert!(
        process.status.success(),
        "{}",
        String::from_utf8_lossy(&process.stderr)
    );
    assert_product(&root.join("PekerisBroadband"), &env, &flp, "kraken");
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn symlink_input_alias_cannot_be_overwritten() {
    let root = directory("symlink");
    let env = fixture("Pekeris.env");
    let output = root.join("alias.h5");
    std::os::unix::fs::symlink(&env, &output).unwrap();
    let old = fs::read(&env).unwrap();
    let process = run(&env, &output, "kraken", &[]);
    assert_eq!(process.status.code(), Some(4));
    assert_eq!(fs::read(&env).unwrap(), old);
    assert!(output.is_symlink());
    fs::remove_dir_all(root).unwrap();
}

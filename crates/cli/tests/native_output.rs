//! Native readers cross-check every stored value, not only counts or successful file creation.
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    clippy::float_cmp
)]
use bellhop::model::{LegacyArrivalEncoding, ReceiverGrid, RunKind};
use hdf5::{File, Group, H5Type};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
#[path = "support/native.rs"]
#[allow(dead_code)]
mod native;

fn data<T: H5Type>(group: &Group, name: &str) -> Vec<T> {
    group.dataset(name).unwrap().read_raw().unwrap()
}
fn directory(name: &str) -> PathBuf {
    let p = std::env::temp_dir().join(format!("native-output-{name}-{}", std::process::id()));
    fs::create_dir(&p).unwrap();
    p
}
fn run(solver: &str, input: &Path, output: &Path, format: Option<&str>) {
    let mut p = Command::new(env!("CARGO_BIN_EXE_pelagic"));
    p.args([solver, "run"])
        .arg(input)
        .arg("--output")
        .arg(output);
    if let Some(format) = format {
        p.args(["--format", format]);
    }
    let result = p.output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
}

#[allow(clippy::too_many_lines)]
fn bellhop(input: &Path, output: &Path) {
    let case = if input.extension().is_some_and(|e| e == "json") {
        bellhop::json::load_case_document_named(&fs::read(input).unwrap(), input)
            .unwrap()
            .value
    } else {
        bellhop::legacy::load_case(input).unwrap().value
    };
    let stem = input.file_stem().unwrap().to_string_lossy();
    let file = File::open(output.join(format!("{stem}.h5"))).unwrap();
    let env = &case.environment;
    let pos = &env.positions;
    match env.run.kind {
        RunKind::Rays | RunKind::Eigenrays => {
            let text = fs::read_to_string(output.join(format!("{stem}.ray"))).unwrap();
            let mut lines = text.lines();
            lines.next().unwrap();
            let mut tokens = lines.flat_map(str::split_whitespace);
            assert_eq!(
                tokens.next().unwrap().parse::<f64>().unwrap(),
                env.frequency_hz
            );
            for n in [
                1,
                1,
                pos.source_depths_m.len(),
                env.trace
                    .selected_launch_angle
                    .map_or(env.trace.launch_angles_degrees.len(), |_| 1),
                1,
            ] {
                assert_eq!(tokens.next().unwrap().parse::<usize>().unwrap(), n);
            }
            for z in [env.sound_speed.top_depth_m, env.sound_speed.bottom_depth_m] {
                assert_eq!(tokens.next().unwrap().parse::<f64>().unwrap(), z);
            }
            assert_eq!(tokens.next(), Some("'rz'"));
            let group = file
                .group(if env.run.kind == RunKind::Rays {
                    "rays"
                } else {
                    "eigenrays"
                })
                .unwrap();
            let angles = data::<f64>(&group, "launch_angle_degrees");
            let offsets = data::<u64>(&group, "point_offset");
            let top = data::<u32>(&group, "top_bounces");
            let bottom = data::<u32>(&group, "bottom_bounces");
            let ranges = data::<f64>(&group, "range_m");
            let depths = data::<f64>(&group, "depth_m");
            for i in 0..angles.len() {
                assert_eq!(
                    tokens.next().unwrap().parse::<f64>().unwrap().to_bits(),
                    angles[i].to_bits()
                );
                for n in [
                    offsets[i + 1] - offsets[i],
                    u64::from(top[i]),
                    u64::from(bottom[i]),
                ] {
                    assert_eq!(tokens.next().unwrap().parse::<u64>().unwrap(), n);
                }
                for j in offsets[i] as usize..offsets[i + 1] as usize {
                    for value in [ranges[j], depths[j]] {
                        assert_eq!(
                            tokens.next().unwrap().parse::<f64>().unwrap().to_bits(),
                            value.to_bits()
                        );
                    }
                }
            }
            assert!(tokens.next().is_none());
        }
        RunKind::Arrivals => {
            if env.run.receiver_grid == ReceiverGrid::Irregular {
                for i in 0..pos.receiver_ranges_m.len() {
                    arrivals(
                        &output.join(format!("{stem}.r{i:06}.arr")),
                        &case,
                        &file,
                        Some(i),
                    );
                }
            } else {
                arrivals(&output.join(format!("{stem}.arr")), &case, &file, None);
            }
        }
        _ => {
            let g = file.group("field").unwrap();
            let mut real = Vec::new();
            let mut imaginary = Vec::new();
            let stored_real = data::<f32>(&g, "pressure_real");
            let stored_imaginary = data::<f32>(&g, "pressure_imaginary");
            let irregular = env.run.receiver_grid == ReceiverGrid::Irregular;
            for source in 0..pos.source_depths_m.len() {
                for depth in 0..if irregular {
                    1
                } else {
                    pos.receiver_depths_m.len()
                } {
                    for range in 0..pos.receiver_ranges_m.len() {
                        let index = if irregular {
                            source * pos.receiver_ranges_m.len() + range
                        } else {
                            (source * pos.receiver_ranges_m.len() + range)
                                * pos.receiver_depths_m.len()
                                + depth
                        };
                        real.push(stored_real[index]);
                        imaginary.push(stored_imaginary[index]);
                    }
                }
            }
            native::shd(
                &output.join(format!("{stem}.shd")),
                env.frequency_hz,
                &pos.source_depths_m,
                &pos.receiver_depths_m,
                &pos.receiver_ranges_m,
                env.run.receiver_grid == ReceiverGrid::Irregular,
                &real,
                &imaginary,
            );
        }
    }
}

#[allow(clippy::too_many_lines)]
fn arrivals(path: &Path, case: &bellhop::Case, file: &File, selected: Option<usize>) {
    let pos = &case.environment.positions;
    let group = file.group("arrivals").unwrap();
    let binary = case.environment.run.arrival_encoding == Some(LegacyArrivalEncoding::Binary);
    let bytes = fs::read(path).unwrap();
    let records = if binary {
        native::sequential(&bytes)
    } else {
        Vec::new()
    };
    let mut tokens = if binary {
        Vec::new()
    } else {
        let text = std::str::from_utf8(&bytes).unwrap();
        let mut words = text.split_whitespace();
        assert_eq!(words.next(), Some("'2D'"));
        words.map(|w| w.parse::<f64>().unwrap()).collect::<Vec<_>>()
    }
    .into_iter();
    if binary {
        assert_eq!(records[0], b"'2D'");
        assert_eq!(records[1].len(), 4);
        assert_eq!(
            native::f32_at(records[1], 0),
            case.environment.frequency_hz as f32
        );
    } else {
        assert_eq!(tokens.next().unwrap(), case.environment.frequency_hz);
    }
    let depths = selected.map_or(pos.receiver_depths_m.as_slice(), |i| {
        &pos.receiver_depths_m[i..=i]
    });
    let ranges = selected.map_or(pos.receiver_ranges_m.as_slice(), |i| {
        &pos.receiver_ranges_m[i..=i]
    });
    for (r, values, double) in [
        (2, pos.source_depths_m.as_slice(), false),
        (3, depths, false),
        (4, ranges, true),
    ] {
        if binary {
            assert_eq!(native::i32_at(records[r], 0), values.len());
            assert_eq!(
                records[r].len(),
                4 + values.len() * if double { 8 } else { 4 }
            );
        } else {
            assert_eq!(tokens.next().unwrap() as usize, values.len());
        }
        for (i, &value) in values.iter().enumerate() {
            let expected = if double {
                value
            } else {
                f64::from(value as f32)
            };
            let actual = if binary {
                if double {
                    native::f64_at(records[r], 4 + 8 * i)
                } else {
                    f64::from(native::f32_at(records[r], 4 + 4 * i))
                }
            } else {
                tokens.next().unwrap()
            };
            assert_eq!(actual, expected);
        }
    }
    let receiver_offsets = data::<u64>(&group, "receiver_offset");
    let offsets = data::<u64>(&group, "arrival_offset");
    let columns: Vec<Vec<f32>> = [
        "amplitude",
        "phase_radians",
        "travel_time_s",
        "attenuation_time_s",
        "source_angle_degrees",
        "receiver_angle_degrees",
    ]
    .iter()
    .map(|n| data(&group, n))
    .collect();
    let top = data::<u32>(&group, "top_bounces");
    let bottom = data::<u32>(&group, "bottom_bounces");
    let mut record = 5;
    assert_eq!(receiver_offsets.len(), pos.source_depths_m.len() + 1);
    for &base in receiver_offsets.iter().take(pos.source_depths_m.len()) {
        let base = base as usize;
        let receivers: Vec<_> = if let Some(receiver) = selected {
            vec![base + receiver]
        } else {
            (0..pos.receiver_depths_m.len())
                .flat_map(|z| {
                    (0..pos.receiver_ranges_m.len())
                        .map(move |r| base + r * pos.receiver_depths_m.len() + z)
                })
                .collect()
        };
        let maximum = receivers
            .iter()
            .map(|&i| offsets[i + 1] - offsets[i])
            .max()
            .unwrap_or(0) as usize;
        let actual = if binary {
            let n = native::i32_at(records[record], 0);
            assert_eq!(records[record].len(), 4);
            record += 1;
            n
        } else {
            tokens.next().unwrap() as usize
        };
        assert_eq!(actual, maximum);
        for receiver in receivers {
            let expected = (offsets[receiver + 1] - offsets[receiver]) as usize;
            let actual = if binary {
                let n = native::i32_at(records[record], 0);
                assert_eq!(records[record].len(), 4);
                record += 1;
                n
            } else {
                tokens.next().unwrap() as usize
            };
            assert_eq!(actual, expected);
            for i in offsets[receiver] as usize..offsets[receiver + 1] as usize {
                let expected = [
                    columns[0][i],
                    columns[1][i].to_degrees(),
                    columns[2][i],
                    columns[3][i],
                    columns[4][i],
                    columns[5][i],
                    top[i] as f32,
                    bottom[i] as f32,
                ];
                if binary {
                    assert_eq!(records[record].len(), 32);
                }
                for (j, value) in expected.into_iter().enumerate() {
                    let actual = if binary {
                        native::f32_at(records[record], 4 * j)
                    } else {
                        tokens.next().unwrap() as f32
                    };
                    assert_eq!(actual.to_bits(), value.to_bits());
                }
                if binary {
                    record += 1;
                }
            }
        }
    }
    if binary {
        assert_eq!(record, records.len());
    } else {
        assert!(tokens.next().is_none());
    }
}

#[test]
fn bellhop_full_native_layouts_match_every_hdf5_value() {
    let root = directory("bellhop");
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("../bellhop/tests/fixtures/golden");
    for entry in fs::read_dir(&fixtures).unwrap() {
        let input = entry.unwrap().path();
        if input.extension().is_none_or(|e| e != "env") {
            continue;
        }
        let output = root.join(input.file_stem().unwrap());
        run("bellhop", &input, &output, Some("both"));
        bellhop(&input, &output);
    }
    // Binary arrivals, irregular ARR/SHD and multiple sources are accepted typed inputs too.
    let original = bellhop::legacy::load_case(&fixtures.join("GeoHat_arrival.env"))
        .unwrap()
        .value;
    for (name, kind) in [
        ("json-arr", RunKind::Arrivals),
        ("rect-arr", RunKind::Arrivals),
        ("rect-arr-ascii", RunKind::Arrivals),
        ("irregular-arr", RunKind::Arrivals),
        ("irregular-arr-ascii", RunKind::Arrivals),
        ("irregular-field", RunKind::Coherent),
    ] {
        let mut definition = original.clone().into_definition();
        definition.environment.run.kind = kind;
        definition.environment.run.arrival_encoding = if kind == RunKind::Arrivals {
            Some(if name.ends_with("ascii") {
                LegacyArrivalEncoding::Ascii
            } else {
                LegacyArrivalEncoding::Binary
            })
        } else {
            None
        };
        if name.starts_with("rect-arr") {
            definition.environment.positions.source_depths_m = vec![40.0, 50.0];
            definition.environment.positions.receiver_depths_m = vec![35.0, 50.0, 65.0];
            definition.environment.positions.receiver_ranges_m = vec![500.0, 1000.0];
        }
        if name.starts_with("irregular") {
            definition.environment.run.receiver_grid = ReceiverGrid::Irregular;
            definition.environment.positions.source_depths_m = vec![40.0, 50.0];
            definition.environment.positions.receiver_depths_m = vec![40.0, 50.0, 60.0];
            definition.environment.positions.receiver_ranges_m = vec![500.0, 750.0, 1000.0];
        }
        let case = bellhop::Case::from_definition(definition).unwrap().value;
        let input = root.join(format!("{name}.json"));
        let document = bellhop::json::export_case_document(&case).unwrap();
        fs::write(&input, serde_json::to_vec(&document).unwrap()).unwrap();
        let output = root.join(name);
        run("bellhop", &input, &output, Some("both"));
        bellhop(&input, &output);
    }
    for (name, receivers, options) in [
        ("binary-rect", "3\n35.0 50.0 65.0 /\n2\n0.5 1.0 /", "'a'"),
        (
            "binary-irregular",
            "3\n40.0 50.0 60.0 /\n3\n0.5 0.75 1.0 /",
            "'a   I'",
        ),
    ] {
        let source = fs::read_to_string(fixtures.join("GeoHat_arrival.env"))
            .unwrap()
            .replace(
                "1\n50.0 /\n1\n50.0 /\n1\n1.0 /",
                &format!("2\n40.0 50.0 /\n{receivers}"),
            )
            .replace("'A'", options);
        let input = root.join(format!("{name}.env"));
        fs::write(&input, source).unwrap();
        assert_eq!(
            bellhop::legacy::load_case(&input)
                .unwrap()
                .value
                .environment
                .run
                .arrival_encoding,
            Some(LegacyArrivalEncoding::Binary)
        );
        let output = root.join(name);
        run("bellhop", &input, &output, Some("both"));
        bellhop(&input, &output);
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn selected_ray_header_dimensions_match_complete_multisource_body() {
    let root = directory("selected-rays");
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../bellhop/tests/fixtures/golden/N2_one_ray.env");
    let case = bellhop::legacy::load_case(&fixture).unwrap().value;
    let mut document = bellhop::json::export_case_document(&case).unwrap();
    document.bellhop.angles = bellhop::json::grid::Grid::Values(vec![-5.0, 0.0, 5.0]);
    document.source.depth = bellhop::json::grid::Grid::Values(vec![40.0, 50.0]);
    for selected in [Some(1), Some(2), Some(3), None] {
        document.bellhop.selected_angle = selected;
        let input = root.join(format!("selected-{selected:?}.json"));
        fs::write(&input, serde_json::to_vec(&document).unwrap()).unwrap();
        let output = root.join(format!("results-{selected:?}"));
        run("bellhop", &input, &output, Some("both"));
        let stem = input.file_stem().unwrap().to_string_lossy();
        let ray = fs::read_to_string(output.join(format!("{stem}.ray"))).unwrap();
        let lines: Vec<_> = ray.lines().collect();
        let sources = lines[2]
            .split_whitespace()
            .last()
            .unwrap()
            .parse::<usize>()
            .unwrap();
        let angles = lines[3]
            .split_whitespace()
            .next()
            .unwrap()
            .parse::<usize>()
            .unwrap();
        assert_eq!(sources, 2);
        assert_eq!(angles, if selected.is_some() { 1 } else { 3 });
        let mut body = lines[7..].iter();
        for _ in 0..sources * angles {
            body.next()
                .expect("missing launch angle")
                .parse::<f64>()
                .unwrap();
            let points = body
                .next()
                .unwrap()
                .split_whitespace()
                .next()
                .unwrap()
                .parse::<usize>()
                .unwrap();
            for _ in 0..points {
                body.next().expect("incomplete trajectory");
            }
        }
        assert!(body.next().is_none(), "undeclared trajectories");
        bellhop(&input, &output);
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn default_legacy_and_three_formats_share_directory_semantics() {
    let root = directory("defaults");
    let input = Path::new(env!("CARGO_MANIFEST_DIR")).join("../kraken/tests/fixtures/Pekeris.env");
    let output = root.join("nested/results");
    run("kraken", &input, &output, None);
    assert!(output.join("Pekeris.mod").is_file());
    assert!(output.join("Pekeris.shd").is_file());
    assert!(!output.join("Pekeris.h5").exists());
    fs::write(output.join("notes.txt"), b"unrelated").unwrap();
    for format in ["hdf5", "both", "legacy"] {
        run("kraken", &input, &output, Some(format));
        assert_eq!(output.join("Pekeris.h5").exists(), format != "legacy");
        assert_eq!(output.join("Pekeris.mod").exists(), format != "hdf5");
        assert_eq!(fs::read(output.join("notes.txt")).unwrap(), b"unrelated");
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn repeated_frequencies_offsets_and_profiles_remain_native_readable() {
    let root = directory("geometry");
    let input = Path::new(env!("CARGO_MANIFEST_DIR")).join("../kraken/tests/fixtures/Pekeris.env");
    let legacy = kraken::input::load_legacy(
        &input,
        &input.with_extension("flp"),
        kraken::ModeSolver::Kraken,
    )
    .unwrap();
    let mut document = serde_json::to_value(legacy.document().unwrap()).unwrap();
    document["frequency"] = serde_json::json!([50.0, 75.0, 50.0]);
    document["source"]["depth"] = serde_json::json!([40.0, 50.0]);
    document["receivers"]["depth"] = serde_json::json!([0.0, 80.0]);
    document["receivers"]["offset"] = serde_json::json!([0.0, 20.0]);
    document["kraken"]["propagation"] = serde_json::json!("adiabatic");
    let mut bottom = document["bottom"].clone();
    bottom["ap"] = serde_json::json!(0.001);
    document["profiles"] = serde_json::json!([
        {"range": 500.0, "title": "second profile", "bottom": bottom}
    ]);
    let path = root.join("different.json");
    fs::write(&path, serde_json::to_vec(&document).unwrap()).unwrap();
    let output = root.join("results");
    run("kraken", &path, &output, Some("both"));
    native::kraken(&output, &path);
    for i in 0..3 {
        assert!(output.join(format!("different.f{i:04}.mod")).is_file());
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
#[ignore = "requires the fresh reference job's actual CLI artifacts"]
fn actual_cli_native_layouts_match_all_hdf5_values() {
    if let Some(input) = std::env::var_os("BELLHOP_NATIVE_INPUT") {
        let output = PathBuf::from(std::env::var_os("BELLHOP_NATIVE_OUTPUT").unwrap());
        bellhop(&PathBuf::from(input), &output);
        return;
    }
    let file = PathBuf::from(std::env::var_os("KRAKEN_HDF5_RESULT").unwrap());
    let input = PathBuf::from(
        std::env::var_os("KRAKEN_DIFFERENTIAL_ENV")
            .or_else(|| std::env::var_os("KRAKEN_JSON_INPUT"))
            .unwrap(),
    );
    native::kraken(file.parent().unwrap(), &input);
}

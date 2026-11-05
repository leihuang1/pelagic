//! The rules ENV and JSON share through Input material: both loaders must give the same verdict on
//! each input here. Each keeps its own diagnostic code and wording, pinned as they are. The tests
//! named `known_difference_*` pin where the two inputs do not agree.
use std::path::Path;

use kraken::{Case, DiagnosticReport, FieldCase, ModeSolver, input, json, legacy};
use serde_json::{Value, json};

const ENV: &str = "'parity'\n50\n1\n'NVW'\n1000 0 100\n0 1500 0 1 0 0 /\n100 1500 0 1 0 0 /\n'A' 0\n100 1700 0 1.5 0.5 0 /\n1400 1700\n0\n1\n50 /\n1\n50 /\n";
const FLP: &str = "/\n'R OC'\n9999\n1\n0 /\n1\n1 /\n1\n50 /\n1\n50 /\n1\n0 /\n";
const ROW_0: &str = "0 1500 0 1 0 0 /";
const ROW_1: &str = "100 1500 0 1 0 0 /";
const HALF_SPACE: &str = "100 1700 0 1.5 0.5 0 /";

fn document() -> Value {
    json!({
      "pelagic": 2, "model": "kraken", "title": "parity", "frequency": 50,
      "interpolation": "n2_linear",
      "media": [{"to": 100, "mesh": 1000, "ssp": [[0, 1500, 0, 1, 0, 0], [100, 1500, 0, 1, 0, 0]]}],
      "surface": "vacuum",
      "bottom": {"type": "halfspace", "cp": 1700, "rho": 1.5, "ap": 0.5},
      "source": {"depth": 50}, "receivers": {"depth": 50, "range": 1000},
      "kraken": {"solver": "kraken", "phase_speed": [1400, 1700], "max_range": 0, "mode_limit": 9999}
    })
}

fn edited_env(source: &str, edits: &[(&str, &str)]) -> String {
    edits.iter().fold(source.to_owned(), |source, (from, to)| {
        assert!(source.contains(from), "{from:?} is not in {source:?}");
        source.replacen(from, to, 1)
    })
}

fn edited_document(edits: Vec<(&str, Value)>) -> Value {
    let mut document = document();
    for (pointer, value) in edits {
        if let Some(slot) = document.pointer_mut(pointer) {
            *slot = value;
        } else {
            let (parent, key) = pointer.rsplit_once('/').unwrap();
            document.pointer_mut(parent).unwrap()[key] = value;
        }
    }
    document
}

fn load_env(env: &str, flp: &str) -> Result<Vec<Case>, DiagnosticReport> {
    legacy::load_frequency_cases_from_sources(
        env,
        flp,
        Path::new("case.env"),
        Path::new("case.flp"),
        ModeSolver::Kraken,
    )
}

fn load_document(document: &Value) -> Result<Vec<FieldCase>, DiagnosticReport> {
    json::load_case_document(&serde_json::to_vec(document).unwrap())
}

fn assert_same_cases(cases: Vec<Case>, document: &Value) -> Vec<Case> {
    let profiles: Vec<_> = load_document(document)
        .unwrap()
        .iter()
        .map(|case| case.profiles()[0].clone())
        .collect();
    assert_eq!(profiles, cases);
    cases
}

/// One input, as ENV source and as a JSON document.
struct Input {
    name: &'static str,
    env: String,
    document: Value,
}

fn input(name: &'static str, env: &[(&str, &str)], document: Vec<(&str, Value)>) -> Input {
    Input {
        name,
        env: edited_env(ENV, env),
        document: edited_document(document),
    }
}

/// A rule, the code and wording each loader reports it with, and inputs that break each clause.
struct Rule {
    env: (&'static str, &'static str),
    json: (&'static str, &'static str),
    inputs: Vec<Input>,
}

fn francois_garrison(name: &'static str, [t, s, ph, depth]: [f64; 4]) -> Input {
    input(
        name,
        &[("'NVW'", &format!("'NVWF'\n{t} {s} {ph} {depth}"))],
        vec![(
            "/attenuation",
            json!({"volume": {
                "type": "francois_garrison",
                "temperature": t, "salinity": s, "ph": ph, "depth": depth
            }}),
        )],
    )
}

fn biological(name: &'static str, layer: [f64; 5]) -> Input {
    let [top, bottom, resonance, q, a0] = layer;
    input(
        name,
        &[(
            "'NVW'",
            &format!("'NVWB'\n1\n{top} {bottom} {resonance} {q} {a0}"),
        )],
        vec![(
            "/attenuation",
            json!({"volume": {"type": "biological", "layers": [layer]}}),
        )],
    )
}

fn power_law(name: &'static str, [beta, transition]: [f64; 2]) -> Input {
    input(
        name,
        &[
            ("'NVW'", "'NVm'"),
            ("1000 0 100\n", &format!("1000 0 100 {beta} {transition}\n")),
            ("'A' 0\n", "'A' 0 1 1000\n"),
            (ROW_0, "0 1500 0 1 0.1 0 /"),
            (ROW_1, "100 1500 0 1 0.1 0 /"),
        ],
        vec![
            (
                "/attenuation",
                json!({"unit": "db_per_meter_power_law", "reference_frequency": 50}),
            ),
            ("/media/0/ssp/0/4", json!(0.1)),
            ("/media/0/ssp/1/4", json!(0.1)),
            ("/media/0/power_law", json!([beta, transition])),
            ("/bottom/power_law", json!([1, 1000])),
        ],
    )
}

#[allow(clippy::too_many_lines)]
fn rules() -> Vec<Rule> {
    vec![
        Rule {
            env: ("KR0102", "depths must increase"),
            json: ("KR0201", "depths must increase"),
            inputs: vec![
                input(
                    "row above the medium's top",
                    &[(ROW_0, "-1 1500 0 1 0 0 /")],
                    vec![("/media/0/ssp/0/0", json!(-1))],
                ),
                input(
                    "row below the medium's bottom",
                    &[(ROW_1, "150 1500 0 1 0 0 /")],
                    vec![("/media/0/ssp/1/0", json!(150))],
                ),
                input(
                    "repeated row depth",
                    &[(ROW_1, "0 1500 0 1 0 0 /")],
                    vec![("/media/0/ssp/1/0", json!(0))],
                ),
            ],
        },
        Rule {
            env: ("KR0102", "constant-density fluid"),
            json: ("KR0201", "constant-density fluid"),
            inputs: vec![
                input(
                    "first row below the medium's top",
                    &[(ROW_0, "10 1500 0 1 0 0 /")],
                    vec![("/media/0/ssp/0/0", json!(10))],
                ),
                input(
                    "negative shear speed",
                    &[(ROW_0, "0 1500 -1 1 0 0 /")],
                    vec![("/media/0/ssp/0/2", json!(-1))],
                ),
                input(
                    "negative compressional loss",
                    &[(ROW_0, "0 1500 0 1 -1 0 /")],
                    vec![("/media/0/ssp/0/4", json!(-1))],
                ),
                input(
                    "negative shear loss",
                    &[
                        (ROW_0, "0 1500 100 1 0 -1 /"),
                        (ROW_1, "100 1500 100 1 0 0 /"),
                    ],
                    vec![
                        ("/media/0/ssp/0/2", json!(100)),
                        ("/media/0/ssp/0/5", json!(-1)),
                        ("/media/0/ssp/1/2", json!(100)),
                    ],
                ),
                input(
                    "shear loss without shear speed",
                    &[(ROW_0, "0 1500 0 1 0 0.5 /")],
                    vec![("/media/0/ssp/0/5", json!(0.5))],
                ),
                input(
                    "fluid density changes",
                    &[(ROW_1, "100 1500 0 2 0 0 /")],
                    vec![("/media/0/ssp/1/3", json!(2))],
                ),
                input(
                    "fluid gains shear speed",
                    &[(ROW_1, "100 1500 100 1 0 0 /")],
                    vec![("/media/0/ssp/1/2", json!(100))],
                ),
                input(
                    "solid loses shear speed",
                    &[(ROW_0, "0 1500 100 1 0 0 /")],
                    vec![("/media/0/ssp/0/2", json!(100))],
                ),
            ],
        },
        Rule {
            env: ("KR0102", "require half-space at the boundary"),
            json: ("KR0201", "require positive cp and rho"),
            inputs: vec![
                input(
                    "zero compressional speed",
                    &[(HALF_SPACE, "100 0 0 1.5 0.5 0 /")],
                    vec![("/bottom/cp", json!(0))],
                ),
                input(
                    "negative shear speed",
                    &[(HALF_SPACE, "100 1700 -1 1.5 0.5 0 /")],
                    vec![("/bottom/cs", json!(-1))],
                ),
                input(
                    "zero density",
                    &[(HALF_SPACE, "100 1700 0 0 0.5 0 /")],
                    vec![("/bottom/rho", json!(0))],
                ),
                input(
                    "negative compressional loss",
                    &[(HALF_SPACE, "100 1700 0 1.5 -1 0 /")],
                    vec![("/bottom/ap", json!(-1))],
                ),
                input(
                    "negative shear loss",
                    &[(HALF_SPACE, "100 1700 100 1.5 0.5 -1 /")],
                    vec![("/bottom/cs", json!(100)), ("/bottom/as", json!(-1))],
                ),
                input(
                    "shear loss without shear speed",
                    &[(HALF_SPACE, "100 1700 0 1.5 0.5 0.5 /")],
                    vec![("/bottom/as", json!(0.5))],
                ),
                input(
                    "surface half-space",
                    &[("'NVW'", "'NAW'\n0 -1600 0 1 0 0 /")],
                    vec![(
                        "/surface",
                        json!({"type": "halfspace", "cp": -1600, "rho": 1}),
                    )],
                ),
            ],
        },
        Rule {
            env: ("KR0202", "finite fluids must be contiguous"),
            json: ("KR0201", "finite fluids must be contiguous"),
            inputs: vec![input(
                "fluid below a bottom solid",
                &[
                    ("1\n'NVW'", "3\n'NVW'"),
                    (
                        ROW_1,
                        "100 1500 0 1 0 0 /\n1000 0 200\n100 1800 100 1.8 0 0 /\n200 1800 100 1.8 0 0 /\n1000 0 300\n200 1500 0 1 0 0 /\n300 1500 0 1 0 0 /",
                    ),
                ],
                vec![(
                    "/media",
                    json!([
                        {"to": 100, "mesh": 1000, "ssp": [[0, 1500], [100, 1500]]},
                        {"to": 200, "mesh": 1000, "ssp": [[100, 1800, 100, 1.8], [200, 1800, 100, 1.8]]},
                        {"to": 300, "mesh": 1000, "ssp": [[200, 1500], [300, 1500]]},
                    ]),
                )],
            )],
        },
        Rule {
            env: ("KR0202", "requires at least one finite fluid"),
            json: ("KR0201", "requires at least one finite fluid"),
            inputs: vec![input(
                "every medium is a solid",
                &[
                    (ROW_0, "0 1500 100 1 0 0 /"),
                    (ROW_1, "100 1500 100 1 0 0 /"),
                ],
                vec![
                    ("/media/0/ssp/0/2", json!(100)),
                    ("/media/0/ssp/1/2", json!(100)),
                ],
            )],
        },
        Rule {
            env: ("KR0201", "elastic half-space requires"),
            json: ("KR0201", "elastic half-space requires"),
            inputs: vec![
                input(
                    "bottom shear speed too close to its compressional speed",
                    &[(HALF_SPACE, "100 1700 1500 1.5 0.5 0 /")],
                    vec![("/bottom/cs", json!(1500))],
                ),
                input(
                    "surface shear speed too close to its compressional speed",
                    &[("'NVW'", "'NAW'\n0 1600 1500 1 0 0 /")],
                    vec![(
                        "/surface",
                        json!({"type": "halfspace", "cp": 1600, "cs": 1500, "rho": 1}),
                    )],
                ),
            ],
        },
        Rule {
            env: ("KR0201", "salinity >= 0"),
            json: ("KR0201", "salinity >= 0"),
            inputs: vec![
                francois_garrison("temperature at absolute zero", [-273.0, 35.0, 8.0, 1000.0]),
                francois_garrison("negative salinity", [4.0, -1.0, 8.0, 1000.0]),
                francois_garrison("negative pH", [4.0, 35.0, -1.0, 1000.0]),
                francois_garrison("pH above 14", [4.0, 35.0, 15.0, 1000.0]),
                francois_garrison("negative mean depth", [4.0, 35.0, 8.0, -1.0]),
            ],
        },
        Rule {
            env: ("KR0201", "positive resonance/Q"),
            json: ("KR0201", "positive resonance/Q"),
            inputs: vec![
                biological("negative layer top", [-1.0, 50.0, 1000.0, 5.0, 0.01]),
                biological(
                    "layer bottom above its top",
                    [50.0, 40.0, 1000.0, 5.0, 0.01],
                ),
                biological("zero resonance", [0.0, 50.0, 0.0, 5.0, 0.01]),
                biological("zero Q", [0.0, 50.0, 1000.0, 0.0, 0.01]),
                biological("negative a0", [0.0, 50.0, 1000.0, 5.0, -1.0]),
            ],
        },
        Rule {
            env: ("KR0201", "at most 200 biological layers"),
            json: ("KR0201", "at most 200 biological layers"),
            inputs: vec![input(
                "too many biological layers",
                &[("'NVW'", "'NVWB'\n201")],
                vec![(
                    "/attenuation",
                    json!({"volume": {
                        "type": "biological",
                        "layers": vec![[0.0, 50.0, 1000.0, 5.0, 0.01]; 201]
                    }}),
                )],
            )],
        },
        Rule {
            env: ("KR0102", "require beta >= 0"),
            json: ("KR0201", "require beta >= 0"),
            inputs: vec![
                power_law("negative beta", [-1.0, 1000.0]),
                power_law("zero transition frequency", [1.0, 0.0]),
            ],
        },
        Rule {
            env: ("KR0302", "scaled or refined mesh"),
            json: ("KR0302", "scaled or refined mesh"),
            inputs: vec![input(
                "mesh reference scales the mesh past its limit",
                &[
                    ("'NVW'", "'NVW  B'"),
                    ("1\n50 /\n1\n50 /\n", "1\n50 /\n1\n50 /\n1\n1000000 /\n"),
                ],
                vec![
                    ("/frequency", json!(1_000_000)),
                    ("/kraken/mesh_frequency", json!(50)),
                ],
            )],
        },
    ]
}

#[test]
fn both_inputs_build_the_same_case_from_the_valid_starting_point() {
    let cases = assert_same_cases(load_env(ENV, FLP).unwrap(), &document());
    assert_eq!(cases.len(), 1);
}

#[test]
fn both_inputs_reject_what_breaks_a_shared_rule_with_their_own_codes() {
    for rule in rules() {
        for input in &rule.inputs {
            for (loader, report, (code, message)) in [
                ("ENV", load_env(&input.env, FLP).err(), rule.env),
                ("JSON", load_document(&input.document).err(), rule.json),
            ] {
                let name = input.name;
                let report = report.unwrap_or_else(|| panic!("{name}: {loader} accepted it"));
                let first = &report.diagnostics()[0];
                assert_eq!(first.code, code, "{name}: {loader}: {report}");
                assert!(
                    first.message.contains(message),
                    "{name}: {loader}: {report}"
                );
            }
        }
    }
}

#[test]
fn both_inputs_accept_and_build_the_same_case_at_each_rule_limit() {
    let layer = "0 50 1000 5 0.01\n";
    for limit in [
        francois_garrison("zero salinity", [4.0, 0.0, 8.0, 1000.0]),
        francois_garrison("pH 0", [4.0, 35.0, 0.0, 1000.0]),
        francois_garrison("pH 14", [4.0, 35.0, 14.0, 1000.0]),
        francois_garrison("zero mean depth", [4.0, 35.0, 8.0, 0.0]),
        biological("zero top and thickness", [0.0, 0.0, 1000.0, 5.0, 0.01]),
        biological("zero a0", [0.0, 50.0, 1000.0, 5.0, 0.0]),
        input(
            "exactly 200 biological layers",
            &[(
                "'NVW'",
                &format!("'NVWB'\n200\n{}", layer.repeat(200).trim_end()),
            )],
            vec![(
                "/attenuation",
                json!({"volume": {
                    "type": "biological",
                    "layers": vec![[0.0, 50.0, 1000.0, 5.0, 0.01]; 200]
                }}),
            )],
        ),
        power_law("zero beta", [0.0, 1000.0]),
        input(
            "half-space without compressional loss",
            &[(HALF_SPACE, "100 1700 0 1.5 0 0 /")],
            vec![("/bottom/ap", json!(0))],
        ),
    ] {
        let name = limit.name;
        let cases =
            load_env(&limit.env, FLP).unwrap_or_else(|report| panic!("{name}: ENV: {report}"));
        assert_same_cases(cases, &limit.document);
    }
}

#[test]
fn only_env_places_a_half_space_at_its_boundary_depth() {
    let report = load_env(
        &edited_env(ENV, &[(HALF_SPACE, "90 1700 0 1.5 0.5 0 /")]),
        FLP,
    )
    .unwrap_err();
    assert_eq!(report.diagnostics()[0].code, "KR0102", "{report}");
    assert!(
        report.diagnostics()[0]
            .message
            .contains("require half-space at the boundary"),
        "{report}"
    );
}

#[test]
fn both_inputs_spend_the_same_per_frequency_storage_budget() {
    let env = edited_env(
        ENV,
        &[
            ("'NVW'", "'NVW  B'"),
            ("1000 0 100\n", "1000 1 100\n"),
            ("1\n50 /\n1\n50 /\n", "1\n1 /\n1\n99 /\n1000\n1 1000 /\n"),
        ],
    );
    // 4 water values, 2 interface roughnesses, 2 mode depths, 1 source, 1 range, 4 pattern values
    // and 2 values for each receiver fill exactly 5000 values at each of 1000 frequencies.
    let receivers = 2493;
    let flp = edited_env(
        FLP,
        &[
            ("'R OC'", "'R *C'"),
            (
                "1\n50 /\n1\n50 /\n1\n0 /\n",
                &format!("1\n50 /\n{receivers}\n1 99 /\n{receivers}\n0 /\n"),
            ),
        ],
    );
    let mut document = edited_document(vec![
        ("/frequency", json!({"from": 1, "to": 1000, "count": 1000})),
        ("/surface", json!({"type": "vacuum", "roughness": 1})),
        ("/source/pattern", json!([[-90, 0], [90, 0]])),
        (
            "/receivers/depth",
            json!({"from": 1, "to": 99, "count": receivers}),
        ),
        ("/kraken/mode_depths", json!([1, 99])),
        ("/kraken/mesh_frequency", json!(50)),
    ]);
    let load_env = |flp: &str| {
        legacy::load_frequency_cases_with_resources(
            &env,
            flp,
            Path::new("case.env"),
            Path::new("case.flp"),
            ModeSolver::Kraken,
            None,
            None,
            Some("2\n-90 0\n90 0\n"),
        )
    };
    assert_eq!(
        assert_same_cases(load_env(&flp).unwrap(), &document).len(),
        1000
    );

    // One more range is one value too many at each frequency.
    document["receivers"]["range"] = json!([1000, 2000]);
    for report in [
        load_env(&flp.replacen("1\n1 /", "2\n1 2 /", 1)).unwrap_err(),
        load_document(&document).unwrap_err(),
    ] {
        assert_eq!(report.diagnostics()[0].code, "KR0201", "{report}");
        assert!(
            report.diagnostics()[0]
                .message
                .contains("input storage limit"),
            "{report}"
        );
    }
}

#[test]
fn both_inputs_keep_samples_on_an_interface_whose_single_precision_spelling_rounds_up() {
    // 0.1_f32 is above 0.1_f64, so an unsnapped sample would fall outside the water.
    let flp = edited_env(
        FLP,
        &[("1\n50 /\n1\n50 /\n1\n0 /\n", "1\n0.1 /\n1\n0.1 /\n1\n0 /\n")],
    );
    let bottom = assert_same_cases(
        load_env(
            &edited_env(
                ENV,
                &[
                    ("1000 0 100\n", "1000 0 0.1\n"),
                    (ROW_1, "0.1 1500 0 1 0 0 /"),
                    (HALF_SPACE, "0.1 1700 0 1.5 0.5 0 /"),
                    ("1\n50 /\n1\n50 /\n", "1\n0.1 /\n1\n0.1 /\n"),
                ],
            ),
            &flp,
        )
        .unwrap(),
        &edited_document(vec![
            ("/media/0/to", json!(0.1)),
            ("/media/0/ssp/1/0", json!(0.1)),
            ("/source/depth", json!(0.1)),
            ("/receivers/depth", json!(0.1)),
        ]),
    );
    assert_eq!(bottom[0].source_depths_m, [0.1]);
    assert_eq!(bottom[0].receiver_depths_m, [0.1]);
    assert_eq!(bottom[0].mode_sample_depths_m, [0.1]);

    let top = assert_same_cases(
        load_env(
            &edited_env(
                ENV,
                &[
                    ("1\n'NVW'", "2\n'NVW'"),
                    (
                        "1000 0 100\n0 1500 0 1 0 0 /\n",
                        "1000 0 0.1\n0 1800 100 1.8 0 0 /\n0.1 1800 100 1.8 0 0 /\n1000 0 100\n0.1 1500 0 1 0 0 /\n",
                    ),
                    ("1\n50 /\n1\n50 /\n", "1\n0.1 /\n1\n0.1 /\n"),
                ],
            ),
            &flp,
        )
        .unwrap(),
        &edited_document(vec![
            (
                "/media",
                json!([
                    {"to": 0.1, "mesh": 1000, "ssp": [[0, 1800, 100, 1.8], [0.1, 1800, 100, 1.8]]},
                    {"to": 100, "mesh": 1000, "ssp": [[0.1, 1500], [100, 1500]]},
                ]),
            ),
            ("/source/depth", json!(0.1)),
            ("/receivers/depth", json!(0.1)),
        ]),
    );
    assert_eq!(top[0].source_depths_m, [0.1]);
    assert_eq!(top[0].receiver_depths_m, [0.1]);
    assert_eq!(top[0].mode_sample_depths_m, [0.1]);
}

// Known differences: existing behaviour, not a design. Changing one changes which published inputs
// load, so it needs a schema decision and must not be "fixed" in passing.

fn intake(name: &str, files: &[(&str, &str)]) -> Result<input::FieldInput, input::InputError> {
    let directory =
        std::env::temp_dir().join(format!("kraken-parity-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    for (file, source) in files {
        std::fs::write(directory.join(file), source).unwrap();
    }
    let loaded = input::load(&directory.join(files[0].0), None, None);
    std::fs::remove_dir_all(directory).unwrap();
    loaded
}

/// A broadband input of 1000 frequencies: two water rows with loss `ap`, two mode depths, one
/// source, `ranges` ranges and `receivers` receivers with an offset each, repeated per profile.
struct Sequence {
    ap: f64,
    profiles: usize,
    ranges: usize,
    receivers: usize,
}

impl Sequence {
    fn env(&self) -> String {
        let ap = self.ap;
        edited_env(
            ENV,
            &[
                ("'NVW'", "'NVW  B'"),
                (ROW_0, &format!("0 1500 0 1 {ap} 0 /")),
                (ROW_1, &format!("100 1500 0 1 {ap} 0 /")),
                ("1\n50 /\n1\n50 /\n", "1\n1 /\n1\n99 /\n1000\n1 1000 /\n"),
            ],
        )
        .repeat(self.profiles)
    }

    fn flp(&self) -> String {
        let Self {
            profiles,
            ranges,
            receivers,
            ..
        } = self;
        let (options, profile_ranges) = if *profiles == 1 {
            ("'R OC'", "0 /")
        } else {
            ("'RAOC'", "0 1 /")
        };
        let receiver_ranges = if *ranges == 1 { "1 /" } else { "1 2 /" };
        format!(
            "/\n{options}\n9999\n{profiles}\n{profile_ranges}\n{ranges}\n{receiver_ranges}\n1\n50 /\n{receivers}\n1 99 /\n{receivers}\n0 /\n"
        )
    }

    fn document(&self) -> String {
        let mut edits = vec![
            ("/frequency", json!({"from": 1, "to": 1000, "count": 1000})),
            ("/media/0/ssp/0/4", json!(self.ap)),
            ("/media/0/ssp/1/4", json!(self.ap)),
            (
                "/receivers/depth",
                json!({"from": 1, "to": 99, "count": self.receivers}),
            ),
            (
                "/receivers/range",
                json!((1..=self.ranges).map(|km| km * 1000).collect::<Vec<_>>()),
            ),
            ("/kraken/mode_depths", json!([1, 99])),
            ("/kraken/mesh_frequency", json!(50)),
        ];
        if self.profiles > 1 {
            edits.push(("/kraken/propagation", json!("adiabatic")));
            edits.push(("/profiles", json!([{"range": 1000}])));
        }
        edited_document(edits).to_string()
    }
}

fn assert_over_cumulative_budget(report: &DiagnosticReport, field: &str) {
    let first = &report.diagnostics()[0];
    assert_eq!(first.code, "KR0201", "{report}");
    assert_eq!(first.field, field, "{report}");
    assert!(
        first.message.contains("cumulative input storage limit"),
        "{report}"
    );
}

#[test]
fn known_difference_env_accepts_a_sequence_that_json_counts_over_the_budget() {
    // V = 4 water + 2 modes + 1 source + 2 ranges + 2 * 1246 = 2501 raw values. Lossless water has
    // no canonical loss values, so C = 2499. JSON: 2 * 2501 * 1000 = 5,002,000, over the limit.
    // ENV: 2501 * 1000 per profile, and (2 + 2 * 2499) * 1000 = 5,000,000 overall, both within it.
    let sequence = Sequence {
        ap: 0.0,
        profiles: 2,
        ranges: 2,
        receivers: 1246,
    };
    let loaded = intake(
        "env-accepts",
        &[("case.env", &sequence.env()), ("case.flp", &sequence.flp())],
    )
    .unwrap();
    assert_eq!(loaded.cases().len(), 1000);
    assert_eq!(loaded.cases()[0].profiles().len(), 2);

    // Export does not reload what it writes, and what it writes for this ENV does not load.
    let exported =
        json::encode_case_document(&loaded.document().unwrap(), Path::new("case.json")).unwrap();
    for source in [exported, sequence.document()] {
        let report = intake("json-rejects", &[("case.json", &source)])
            .unwrap_err()
            .into_report();
        assert_over_cumulative_budget(&report, "frequency");
    }
}

#[test]
fn known_difference_json_accepts_a_case_that_env_counts_over_the_budget() {
    // V = C = 4 water + 2 modes + 1 source + 1 range + 2 * 2496 = 5000. JSON: 5000 * 1000 =
    // 5,000,000, within the limit. ENV also charges a value per profile and frequency, so
    // (1 + 5000) * 1000 = 5,001,000 is over it, though its per-profile check passes.
    let sequence = Sequence {
        ap: 0.1,
        profiles: 1,
        ranges: 1,
        receivers: 2496,
    };
    let report = intake(
        "env-rejects",
        &[("case.env", &sequence.env()), ("case.flp", &sequence.flp())],
    )
    .unwrap_err()
    .into_report();
    assert_over_cumulative_budget(&report, "profiles");
    let loaded = intake("json-accepts", &[("case.json", &sequence.document())]).unwrap();
    assert_eq!(loaded.cases().len(), 1000);
}

#[test]
fn known_difference_only_json_accepts_an_empty_biological_list() {
    let document = edited_document(vec![(
        "/attenuation",
        json!({"volume": {"type": "biological", "layers": []}}),
    )]);
    intake("empty-json", &[("case.json", &document.to_string())]).unwrap();

    // The generic vector-count bound rejects it, before any biological rule is asked.
    let report = intake(
        "empty-env",
        &[
            ("case.env", &edited_env(ENV, &[("'NVW'", "'NVWB'\n0")])),
            ("case.flp", FLP),
        ],
    )
    .unwrap_err()
    .into_report();
    let first = &report.diagnostics()[0];
    assert_eq!(first.code, "KR0201", "{report}");
    assert_eq!(first.field, "bio_layers.count", "{report}");
}

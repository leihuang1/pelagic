use std::path::{Path, PathBuf};

use kraken::{ModeSolver, input, json, legacy};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
        .with_extension("env")
}

fn legacy_input(name: &str, solver: ModeSolver) -> input::FieldInput {
    let env = fixture(name);
    input::load_legacy(&env, &env.with_extension("flp"), solver).unwrap()
}

fn document(name: &str, solver: ModeSolver) -> serde_json::Value {
    serde_json::to_value(legacy_input(name, solver).document().unwrap()).unwrap()
}

fn load(value: &serde_json::Value) -> Result<Vec<kraken::FieldCase>, kraken::DiagnosticReport> {
    json::load_case_document_named(&serde_json::to_vec(value).unwrap(), Path::new("case.json"))
}

#[test]
fn every_accepted_legacy_fixture_builds_the_same_cases_from_its_document() {
    let mut count = 0;
    for entry in std::fs::read_dir(fixture("Pekeris").parent().unwrap()).unwrap() {
        let env = entry.unwrap().path();
        if env.extension().is_none_or(|extension| extension != "env") {
            continue;
        }
        for solver in [ModeSolver::Kraken, ModeSolver::Krakenc] {
            let Ok(legacy) = input::load_legacy(&env, &env.with_extension("flp"), solver) else {
                continue;
            };
            let document = legacy.document().unwrap();
            let bytes = serde_json::to_vec(&document).unwrap();
            let decoded = json::decode_case_document(&bytes, Path::new("case.json")).unwrap();
            assert_eq!(decoded, document, "{} / {solver:?}", env.display());
            let cases = decoded
                .cases(Path::new("case.json"))
                .unwrap_or_else(|report| panic!("{} / {solver:?}: {report}", env.display()));
            assert_eq!(cases, legacy.cases(), "{} / {solver:?}", env.display());
            count += 1;
        }
    }
    eprintln!("{count} legacy collections rebuild exactly");
    assert!(
        count >= 200,
        "expected curated legacy coverage; found {count}"
    );
}

#[test]
fn broadband_documents_hold_one_environment_and_a_frequency_grid() {
    let document = document("WaterLossPower", ModeSolver::Kraken);
    // Each loss recipe is stored once; every frequency is converted on load.
    assert!(document["frequency"].is_array() || document["frequency"].is_object());
    assert!(document.get("profiles").is_none());
    let cases = load(&document).unwrap();
    assert_eq!(
        cases
            .iter()
            .map(|case| case.profiles()[0].frequency_hz)
            .collect::<Vec<_>>(),
        [75.0, 50.0, 62.5, 50.0]
    );
}

#[test]
fn hand_written_case_uses_defaults_grids_and_shorthand() {
    let cases = json::load_case_document(
        br#"{
          "pelagic": 2, "model": "kraken", "title": "Pekeris",
          "frequency": {"from": 50, "to": 150, "count": 3},
          "interpolation": "n2_linear",
          "media": [{"to": 100, "ssp": [[0, 1500], [100, 1500]]}],
          "surface": "vacuum",
          "bottom": {"type": "halfspace", "cp": 1700, "rho": 1.5, "ap": 0.8},
          "source": {"depth": 25},
          "receivers": {"depth": [25, 50.1], "range": {"from": 0.5, "to": 2, "count": 4, "unit": "km"}},
          "kraken": {"solver": "kraken", "phase_speed": [1400, 1700], "max_range": 2000}
        }"#,
    )
    .unwrap();
    assert_eq!(cases.len(), 3);
    let case = &cases[1].profiles()[0];
    assert_eq!(case.frequency_hz, 100.0);
    // dB/wavelength without volume loss is used as written at every frequency.
    assert_eq!(case.bottom_attenuation_db_per_wavelength, 0.8);
    assert_eq!(case.receiver_depths_m, [25.0, f64::from(50.1_f32)]);
    assert_eq!(case.receiver_offsets_m, [0.0, 0.0]);
    assert_eq!(case.receiver_ranges_m, [500.0, 1000.0, 1500.0, 2000.0]);
    assert_eq!(case.mode_sample_depths_m, [25.0, f64::from(50.1_f32)]);
    assert_eq!(case.source_geometry, kraken::SourceGeometry::Point);
}

#[test]
fn strict_json_rejects_malformed_fields_and_locates_semantic_failures() {
    let original = document("TabRefIrcC", ModeSolver::Krakenc);
    for (pointer, replacement, code, field) in [
        ("/pelagic", serde_json::json!(1), "KR0202", "pelagic"),
        ("/model", serde_json::json!("bellhop"), "KR0202", "model"),
        ("/frequency", serde_json::json!([]), "KR0201", "frequency"),
        (
            "/kraken/solver",
            serde_json::json!("unknown"),
            "KR0103",
            "kraken.solver",
        ),
        (
            "/media/0/mesh",
            serde_json::json!(1),
            "KR0201",
            "media[0].mesh",
        ),
        (
            "/media/0/ssp/0",
            serde_json::json!([0.0]),
            "KR0201",
            "media[0].ssp[0]",
        ),
        (
            "/media/0/extra",
            serde_json::json!(1),
            "KR0103",
            "media[0].extra",
        ),
        ("/bottom/extra", serde_json::json!(1), "KR0103", "bottom"),
        (
            "/surface",
            serde_json::json!("halfspace"),
            "KR0103",
            "surface",
        ),
    ] {
        let mut value = original.clone();
        if let Some(slot) = value.pointer_mut(pointer) {
            *slot = replacement;
        } else {
            let (parent, key) = pointer.rsplit_once('/').unwrap();
            value.pointer_mut(parent).unwrap()[key] = replacement;
        }
        let report = load(&value).unwrap_err();
        assert_eq!(report.diagnostics()[0].code, code, "{pointer}: {report}");
        assert_eq!(report.diagnostics()[0].field, field, "{pointer}: {report}");
        assert_eq!(report.diagnostics()[0].path, Path::new("case.json"));
    }
    for source in [
        b"{".as_slice(),
        b"{\"pelagic\":2,\"pelagic\":2}",
        b"{\"extra\":0}",
    ] {
        assert_eq!(
            json::load_case_document(source).unwrap_err().diagnostics()[0].code,
            "KR0103"
        );
    }
    assert_eq!(
        json::load_case_document(&vec![
            b' ';
            usize::try_from(legacy::MAX_INPUT_BYTES).unwrap() + 1
        ])
        .unwrap_err()
        .diagnostics()[0]
            .code,
        "KR0101"
    );
}

#[test]
fn range_dependent_documents_list_only_what_each_profile_changes() {
    let document = document("ProfilesCm", ModeSolver::Kraken);
    let profiles = document["profiles"].as_array().unwrap();
    assert_ne!(profiles.len(), 0);
    assert_eq!(document["kraken"]["propagation"], "coupled");
    let mut value = document.clone();
    value["kraken"]
        .as_object_mut()
        .unwrap()
        .remove("propagation");
    assert_eq!(
        load(&value).unwrap_err().diagnostics()[0].field,
        "kraken.propagation"
    );
    let mut value = document;
    value["kraken"]["addition"] = serde_json::json!("incoherent");
    assert!(load(&value).is_err());
}

#[test]
fn each_profile_keeps_its_own_broadband_mesh_frequency() {
    let directory = std::env::temp_dir().join(format!("kraken-json-mesh-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let env = fixture("PekerisBroadband");
    let profile = std::fs::read_to_string(&env).unwrap();
    // Two broadband profiles solve the same frequencies with 50 and 100 Hz mesh headers.
    let second = profile.replacen("\n50.0\n", "\n100.0\n", 1);
    assert_ne!(second, profile);
    let flp = std::fs::read_to_string(env.with_extension("flp"))
        .unwrap()
        .replacen("'X OC'", "'XAOC'", 1)
        .replacen("1\n0.0 /", "2\n0.0 1.0 /", 1);
    let (two_env, two_flp) = (directory.join("two.env"), directory.join("two.flp"));
    std::fs::write(&two_env, profile + &second).unwrap();
    std::fs::write(&two_flp, flp).unwrap();
    let legacy = input::load_legacy(&two_env, &two_flp, ModeSolver::Kraken).unwrap();
    let meshes: Vec<_> = legacy.cases()[0]
        .profiles()
        .iter()
        .map(|case| case.mesh_reference_frequency_hz)
        .collect();
    assert_eq!(meshes, [Some(50.0), Some(100.0)]);
    let document = serde_json::to_value(legacy.document().unwrap()).unwrap();
    assert_eq!(document["kraken"]["mesh_frequency"], 50.0);
    assert_eq!(document["profiles"][0]["kraken"]["mesh_frequency"], 100.0);
    assert_eq!(load(&document).unwrap(), legacy.cases());
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn profiles_may_mix_broadband_and_single_frequency_headers() {
    let env = fixture("PekerisBroadband");
    let lines: Vec<String> = std::fs::read_to_string(&env)
        .unwrap()
        .lines()
        .map(str::to_owned)
        .collect();
    // An ordinary 50 Hz header, and a broadband 100 Hz header solving [50].
    let mut ordinary = lines[..lines.len() - 2].to_vec();
    ordinary[3] = "'NVW'".into();
    let mut broadband = lines[..lines.len() - 2].to_vec();
    broadband[1] = "100.0".into();
    broadband.extend(["1".into(), "50.0 /".into()]);
    let flp = std::fs::read_to_string(env.with_extension("flp"))
        .unwrap()
        .replacen("'X OC'", "'XAOC'", 1)
        .replacen("1\n0.0 /", "2\n0.0 1.0 /", 1);
    let directory = std::env::temp_dir().join(format!("kraken-json-mixed-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    for (name, first, second, override_) in [
        (
            "ordinary-first",
            &ordinary,
            &broadband,
            serde_json::json!(100.0),
        ),
        (
            "broadband-first",
            &broadband,
            &ordinary,
            serde_json::Value::Null,
        ),
    ] {
        let (path, flp_path) = (
            directory.join(format!("{name}.env")),
            directory.join(format!("{name}.flp")),
        );
        std::fs::write(
            &path,
            [first.join("\n"), second.join("\n")].join("\n") + "\n",
        )
        .unwrap();
        std::fs::write(&flp_path, &flp).unwrap();
        // Loading and solving never depend on the JSON form.
        let legacy = input::load_legacy(&path, &flp_path, ModeSolver::Kraken).unwrap();
        let document = serde_json::to_value(legacy.document().unwrap()).unwrap();
        assert_eq!(
            document["profiles"][0]["kraken"]["mesh_frequency"], override_,
            "{name}"
        );
        assert_eq!(load(&document).unwrap(), legacy.cases(), "{name}");
    }
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn power_law_reference_frequency_must_be_positive() {
    let mut document = serde_json::json!({
      "pelagic": 2, "model": "kraken", "frequency": 50, "interpolation": "n2_linear",
      "attenuation": {"unit": "db_per_meter_power_law", "reference_frequency": 50},
      "media": [{"to": 100, "ssp": [[0, 1500], [100, 1500]], "power_law": [2, 1000]}],
      "surface": "vacuum",
      "bottom": {"type": "halfspace", "cp": 1700, "rho": 1.5, "ap": 0.5, "power_law": [2, 1000]},
      "source": {"depth": 25}, "receivers": {"depth": 50, "range": 1000},
      "kraken": {"solver": "kraken", "phase_speed": [1400, 1700], "max_range": 1000}
    });
    load(&document).unwrap();
    for reference in [-50.0, 0.0] {
        document["attenuation"]["reference_frequency"] = serde_json::json!(reference);
        let report = load(&document).unwrap_err();
        assert_eq!(
            report.diagnostics()[0].field,
            "attenuation.reference_frequency",
            "{reference}: {report}"
        );
    }
}

#[test]
fn encoding_rejects_documents_that_outgrow_the_input_limit() {
    // Integer depths and speeds are read compactly but written back as `1500.0`.
    let rows: Vec<String> = (0..=65_000)
        .map(|depth| format!("[{depth},1500]"))
        .collect();
    let source = format!(
        r#"{{"pelagic":2,"model":"kraken","frequency":50,"interpolation":"c_linear",
        "media":[{{"to":65000,"ssp":[{}]}}],"surface":"vacuum",
        "bottom":{{"type":"halfspace","cp":1700,"rho":1.5}},
        "source":{{"depth":25}},"receivers":{{"depth":50,"range":1000}},
        "kraken":{{"solver":"kraken","phase_speed":[1400,1700],"max_range":1000}}}}"#,
        rows.join(",")
    );
    let limit = usize::try_from(legacy::MAX_INPUT_BYTES).unwrap();
    assert!(source.len() < limit);
    let path = Path::new("case.json");
    let document = json::decode_case_document(source.as_bytes(), path).unwrap();
    let report = json::encode_case_document(&document, path).unwrap_err();
    assert_eq!(report.diagnostics()[0].code, "KR0101");
    // A document that fits is written exactly as serde writes it.
    let small = legacy_input("Pekeris", ModeSolver::Kraken)
        .document()
        .unwrap();
    assert_eq!(
        json::encode_case_document(&small, path).unwrap(),
        serde_json::to_string(&small).unwrap()
    );
}

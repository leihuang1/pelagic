use std::path::{Path, PathBuf};

use kraken::{ModeSolver, json, legacy};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
        .with_extension("env")
}

fn document(name: &str, solver: ModeSolver) -> json::CaseDocument {
    let env = fixture(name);
    let cases = legacy::load_field_cases(&env, env.with_extension("flp"), solver).unwrap();
    json::export_case_document(&cases).unwrap()
}

#[test]
fn accepted_legacy_fixtures_round_trip_without_changing_case_definitions() {
    let mut count = 0;
    for entry in std::fs::read_dir(fixture("Pekeris").parent().unwrap()).unwrap() {
        let env = entry.unwrap().path();
        if env.extension().is_none_or(|extension| extension != "env") {
            continue;
        }
        for solver in [ModeSolver::Kraken, ModeSolver::Krakenc] {
            let Ok(cases) = legacy::load_field_cases(&env, env.with_extension("flp"), solver)
            else {
                continue;
            };
            let document = json::export_case_document(&cases).unwrap();
            let bytes = serde_json::to_vec(&document).unwrap();
            let restored =
                json::load_case_document_named(&bytes, Path::new("relocated.json")).unwrap();
            assert_eq!(restored, cases, "{} / {solver:?}", env.display());
            assert_eq!(json::export_case_document(&restored).unwrap(), document);
            count += 1;
        }
    }
    let example =
        json::load_case_document(include_bytes!("../../../examples/kraken-pekeris.json")).unwrap();
    let env = fixture("Pekeris");
    assert_eq!(
        example,
        legacy::load_field_cases(&env, env.with_extension("flp"), ModeSolver::Kraken).unwrap()
    );
    eprintln!("{count} legacy collections round-trip exactly");
    assert!(
        count >= 200,
        "expected curated legacy coverage; found {count}"
    );
}

#[test]
fn library_export_enforces_canonical_json_bytes_not_vector_or_character_counts() {
    let mut empty = document("Pekeris", ModeSolver::Kraken);
    empty.frequencies[0].profiles[0].title.clear();
    let overhead = serde_json::to_vec(&empty).unwrap().len();
    let limit = usize::try_from(legacy::MAX_INPUT_BYTES).unwrap();
    for title in [
        "a".repeat(limit - overhead),
        "a".repeat(limit - overhead + 1),
        "\"".repeat((limit - overhead) / 2 + 1),
        "é".repeat((limit - overhead) / 2 + 1),
    ] {
        let mut definition = empty.frequencies[0].profiles[0].clone();
        definition.title = title;
        let case = kraken::FieldCase::new(
            vec![kraken::Case::from_definition(definition).unwrap()],
            vec![0.0],
            kraken::FieldPropagation::RangeIndependent,
        )
        .unwrap();
        // All are valid cases with identical small vectors, irrespective of serialized title size.
        let mut expected = empty.clone();
        expected.frequencies[0].profiles[0].title = case.profiles()[0].title.clone();
        let expected_bytes = serde_json::to_vec(&expected).unwrap();
        let cases = [case];
        if expected_bytes.len() <= limit {
            let exported = json::export_case_document(&cases).unwrap();
            assert_eq!(serde_json::to_vec(&exported).unwrap(), expected_bytes);
            assert_eq!(json::load_case_document(&expected_bytes).unwrap(), cases);
        } else {
            let report = json::export_case_document(&cases)
                .map(drop)
                .expect_err("library export must reject oversized canonical JSON");
            assert_eq!(report.diagnostics()[0].code, "KR0101");
            assert_eq!(report.diagnostics()[0].field, "json");
        }
    }
    // A compact vector-rich case also exceeds the byte limit while remaining below entry budgets.
    let mut definition = empty.frequencies[0].profiles[0].clone();
    definition.receiver_ranges_m = (0..100_000).map(|value| f64::from(value) / 3.0).collect();
    let case = kraken::FieldCase::new(
        vec![kraken::Case::from_definition(definition).unwrap()],
        vec![0.0],
        kraken::FieldPropagation::RangeIndependent,
    )
    .unwrap();
    assert_eq!(
        json::export_case_document(&[case])
            .map(drop)
            .unwrap_err()
            .diagnostics()[0]
            .code,
        "KR0101"
    );
}

#[test]
fn strict_json_rejects_malformed_nested_fields_and_shared_semantic_failures() {
    let original = serde_json::to_value(document("TabRefIrcC", ModeSolver::Krakenc)).unwrap();
    for (pointer, replacement, code, field) in [
        (
            "/schema_version",
            serde_json::json!(2),
            "KR0202",
            "schema_version",
        ),
        (
            "/frequencies",
            serde_json::json!([]),
            "KR0201",
            "frequencies",
        ),
        (
            "/frequencies/0/propagation",
            serde_json::json!("unknown"),
            "KR0103",
            "frequencies[0].propagation",
        ),
        (
            "/frequencies/0/profiles/0/mesh_points",
            serde_json::json!(1),
            "KR0201",
            "frequencies[0].profiles[0].mesh_points",
        ),
        (
            "/frequencies/0/profiles/0/bottom_boundary/data/points/0/f",
            serde_json::json!({"real": 1.0, "imaginary": 0.0, "extra": 1}),
            "KR0103",
            "frequencies[0].profiles[0].bottom_boundary",
        ),
        (
            "/frequencies/0/profiles/0/sound_speed_profile/0",
            serde_json::json!({"depth_m": 0.0, "sound_speed_mps": 1500.0, "extra": 1}),
            "KR0103",
            "frequencies[0].profiles[0].sound_speed_profile[0].extra",
        ),
        (
            "/frequencies/0/profiles/0/surface_boundary",
            serde_json::json!({"type": "vacuum", "extra": 1}),
            "KR0103",
            "frequencies[0].profiles[0].surface_boundary",
        ),
    ] {
        let mut value = original.clone();
        *value.pointer_mut(pointer).unwrap() = replacement;
        let report = json::load_case_document_named(
            &serde_json::to_vec(&value).unwrap(),
            Path::new("bad.json"),
        )
        .unwrap_err();
        assert_eq!(report.diagnostics()[0].code, code, "{pointer}");
        assert_eq!(report.diagnostics()[0].field, field, "{pointer}");
        assert_eq!(report.diagnostics()[0].path, Path::new("bad.json"));
    }
    for source in [
        b"{\"schema_version\":1,\"schema_version\":1}".as_slice(),
        b"{",
        b"{\"schema_version\":1,\"extra\":0}",
        b"{\"schema_version\":1,\"frequencies\":[],\"extra\":0}",
    ] {
        assert_eq!(
            json::load_case_document(source).unwrap_err().diagnostics()[0].code,
            "KR0103"
        );
    }
    let mut value = original;
    value["frequencies"][0]["profiles"][0]
        .as_object_mut()
        .unwrap()
        .remove("title");
    assert_eq!(
        json::load_case_document(&serde_json::to_vec(&value).unwrap())
            .unwrap_err()
            .diagnostics()[0]
            .code,
        "KR0103"
    );
}

#[test]
fn json_preserves_frequency_order_and_rejects_invalid_collections_and_coupling() {
    let mut document = document("WaterLossPower", ModeSolver::Kraken);
    document.frequencies.push(document.frequencies[1].clone());
    let cases = json::load_case_document(&serde_json::to_vec(&document).unwrap()).unwrap();
    assert_eq!(
        cases
            .iter()
            .map(|case| case.profiles()[0].frequency_hz)
            .collect::<Vec<_>>(),
        [75.0, 50.0, 62.5, 50.0, 50.0]
    );
    document.frequencies[4].profiles[0].mode_solver = ModeSolver::Krakenc;
    let report = json::load_case_document(&serde_json::to_vec(&document).unwrap()).unwrap_err();
    assert_eq!(
        report.diagnostics()[0].field,
        "frequencies[4].profiles[0].mode_solver"
    );
    let mut document = self::document("ProfilesCm", ModeSolver::Kraken);
    document.frequencies[0].profiles[3].mode_addition = kraken::ModeAddition::Incoherent;
    assert!(json::load_case_document(&serde_json::to_vec(&document).unwrap()).is_err());
    assert!(json::export_case_document(&[]).is_err());
    assert!(json::export_case_document(&vec![cases[0].clone(); 1001]).is_err());
    let mut input = cases[0].profiles()[0].clone().into_definition();
    input.receiver_ranges_m = (0..10_000).map(f64::from).collect();
    let large = kraken::FieldCase::new(
        vec![kraken::Case::from_definition(input).unwrap()],
        vec![0.0],
        kraken::FieldPropagation::RangeIndependent,
    )
    .unwrap();
    let report = json::export_case_document(&vec![large; 501]).unwrap_err();
    assert!(
        report.diagnostics()[0]
            .message
            .contains("cumulative input storage")
    );
    let mut document = self::document("Pekeris", ModeSolver::Kraken);
    document.frequencies = vec![
        json::FieldDocument {
            propagation: kraken::FieldPropagation::RangeIndependent,
            profile_ranges_m: vec![0.0],
            profiles: vec![]
        };
        1001
    ];
    let report = json::load_case_document(&serde_json::to_vec(&document).unwrap()).unwrap_err();
    assert_eq!(report.diagnostics()[0].field, "frequencies");
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

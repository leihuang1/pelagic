use std::fs;
use std::io::Read;
use std::path::Path;

use kraken::{ModeSolver, input, json, legacy};

#[test]
fn single_environment_discovery_keeps_first_profile_scope_and_input_bounds() {
    let path = Path::new("discovery.env");
    let first = include_str!("fixtures/PekerisComplexBlank.env");
    let later = include_str!("fixtures/TabRefBrcC.env");
    let source = format!("{first}\n{later}");
    assert_eq!(
        legacy::bottom_table_extension(&source, path, ModeSolver::Krakenc).unwrap(),
        None
    );
    assert_eq!(
        legacy::surface_table_extension(&source, path, ModeSolver::Krakenc).unwrap(),
        None
    );
    assert_eq!(
        legacy::field_table_extensions(&source, path, ModeSolver::Krakenc).unwrap(),
        [None, Some("brc"), None]
    );
    // Discovery of a single environment must not validate trailing profiles.
    let source = format!("{first}\nnot a valid profile\n");
    assert_eq!(
        legacy::bottom_table_extension(&source, path, ModeSolver::Krakenc).unwrap(),
        None
    );
    assert_eq!(
        legacy::surface_table_extension(&source, path, ModeSolver::Krakenc).unwrap(),
        None
    );
    assert!(legacy::field_table_extensions(&source, path, ModeSolver::Krakenc).is_err());
    let oversized = " ".repeat(1_048_577);
    for report in [
        legacy::bottom_table_extension(&oversized, path, ModeSolver::Krakenc).unwrap_err(),
        legacy::surface_table_extension(&oversized, path, ModeSolver::Krakenc).unwrap_err(),
    ] {
        let d = &report.diagnostics()[0];
        assert_eq!(
            (d.code, d.field.as_str(), d.path.as_path(), d.line, d.column),
            ("KR0201", "input", path, 1, 1)
        );
    }
}

#[test]
fn single_environment_loading_keeps_resource_and_parse_error_order() {
    let root = std::env::temp_dir().join(format!("kraken-discovery-{}", std::process::id()));
    fs::create_dir(&root).unwrap();
    let env = root.join("environment.env");
    let flp = root.join("geometry.flp");
    // Both resources are discovered before this unsupported combination is validated.
    let source = include_str!("fixtures/TabRefBrcC.env").replace("'CVW'", "'CFW'");
    fs::write(&env, format!("{source}\ntrailing input\n")).unwrap();
    fs::write(&flp, include_str!("fixtures/FieldPattern.flp")).unwrap();
    // Adjacent FLP tables and ENV patterns must not satisfy these resource roles.
    fs::write(flp.with_extension("brc"), "unused").unwrap();
    fs::write(flp.with_extension("trc"), "unused").unwrap();
    fs::write(env.with_extension("sbp"), "unused").unwrap();
    for missing in [
        env.with_extension("brc"),
        env.with_extension("trc"),
        flp.with_extension("sbp"),
    ] {
        for report in [
            legacy::load_complex_case(&env, &flp).unwrap_err(),
            legacy::load_frequency_cases(&env, &flp, ModeSolver::Krakenc).unwrap_err(),
        ] {
            let d = &report.diagnostics()[0];
            assert_eq!(
                (d.code, d.field.as_str(), &d.path),
                ("KR0001", "input", &missing)
            );
        }
        // Only read here; malformed resource content is not parsed until assembly.
        fs::write(missing, "invalid table\n").unwrap();
    }
    let report = legacy::load_complex_case(&env, &flp).unwrap_err();
    let d = &report.diagnostics()[0];
    assert_eq!(
        (d.code, d.field.as_str(), &d.path),
        ("KR0102", "input", &env)
    );
    assert_eq!(d.line, source.lines().count() + 2);
    assert!(d.message.contains("unexpected trailing input"));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn retained_snapshots_match_parsing_and_ignore_later_file_changes() {
    let root = std::env::temp_dir().join(format!("kraken-snapshots-{}", std::process::id()));
    fs::create_dir(&root).unwrap();
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let env = root.join("profiles.env");
    let flp = root.join("geometry.flp");
    let mut source = String::new();
    for name in [
        "PekerisComplexBlank",
        "TabRefBrcC",
        "TabRefIrcC",
        "FluidTrcC",
    ] {
        source.push_str(&fs::read_to_string(fixtures.join(name).with_extension("env")).unwrap());
    }
    // Deliberate CRLF snapshot: preserve transport bytes, not just equivalent definitions.
    fs::write(&env, source.replace('\n', "\r\n")).unwrap();
    let field = fs::read_to_string(fixtures.join("TabRefBrcC.flp")).unwrap();
    fs::write(
        &flp,
        field
            .replace("'R OC'", "'RA*C'")
            .replace("1\n0.0 /", "4\n0.0 0.4 0.8 1.2 /"),
    )
    .unwrap();
    for (name, ext) in [
        ("TabRefBrcC", "brc"),
        ("TabRefIrcC", "irc"),
        ("FluidTrcC", "trc"),
    ] {
        fs::copy(
            fixtures.join(name).with_extension(ext),
            env.with_extension(ext),
        )
        .unwrap();
    }
    fs::copy(fixtures.join("FieldPattern.sbp"), flp.with_extension("sbp")).unwrap();
    // Neither ENV's SBP nor FLP's table stem is consumed.
    fs::write(env.with_extension("sbp"), b"poisoned unused source pattern").unwrap();
    fs::write(flp.with_extension("trc"), b"poisoned unused table").unwrap();
    let loaded = input::load_legacy(&env, &flp, ModeSolver::Krakenc).unwrap();
    assert_eq!(
        loaded
            .snapshots()
            .iter()
            .map(input::InputSnapshot::role)
            .collect::<Vec<_>>(),
        ["env", "flp", "trc", "brc", "irc", "sbp"]
    );
    assert_eq!(
        legacy::load_field_cases(&env, &flp, ModeSolver::Krakenc).unwrap(),
        loaded.cases()
    );
    for snapshot in loaded.snapshots() {
        assert_eq!(
            snapshot.source().as_bytes(),
            fs::read(snapshot.path()).unwrap()
        );
        fs::write(snapshot.path(), b"changed after loading").unwrap();
    }
    assert!(loaded.snapshots()[0].source().contains("\r\n"));
    let expected = legacy::load_field_cases_with_resources(
        loaded.snapshots()[0].source(),
        loaded.snapshots()[1].source(),
        &env,
        &flp,
        ModeSolver::Krakenc,
        [2, 3, 4].map(|index| Some(loaded.snapshots()[index].source())),
        Some(loaded.snapshots()[5].source()),
    )
    .unwrap();
    assert_eq!(expected, loaded.cases());
    fs::remove_dir_all(root).unwrap();
}

/// A directory holding a legacy pair and the JSON document it converts to.
struct IntakeFiles {
    root: std::path::PathBuf,
    env: std::path::PathBuf,
    flp: std::path::PathBuf,
    json: std::path::PathBuf,
    text: String,
}

impl IntakeFiles {
    fn new(name: &str) -> Self {
        let root =
            std::env::temp_dir().join(format!("kraken-intake-{name}-{}", std::process::id()));
        fs::create_dir(&root).unwrap();
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/PekerisBroadband");
        let (env, flp, json) = (
            root.join("case.env"),
            root.join("case.flp"),
            root.join("case.json"),
        );
        fs::copy(fixture.with_extension("env"), &env).unwrap();
        fs::copy(fixture.with_extension("flp"), &flp).unwrap();
        let document = input::load_legacy(&env, &flp, ModeSolver::Kraken)
            .unwrap()
            .document()
            .unwrap();
        let text = serde_json::to_string_pretty(&document).unwrap();
        fs::write(&json, &text).unwrap();
        Self {
            root,
            env,
            flp,
            json,
            text,
        }
    }
}

impl Drop for IntakeFiles {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

/// Fails the test if the intake reads it.
struct Unread;

impl Read for Unread {
    fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
        panic!("the source was read")
    }
}

fn only(report: &kraken::DiagnosticReport) -> (&'static str, &str, &str, &Path, usize, usize) {
    let [d] = report.diagnostics() else {
        panic!("{report}");
    };
    (
        d.code,
        d.field.as_str(),
        d.message.as_str(),
        d.path.as_path(),
        d.line,
        d.column,
    )
}

#[test]
fn intake_keeps_each_source_kinds_snapshots_and_consumed_inputs() {
    let files = IntakeFiles::new("sources");

    let legacy = input::load(&files.env, None, None).unwrap();
    assert_eq!(legacy.path(), files.env);
    assert_eq!(
        legacy.consumed_inputs(),
        [files.env.clone(), files.flp.clone()]
    );
    assert_eq!(
        legacy
            .snapshots()
            .iter()
            .map(input::InputSnapshot::role)
            .collect::<Vec<_>>(),
        ["env", "flp"]
    );

    let file = input::load(&files.json, None, None).unwrap();
    assert_eq!(file.path(), files.json);
    assert_eq!(file.consumed_inputs(), std::slice::from_ref(&files.json));
    let [snapshot] = file.snapshots() else {
        panic!("one JSON snapshot");
    };
    assert_eq!(
        (snapshot.role(), snapshot.path(), snapshot.source()),
        ("json", files.json.as_path(), files.text.as_str())
    );

    let reader = input::load_reader(files.text.as_bytes(), None, None).unwrap();
    assert_eq!(reader.path(), Path::new("stdin.json"));
    assert_eq!(reader.consumed_inputs(), [] as [std::path::PathBuf; 0]);
    let [snapshot] = reader.snapshots() else {
        panic!("one JSON snapshot");
    };
    assert_eq!(
        (snapshot.role(), snapshot.path(), snapshot.source()),
        ("json", Path::new("stdin.json"), files.text.as_str())
    );

    assert_eq!(legacy.cases(), file.cases());
    assert_eq!(file.cases(), reader.cases());
    let decoded = json::decode_case_document(files.text.as_bytes(), &files.json).unwrap();
    assert_eq!(legacy.document().unwrap(), decoded);
    assert_eq!(file.document().unwrap(), decoded);
    assert_eq!(reader.document().unwrap(), decoded);
}

#[test]
fn intake_defaults_the_legacy_flp_and_engine() {
    let files = IntakeFiles::new("defaults");
    let other = files.root.join("other.flp");
    fs::rename(&files.flp, &other).unwrap();
    let report = input::load(&files.env, None, None)
        .unwrap_err()
        .into_report();
    let (code, field, _, path, ..) = only(&report);
    assert_eq!(
        (code, field, path),
        ("KR0001", "input", files.flp.as_path())
    );

    let chosen = input::load(&files.env, Some(&other), None).unwrap();
    assert_eq!(chosen.consumed_inputs(), [files.env.clone(), other.clone()]);
    let default = input::load_legacy(&files.env, &other, ModeSolver::Kraken).unwrap();
    assert_eq!(chosen, default);
    let complex = input::load(&files.env, Some(&other), Some(ModeSolver::Krakenc)).unwrap();
    assert_eq!(
        complex,
        input::load_legacy(&files.env, &other, ModeSolver::Krakenc).unwrap()
    );
    assert_ne!(chosen.cases(), complex.cases());
}

#[test]
fn only_a_json_extension_selects_json() {
    let files = IntakeFiles::new("routing");
    let flp = files.root.join("other.flp");
    fs::copy(&files.flp, &flp).unwrap();
    for name in ["other.JSON", "other"] {
        let path = files.root.join(name);
        fs::copy(&files.env, &path).unwrap();
        let loaded = input::load(&path, None, None).unwrap();
        assert_eq!(loaded.snapshots()[0].role(), "env");
        assert_eq!(loaded.consumed_inputs(), [path, flp.clone()]);
    }
}

#[test]
fn intake_rejects_an_flp_for_json_before_reading() {
    let flp = Path::new("unread.flp");
    let missing = Path::new("no-such-directory/case.json");
    for error in [
        input::load(missing, Some(flp), None).unwrap_err(),
        input::load_reader(Unread, Some(flp), None).unwrap_err(),
    ] {
        assert_eq!(error.snapshot(), None);
        assert_eq!(
            only(&error.into_report()),
            (
                "KR0202",
                "flp",
                "--flp is invalid for self-contained JSON",
                Path::new("<case>"),
                1,
                1
            )
        );
    }
}

#[test]
fn intake_asserts_the_solver_for_json_after_validating_it() {
    let files = IntakeFiles::new("solver");
    let asserted = Some(ModeSolver::Krakenc);
    let invalid = files
        .text
        .replace("\"frequency\": [", "\"frequency\": [-1,");
    assert_ne!(invalid, files.text);
    let cases = [
        (
            input::load(&files.json, None, asserted).unwrap_err(),
            files.json.as_path(),
        ),
        (
            input::load_reader(files.text.as_bytes(), None, asserted).unwrap_err(),
            Path::new("stdin.json"),
        ),
    ];
    for (error, path) in cases {
        assert_eq!(error.snapshot().unwrap().source(), files.text);
        assert_eq!(
            only(&error.into_report()),
            (
                "KR0202",
                "kraken.solver",
                "--solver must match the JSON document",
                path,
                1,
                1
            )
        );
    }
    let report = input::load_reader(invalid.as_bytes(), None, asserted)
        .unwrap_err()
        .into_report();
    assert_eq!(report.diagnostics()[0].field, "frequency");
    let matching = input::load(&files.json, None, Some(ModeSolver::Kraken)).unwrap();
    assert_eq!(matching, input::load(&files.json, None, None).unwrap());
}

#[test]
fn json_failures_carry_what_was_read_and_legacy_failures_carry_nothing() {
    let files = IntakeFiles::new("failures");
    let oversized = " ".repeat(usize::try_from(legacy::MAX_INPUT_BYTES).unwrap() + 10);
    let read = oversized[..=usize::try_from(legacy::MAX_INPUT_BYTES).unwrap()].to_owned();
    for (source, code, kept) in [
        ("{ malformed", "KR0103", "{ malformed"),
        (r#"{"pelagic": 1}"#, "KR0202", r#"{"pelagic": 1}"#),
        (oversized.as_str(), "KR0101", read.as_str()),
    ] {
        fs::write(&files.json, source).unwrap();
        for (error, path) in [
            (
                input::load(&files.json, None, None).unwrap_err(),
                files.json.as_path(),
            ),
            (
                input::load_reader(source.as_bytes(), None, None).unwrap_err(),
                Path::new("stdin.json"),
            ),
        ] {
            let snapshot = error.snapshot().unwrap();
            assert_eq!(
                (snapshot.role(), snapshot.path(), snapshot.source()),
                ("json", path, kept)
            );
            assert_eq!(error.into_report().diagnostics()[0].code, code);
        }
    }

    let error = input::load_reader(&b"{"[..], None, None).unwrap_err();
    assert_eq!(error.to_string(), error.clone().into_report().to_string());

    // Nothing was read to fingerprint.
    fs::remove_file(&files.json).unwrap();
    let error = input::load(&files.json, None, None).unwrap_err();
    assert_eq!(error.snapshot(), None);
    let report = error.into_report();
    let (code, field, message, path, line, column) = only(&report);
    assert_eq!(
        (code, field, path, line, column),
        ("KR0001", "input", files.json.as_path(), 1, 1)
    );
    assert!(message.starts_with("unable to read input: "), "{message}");
    let error = input::load_reader(&[0xff_u8, 0xfe][..], None, None).unwrap_err();
    assert_eq!(error.snapshot(), None);
    let report = error.into_report();
    let (code, field, message, path, line, column) = only(&report);
    assert_eq!(
        (code, field, path, line, column),
        ("KR0001", "input", Path::new("stdin.json"), 1, 1)
    );
    assert!(message.starts_with("unable to read input: "), "{message}");

    // Read and invalid, but legacy failures have no fingerprint.
    fs::write(&files.env, "not an environment\n").unwrap();
    let error = input::load(&files.env, None, None).unwrap_err();
    assert_eq!(error.snapshot(), None);
    assert_ne!(error.into_report().diagnostics()[0].code, "KR0001");
}

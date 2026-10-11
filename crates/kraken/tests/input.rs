use std::fs;
use std::path::Path;

use kraken::{ModeSolver, input, legacy};

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

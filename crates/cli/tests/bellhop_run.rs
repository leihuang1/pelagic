use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn directory(name: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!("bellhop-run-{name}-{}", std::process::id()));
    fs::create_dir(&path).unwrap();
    path
}

fn run(input: &Path, output: &Path, format: &str) -> Output {
    Command::new(env!("CARGO_BIN_EXE_pelagic"))
        .args(["bellhop", "run", "--format", format])
        .arg(input)
        .arg("--output")
        .arg(output)
        .output()
        .unwrap()
}

#[test]
fn run_protects_primary_and_consumed_auxiliary_inputs_including_symlinks() {
    let root = directory("inputs");
    let json = root.join("case.json");
    fs::write(
        &json,
        include_bytes!("../../../examples/cases/bellhop/field-g/field-g.json"),
    )
    .unwrap();
    let mut pairs = vec![(json.clone(), vec![json])];
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("../bellhop/tests/fixtures/golden");
    for (name, extensions) in [
        ("ShadedField", vec!["env", "sbp"]),
        ("Quadrilateral_one_ray", vec!["env", "ssp", "bty"]),
        ("ParaBotCritical", vec!["env", "ati", "bty"]),
        ("InternalReflection", vec!["env", "irc"]),
    ] {
        let env = root.join(name).with_extension("env");
        let paths = extensions
            .into_iter()
            .map(|extension| {
                let path = env.with_extension(extension);
                fs::copy(fixtures.join(name).with_extension(extension), &path).unwrap();
                path
            })
            .collect();
        pairs.push((env, paths));
    }
    let reflection = root.join("ReflectionTables.env");
    let source = fs::read_to_string(fixtures.join("InternalReflection.env"))
        .unwrap()
        .replace("'CVW'", "'CFW'")
        .replace("'P' 0.0", "'F' 0.0");
    fs::write(&reflection, source).unwrap();
    let mut paths = vec![reflection.clone()];
    for extension in ["trc", "brc"] {
        let path = reflection.with_extension(extension);
        fs::write(&path, b"2\n0.0 1.0 0.0\n90.0 1.0 0.0\n").unwrap();
        paths.push(path);
    }
    let unused = reflection.with_extension("irc");
    fs::write(&unused, b"poisoned unused table").unwrap();
    let process = run(&reflection, &root.join("unused-resource-run"), "both");
    assert!(
        process.status.success(),
        "{}",
        String::from_utf8_lossy(&process.stderr)
    );
    assert_eq!(fs::read(&unused).unwrap(), b"poisoned unused table");
    pairs.push((reflection, paths));
    for (input, paths) in pairs {
        for path in paths {
            let before = fs::read(&path).unwrap();
            for format in ["legacy", "hdf5", "both"] {
                let process = run(&input, &path, format);
                assert_eq!(
                    process.status.code(),
                    Some(4),
                    "{}",
                    String::from_utf8_lossy(&process.stderr)
                );
                assert!(
                    String::from_utf8_lossy(&process.stderr).contains("must not replace an input")
                );
                assert_eq!(fs::read(&path).unwrap(), before);
                #[cfg(unix)]
                {
                    let alias = root.join("alias");
                    std::os::unix::fs::symlink(&path, &alias).unwrap();
                    assert_eq!(run(&input, &alias, format).status.code(), Some(4));
                    assert_eq!(fs::read(&path).unwrap(), before);
                    fs::remove_file(alias).unwrap();
                }
            }
        }
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn run_updates_only_owned_artifacts_and_preserves_unowned_scratch() {
    let root = directory("publication");
    let input = root.join("case.json");
    fs::write(
        &input,
        include_bytes!("../../../examples/cases/bellhop/field-g/field-g.json"),
    )
    .unwrap();
    let output = root.join("results");
    fs::create_dir(&output).unwrap();
    fs::write(output.join("case.h5"), b"unowned result").unwrap();
    assert_eq!(run(&input, &output, "both").status.code(), Some(4));
    assert_eq!(fs::read(output.join("case.h5")).unwrap(), b"unowned result");
    fs::remove_file(output.join("case.h5")).unwrap();
    let scratch = output.join(".pelagic-stage");
    fs::write(&scratch, b"unowned scratch").unwrap();
    assert_eq!(run(&input, &output, "both").status.code(), Some(4));
    assert_eq!(fs::read(&scratch).unwrap(), b"unowned scratch");
    fs::remove_file(scratch).unwrap();
    let process = run(&input, &output, "both");
    assert!(
        process.status.success(),
        "{}",
        String::from_utf8_lossy(&process.stderr)
    );
    assert!(
        fs::read(output.join("case.h5"))
            .unwrap()
            .starts_with(b"\x89HDF\r\n\x1a\n")
    );
    fs::write(output.join("notes.txt"), b"unrelated notes").unwrap();
    assert!(run(&input, &output, "legacy").status.success());
    assert!(!output.join("case.h5").exists());
    assert!(output.join("case.shd").is_file());
    assert_eq!(
        fs::read(output.join("notes.txt")).unwrap(),
        b"unrelated notes"
    );
    let manifest = fs::read(output.join("pelagic-manifest.json")).unwrap();
    fs::write(output.join("case.shd"), b"modified by someone else").unwrap();
    assert_eq!(run(&input, &output, "hdf5").status.code(), Some(4));
    assert_eq!(
        fs::read(output.join("case.shd")).unwrap(),
        b"modified by someone else"
    );
    assert_eq!(
        fs::read(output.join("pelagic-manifest.json")).unwrap(),
        manifest
    );
    assert!(!output.join(".pelagic-stage").exists());
    fs::remove_dir_all(root).unwrap();
}

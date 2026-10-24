use hdf5::types::VarLenUnicode;
use hdf5::{File, Group};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
#[path = "support/publication.rs"]
mod publication;
use publication::{h5, old_output, read_output};
#[path = "support/native.rs"]
#[allow(dead_code)]
mod native;

fn directory(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("kraken-json-{name}-{}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    root
}
fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../kraken/tests/fixtures")
        .join(name)
        .with_extension("env")
}
fn invoke(command: &str, input: &Path, extra: &[&str]) -> Output {
    let mut process = Command::new(env!("CARGO_BIN_EXE_pelagic"));
    process.args(["kraken", command]).arg(input);
    if command == "run" {
        process.args(["--format", "both"]);
    }
    process.args(extra).output().unwrap()
}
fn text(location: &hdf5::Location, name: &str) -> String {
    location
        .attr(name)
        .unwrap()
        .read_scalar::<VarLenUnicode>()
        .unwrap()
        .as_str()
        .to_owned()
}
fn assert_datasets(left: &Group, right: &Group) {
    assert_eq!(left.attr_names().unwrap(), right.attr_names().unwrap());
    for name in left.attr_names().unwrap() {
        let left = left.attr(&name).unwrap();
        let right = right.attr(&name).unwrap();
        assert_eq!(left.dtype().unwrap(), right.dtype().unwrap(), "{name}");
        if left.dtype().unwrap().is::<VarLenUnicode>() {
            assert_eq!(
                left.read_scalar::<VarLenUnicode>().unwrap(),
                right.read_scalar::<VarLenUnicode>().unwrap(),
                "{name}"
            );
        } else {
            assert_eq!(
                left.read_scalar::<f64>().unwrap().to_bits(),
                right.read_scalar::<f64>().unwrap().to_bits(),
                "{name}"
            );
        }
    }
    assert_eq!(left.member_names().unwrap(), right.member_names().unwrap());
    for name in left.member_names().unwrap() {
        if let Ok(group) = left.group(&name) {
            assert_datasets(&group, &right.group(&name).unwrap());
        } else {
            let left = left.dataset(&name).unwrap();
            let right = right.dataset(&name).unwrap();
            assert_eq!(left.shape(), right.shape(), "{name}");
            assert_eq!(left.dtype().unwrap(), right.dtype().unwrap(), "{name}");
            assert_eq!(
                left.read_raw::<f64>().unwrap(),
                right.read_raw::<f64>().unwrap(),
                "{name}"
            );
            assert_eq!(text(&left, "unit"), text(&right, "unit"), "{name}");
        }
    }
}

#[test]
#[allow(clippy::too_many_lines)]
fn json_cli_exports_and_runs_relocated_documents_without_auxiliary_files() {
    let root = directory("round-trip");
    for (name, solver) in [
        ("Pekeris", "kraken"),
        ("PekerisDenseLoss", "krakenc"),
        ("PekerisRigidLoss", "krakenc"),
        ("WaterLossPower", "krakenc"),
        ("OriginalLayeredDouble", "kraken"),
        ("OriginalLayeredDouble", "krakenc"),
        ("LayeredFluidThreeWideRefined", "krakenc"),
        ("FiniteSingleIceC", "kraken"),
        ("GradedElasticBottomN", "kraken"),
        ("GradedElasticTopS", "krakenc"),
        ("GradedElasticPower", "krakenc"),
        ("GradedElasticBio", "krakenc"),
        ("ElasticHalfTopP", "krakenc"),
        ("ElasticHalfTopN", "kraken"),
        ("ElasticHalfBothS", "kraken"),
        ("ElasticHalfTopBroadband", "kraken"),
        ("FluidTrcC", "krakenc"),
        ("TabRefBrcC", "krakenc"),
        ("TabRefIrcC", "krakenc"),
        ("FieldPattern", "krakenc"),
        ("ProfilesAd", "kraken"),
        ("ProfilesCm", "kraken"),
        ("ProfilesAd", "krakenc"),
        ("ProfilesCm", "krakenc"),
    ] {
        let env = fixture(name);
        let stem = format!("{name}-{solver}");
        let document = root.join(&stem).with_extension("json");
        let export = invoke("export", &env, &["--solver", solver]);
        assert!(
            export.status.success(),
            "{}",
            String::from_utf8_lossy(&export.stderr)
        );
        let mut source = export.stdout;
        fs::write(&document, &source).unwrap();
        assert_eq!(invoke("export", &document, &[]).stdout, source);
        source.splice(0..0, b" \r\n".iter().copied());
        source.extend_from_slice(b"\r\n \t");
        fs::write(&document, &source).unwrap();
        for extension in ["env", "flp", "trc", "brc", "irc", "sbp"] {
            fs::write(
                document.with_extension(extension),
                b"invalid auxiliary input",
            )
            .unwrap();
        }
        let legacy_output = root.join(format!("{stem}-legacy"));
        let json_output = root.join(format!("{stem}-json"));
        for (input, output, flags) in [
            (&env, &legacy_output, vec!["--solver", solver]),
            (&document, &json_output, vec![]),
        ] {
            let mut flags = flags;
            flags.extend(["--output", output.to_str().unwrap()]);
            let process = invoke("run", input, &flags);
            if process.status.success() {
                native::kraken(output, input);
            }
            assert!(
                process.status.success(),
                "{}",
                String::from_utf8_lossy(&process.stderr)
            );
        }
        let left = File::open(h5(&legacy_output, &env)).unwrap();
        let right = File::open(h5(&json_output, &document)).unwrap();
        assert_eq!(text(&right, "solver"), solver);
        assert_eq!(
            right
                .attr("schema_version")
                .unwrap()
                .read_scalar::<u32>()
                .unwrap(),
            1
        );
        assert_eq!(
            left.dataset("frequency_hz")
                .unwrap()
                .read_raw::<f64>()
                .unwrap(),
            right
                .dataset("frequency_hz")
                .unwrap()
                .read_raw::<f64>()
                .unwrap()
        );
        assert_datasets(
            &left.group("frequencies").unwrap(),
            &right.group("frequencies").unwrap(),
        );
        let inputs = right.group("inputs").unwrap();
        assert_eq!(inputs.member_names().unwrap(), ["json"]);
        let provenance = inputs.group("json").unwrap();
        assert_eq!(text(&provenance, "filename"), document.to_str().unwrap());
        assert_eq!(
            text(&provenance, "sha256"),
            format!("{:x}", Sha256::digest(&source))
        );
        assert_eq!(
            provenance
                .attr("size_bytes")
                .unwrap()
                .read_scalar::<u64>()
                .unwrap(),
            source.len() as u64
        );
        assert!(!json_output.join(".pelagic-stage").exists());
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
#[allow(clippy::too_many_lines)]
fn json_failures_preserve_outputs_and_protect_input_aliases() {
    let root = directory("failures");
    let document = root.join("case.json");
    let source = invoke("export", &fixture("PekerisBroadband"), &[]).stdout;
    fs::write(&document, &source).unwrap();
    let output = root.join("old");
    old_output(&output, b"old result");
    let conflict = h5(&output, &document);
    fs::write(&conflict, b"old result").unwrap();
    for (flags, code) in [
        (vec![], 4),
        (vec!["--max-output-bytes", "1"], 4),
        (vec!["--solver", "krakenc"], 2),
        (vec!["--flp", "missing.flp"], 2),
    ] {
        let mut flags = flags;
        flags.extend(["--output", output.to_str().unwrap()]);
        let process = invoke("run", &document, &flags);
        assert_eq!(
            process.status.code(),
            Some(code),
            "{}",
            String::from_utf8_lossy(&process.stderr)
        );
        assert_eq!(read_output(&output), b"old result");
        assert!(!output.join(".pelagic-stage").exists());
    }
    fs::remove_file(conflict).unwrap();
    assert_eq!(
        invoke("run", &document, &["--output", document.to_str().unwrap()])
            .status
            .code(),
        Some(4)
    );
    assert_eq!(fs::read(&document).unwrap(), source);
    let mut value: serde_json::Value = serde_json::from_slice(&source).unwrap();
    // 5 Hz has no modes in the window: the second frequency fails after the first solves.
    value["frequency"] = serde_json::json!([75.0, 5.0, 62.5]);
    fs::write(&document, serde_json::to_vec(&value).unwrap()).unwrap();
    let process = invoke("run", &document, &["--output", output.to_str().unwrap()]);
    assert_eq!(
        process.status.code(),
        Some(3),
        "{}",
        String::from_utf8_lossy(&process.stderr)
    );
    assert!(String::from_utf8_lossy(&process.stderr).contains("frequency[1]"));
    assert_eq!(read_output(&output), b"old result");
    assert!(!output.join(".pelagic-stage").exists());
    for bad in [b"{ malformed JSON".to_vec(), vec![b' '; 1_048_577]] {
        fs::write(&document, bad).unwrap();
        assert_eq!(
            invoke("run", &document, &["--output", output.to_str().unwrap()])
                .status
                .code(),
            Some(2)
        );
        let export = invoke("export", &document, &[]);
        assert_eq!(export.status.code(), Some(2));
        assert_eq!(export.stdout, [] as [u8; 0]);
        assert_eq!(read_output(&output), b"old result");
    }
    fs::write(output.join(".pelagic-stage"), b"unowned scratch").unwrap();
    fs::write(&document, &source).unwrap();
    assert_eq!(
        invoke("run", &document, &["--output", output.to_str().unwrap()])
            .status
            .code(),
        Some(4)
    );
    assert_eq!(
        fs::read(output.join(".pelagic-stage")).unwrap(),
        b"unowned scratch"
    );
    assert_eq!(read_output(&output), b"old result");
    #[cfg(unix)]
    {
        let alias = root.join("input-alias");
        std::os::unix::fs::symlink(&document, &alias).unwrap();
        assert_eq!(
            invoke("run", &document, &["--output", alias.to_str().unwrap()])
                .status
                .code(),
            Some(4)
        );
        assert_eq!(fs::read(&document).unwrap(), source);
    }
    let env = root.join("expanded.env");
    fs::copy(fixture("Pekeris"), &env).unwrap();
    let flp = fs::read_to_string(fixture("Pekeris").with_extension("flp")).unwrap();
    let expanded = flp.replace("3\n0.5 1.0 2.0 /", "100000\n0.0 1000.0 /");
    assert_ne!(expanded, flp);
    fs::write(env.with_extension("flp"), expanded).unwrap();
    // A subtabulated vector exports as its grid, not 100000 values.
    let export = invoke("export", &env, &[]);
    assert!(
        export.status.success(),
        "{}",
        String::from_utf8_lossy(&export.stderr)
    );
    assert!(export.stdout.len() < 4096);
    let document: serde_json::Value = serde_json::from_slice(&export.stdout).unwrap();
    assert_eq!(document["receivers"]["range"]["count"], 100_000);
    fs::remove_dir_all(root).unwrap();
}

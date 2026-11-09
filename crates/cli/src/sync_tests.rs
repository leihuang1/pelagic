//! Parse real CLI arguments, then observe actual publication in this process.
use super::*;
use native_output::directory::sync_observer::{Kind, Registration};
use std::ffi::OsString;
use std::fs;
use std::path::Path;

fn check(model: &str, command: &str) {
    let root = std::env::temp_dir().join(format!(
        "pelagic-sync-{model}-{command}-{}",
        std::process::id()
    ));
    fs::create_dir(&root).unwrap();
    let cases = root.join("cases");
    fs::create_dir(&cases).unwrap();
    let name = if model == "kraken" {
        "pekeris"
    } else {
        "field-g"
    };
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join(format!("../../examples/cases/{model}/{name}/{name}.json"));
    let count = if command == "run-many" { 4 } else { 1 };
    for index in 0..count {
        fs::copy(&fixture, cases.join(format!("c{index}.json"))).unwrap();
    }
    let observer = Registration::new(&root);
    for no_sync in [false, true] {
        let output = root.join(if no_sync { "scratch" } else { "durable" });
        let input = if count == 1 {
            cases.join("c0.json")
        } else {
            cases.clone()
        };
        let mut args: Vec<OsString> = ["pelagic", model, command].map(Into::into).to_vec();
        args.extend([
            input.into_os_string(),
            "--output".into(),
            output.clone().into_os_string(),
            "--threads".into(),
            "1".into(),
        ]);
        if count > 1 {
            args.extend(["--workers".into(), "4".into()]);
        }
        if no_sync {
            args.push("--no-sync".into());
        }
        assert_eq!(
            execute(Cli::try_parse_from(args).unwrap()),
            ExitCode::SUCCESS
        );
        let mut expected = Vec::new();
        for index in 0..count {
            let directory = if count == 1 {
                output.clone()
            } else {
                output.join(format!("c{index}"))
            };
            let manifest: serde_json::Value =
                serde_json::from_slice(&fs::read(directory.join("pelagic-manifest.json")).unwrap())
                    .unwrap();
            if !no_sync {
                for artifact in manifest["artifacts"].as_array().unwrap() {
                    expected.push((artifact["name"].as_str().unwrap().to_owned(), Kind::File));
                }
                expected.push(("pelagic-manifest.json".into(), Kind::File));
                if cfg!(target_vendor = "apple") && count > 1 {
                    expected.push(("pelagic-manifest.json".into(), Kind::SharedBarrier));
                }
            }
        }
        let (flushes, mut actual): (Vec<_>, Vec<_>) = observer
            .take()
            .into_iter()
            .map(|event| {
                (
                    event
                        .path
                        .file_name()
                        .unwrap()
                        .to_string_lossy()
                        .into_owned(),
                    event.kind,
                )
            })
            .partition(|(_, kind)| *kind == Kind::DriveFlush);
        actual.sort();
        expected.sort();
        assert_eq!(actual, expected, "{model} {command}, no_sync={no_sync}");
        // Apple flushes the drive once per barrier, and one shared barrier can cover several cases.
        let allowed = if no_sync || !cfg!(target_vendor = "apple") {
            0..=0
        } else {
            1..=count
        };
        assert!(
            allowed.contains(&flushes.len())
                && flushes
                    .iter()
                    .all(|(name, _)| name == "pelagic-manifest.json"),
            "{model} {command}, no_sync={no_sync}: {flushes:?}"
        );
    }
    drop(observer);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn kraken_run_sync_policy() {
    check("kraken", "run");
}

#[test]
fn bellhop_run_sync_policy() {
    check("bellhop", "run");
}

#[test]
fn kraken_run_many_sync_policy() {
    check("kraken", "run-many");
}

#[test]
fn bellhop_run_many_sync_policy() {
    check("bellhop", "run-many");
}

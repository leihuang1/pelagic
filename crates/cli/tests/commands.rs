use std::process::Command;

#[test]
fn unified_cli_routes_solver_commands_and_rejects_invalid_formats_and_old_overwrite() {
    for args in [
        vec!["--help"],
        vec!["bellhop", "--help"],
        vec!["bellhop", "validate", "--help"],
        vec!["bellhop", "export", "--help"],
        vec!["bellhop", "run", "--help"],
        vec!["bellhop", "run-many", "--help"],
        vec!["kraken", "--help"],
        vec!["kraken", "export", "--help"],
        vec!["kraken", "run", "--help"],
        vec!["kraken", "run-many", "--help"],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_pelagic"))
            .args(&args)
            .output()
            .unwrap();
        assert!(output.status.success(), "{args:?}");
        assert!(String::from_utf8_lossy(&output.stdout).contains("pelagic"));
    }
    for args in [
        vec!["run", "case.env"],
        vec!["bellhop", "run", "case.env", "--format", "csv"],
        vec!["kraken", "run", "case.env", "--overwrite"],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_pelagic"))
            .args(&args)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(2), "{args:?}");
    }
}

#[test]
fn thread_count_never_changes_native_products() {
    let examples = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/cases");
    let root = std::env::temp_dir().join(format!("pelagic-threads-{}", std::process::id()));
    for (index, (model, case)) in [
        ("bellhop", "bellhop/munk-arrivals/munk-arrivals.json"),
        ("bellhop", "bellhop/field-g/field-g.json"),
        ("kraken", "kraken/gulf/gulf-coupled.json"),
    ]
    .into_iter()
    .enumerate()
    {
        let mut products = Vec::new();
        for (flag, environment) in [(Some("1"), None), (Some("3"), None), (None, Some("2"))] {
            let output = root.join(format!("{index}-{}", products.len()));
            let mut command = Command::new(env!("CARGO_BIN_EXE_pelagic"));
            if let Some(threads) = flag {
                command.args(["--threads", threads]);
            }
            if let Some(threads) = environment {
                command.env("PELAGIC_THREADS", threads);
            }
            let status = command
                .args([model, "run"])
                .arg(examples.join(case))
                .arg("--output")
                .arg(&output)
                .output()
                .unwrap();
            assert!(status.status.success(), "{case}: {status:?}");
            let mut files: Vec<_> = std::fs::read_dir(&output)
                .unwrap()
                .map(|entry| entry.unwrap().path())
                .collect();
            files.sort();
            products.push(
                files
                    .iter()
                    .map(|path| {
                        (
                            path.file_name().unwrap().to_owned(),
                            std::fs::read(path).unwrap(),
                        )
                    })
                    .collect::<Vec<_>>(),
            );
        }
        assert!(
            products.iter().all(|product| *product == products[0]),
            "{case}"
        );
    }
    let _ = std::fs::remove_dir_all(root);
    for threads in ["0", "-1", "many"] {
        let output = Command::new(env!("CARGO_BIN_EXE_pelagic"))
            .args(["--threads", threads, "version"])
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(2), "{threads}");
    }
}

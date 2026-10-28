use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use serde_json::Value;

fn directory(name: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!("pelagic-run-many-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&path);
    fs::create_dir_all(&path).unwrap();
    path
}

fn example(model: &str, name: &str, extension: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/cases")
        .join(model)
        .join(name)
        .join(format!("{name}.{extension}"))
}

fn pelagic(args: &[&dyn AsRef<std::ffi::OsStr>]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_pelagic"))
        .args(args.iter().map(AsRef::as_ref))
        .output()
        .unwrap()
}

/// Every parseable status line in `output`, in file order.
fn records(output: &Path) -> Vec<Value> {
    let mut paths: Vec<_> = fs::read_dir(output)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|e| e == "ndjson"))
        .collect();
    paths.sort();
    paths
        .iter()
        .flat_map(|path| {
            fs::read_to_string(path)
                .unwrap()
                .lines()
                .filter_map(|line| serde_json::from_str(line).ok())
                .collect::<Vec<Value>>()
        })
        .collect()
}

fn outcomes(output: &Path) -> BTreeMap<String, String> {
    records(output)
        .into_iter()
        .map(|r| {
            (
                r["stem"].as_str().unwrap().to_owned(),
                r["outcome"].as_str().unwrap().to_owned(),
            )
        })
        .collect()
}

fn files(directory: &Path) -> BTreeMap<String, Vec<u8>> {
    fs::read_dir(directory)
        .unwrap()
        .map(|entry| {
            let entry = entry.unwrap();
            (
                entry.file_name().into_string().unwrap(),
                fs::read(entry.path()).unwrap(),
            )
        })
        .collect()
}

#[test]
fn shards_partition_the_sorted_cases_and_each_product_matches_run() {
    let root = directory("shards");
    let cases = root.join("cases");
    fs::create_dir(&cases).unwrap();
    for index in 0..5 {
        fs::copy(
            example("kraken", "pekeris", "json"),
            cases.join(format!("c{index}.json")),
        )
        .unwrap();
    }
    for extension in ["env", "flp"] {
        fs::copy(
            example("kraken", "pekeris", extension),
            cases.join(format!("legacy.{extension}")),
        )
        .unwrap();
    }
    let output = root.join("results");
    let mut shards = Vec::new();
    for index in 0..3 {
        let shard = format!("{index}/3");
        let run = pelagic(&[
            &"kraken",
            &"run-many",
            &cases,
            &"--output",
            &output,
            &"--shard",
            &shard,
        ]);
        assert!(
            run.status.success(),
            "{}",
            String::from_utf8_lossy(&run.stderr)
        );
        let status = output.join(format!("status-{index}-of-3.ndjson"));
        let stems: Vec<String> = fs::read_to_string(status)
            .unwrap()
            .lines()
            .map(|line| {
                serde_json::from_str::<Value>(line).unwrap()["stem"]
                    .as_str()
                    .unwrap()
                    .to_owned()
            })
            .collect();
        shards.push(stems.into_iter().collect::<BTreeSet<_>>());
    }
    // Sorted: c0..c4, legacy; case j belongs to shard j % 3.
    let expected = [["c0", "c3"], ["c1", "c4"], ["c2", "legacy"]];
    for (shard, expected) in shards.iter().zip(expected) {
        assert_eq!(shard, &expected.into_iter().map(String::from).collect());
    }
    for (stem, case) in [
        ("c2", cases.join("c2.json")),
        ("legacy", cases.join("legacy.env")),
    ] {
        let single = root.join(format!("run-{stem}"));
        assert!(
            pelagic(&[&"kraken", &"run", &case, &"--output", &single])
                .status
                .success()
        );
        assert_eq!(files(&output.join(stem)), files(&single), "{stem}");
    }
    let bellhop = root.join("bellhop");
    let input = example("bellhop", "field-g", "json");
    assert!(
        pelagic(&[&"bellhop", &"run-many", &input, &"--output", &bellhop])
            .status
            .success()
    );
    let single = root.join("run-field-g");
    assert!(
        pelagic(&[&"bellhop", &"run", &input, &"--output", &single])
            .status
            .success()
    );
    assert_eq!(files(&bellhop.join("field-g")), files(&single));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn a_failing_case_is_recorded_and_the_others_still_run() {
    let root = directory("failures");
    let cases = root.join("cases");
    fs::create_dir(&cases).unwrap();
    for name in ["a", "c"] {
        fs::copy(
            example("kraken", "pekeris", "json"),
            cases.join(format!("{name}.json")),
        )
        .unwrap();
    }
    fs::write(cases.join("b.json"), r#"{"pelagic": 2}"#).unwrap();
    let output = root.join("results");
    let run = pelagic(&[
        &"--diagnostics",
        &"json",
        &"kraken",
        &"run-many",
        &cases,
        &"--output",
        &output,
    ]);
    assert_eq!(run.status.code(), Some(2));
    let stderr: Value = serde_json::from_slice(&run.stderr).unwrap();
    assert_eq!(
        stderr["case"],
        cases.join("b.json").to_string_lossy().as_ref()
    );
    assert_eq!(stderr["diagnostics"][0]["code"], "KR0103");
    let records = records(&output);
    let failed: Vec<_> = records
        .iter()
        .filter(|r| r["outcome"] == "failed")
        .collect();
    assert_eq!(failed.len(), 1);
    assert_eq!(failed[0]["stem"], "b");
    assert_eq!(failed[0]["exit_code"], 2);
    // The file was read, so its bytes are fingerprinted even though it is invalid.
    assert_eq!(failed[0]["input_sha256"].as_str().unwrap().len(), 64);
    assert_eq!(failed[0]["diagnostics"][0]["code"], "KR0103");
    for record in records.iter().filter(|r| r["outcome"] == "ok") {
        assert_eq!(record["status_version"], 1);
        assert_eq!(record["format"], "legacy");
        assert_eq!(record["exit_code"], 0);
        assert!(
            output
                .join(record["stem"].as_str().unwrap())
                .join("pelagic-manifest.json")
                .is_file()
        );
    }
    assert_eq!(records.len(), 3);
    // Text diagnostics name the case at the start of each line.
    let text = pelagic(&[&"kraken", &"run-many", &cases, &"--output", &output]);
    let stderr = String::from_utf8(text.stderr).unwrap();
    assert!(
        stderr
            .lines()
            .all(|line| line.starts_with(&*cases.join("b.json").to_string_lossy())),
        "{stderr}"
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn resume_skips_only_current_results_after_a_truncated_status_file() {
    let root = directory("resume");
    let cases = root.join("cases");
    fs::create_dir(&cases).unwrap();
    for name in ["a", "b", "c", "d"] {
        fs::copy(
            example("kraken", "pekeris", "json"),
            cases.join(format!("{name}.json")),
        )
        .unwrap();
    }
    let output = root.join("results");
    let run = |resume: bool| {
        let mut args: Vec<&dyn AsRef<std::ffi::OsStr>> = vec![
            &"kraken",
            &"run-many",
            &cases,
            &"--output",
            &output,
            &"--workers",
            &"1",
        ];
        if resume {
            args.push(&"--resume");
        }
        pelagic(&args)
    };
    assert!(run(false).status.success());
    // A killed job leaves the last record cut short: b's, with one worker.
    let status = output.join("status.ndjson");
    let text = fs::read_to_string(&status).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    assert!(lines[1].contains(r#""stem":"b""#));
    fs::write(
        &status,
        format!("{}\n{}", lines[0], &lines[1][..lines[1].len() / 2]),
    )
    .unwrap();
    // c's input changes; d's results are edited by hand.
    let mut c: Value = serde_json::from_slice(&fs::read(cases.join("c.json")).unwrap()).unwrap();
    c["title"] = "changed".into();
    fs::write(cases.join("c.json"), serde_json::to_vec(&c).unwrap()).unwrap();
    let edited = fs::read_dir(output.join("d"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| path.extension().is_some_and(|e| e == "prt"))
        .unwrap();
    fs::write(&edited, b"my notes").unwrap();
    let resumed = run(true);
    // d's edit is protected: its rerun cannot replace the modified file.
    assert_eq!(resumed.status.code(), Some(4));
    assert_eq!(fs::read(&edited).unwrap(), b"my notes");
    let after: Vec<Value> = fs::read_to_string(&status)
        .unwrap()
        .lines()
        .skip(2)
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    let outcome: BTreeMap<_, _> = after
        .iter()
        .map(|r| (r["stem"].as_str().unwrap(), r["outcome"].as_str().unwrap()))
        .collect();
    assert_eq!(
        outcome,
        BTreeMap::from([("a", "skipped"), ("b", "ok"), ("c", "ok"), ("d", "failed")])
    );
    // A second resume skips everything recorded as current, including skips.
    fs::remove_file(&edited).unwrap();
    fs::remove_dir_all(output.join("d")).unwrap();
    assert!(run(true).status.success());
    let last = outcomes(&output);
    assert_eq!(last["a"], "skipped");
    assert_eq!(last["b"], "skipped");
    assert_eq!(last["c"], "skipped");
    assert_eq!(last["d"], "ok");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn inputs_come_from_lists_and_flat_directories_and_stems_must_be_distinct() {
    let root = directory("inputs");
    let cases = root.join("cases");
    fs::create_dir_all(cases.join("nested")).unwrap();
    for path in [cases.join("a.json"), cases.join("nested/z.json")] {
        fs::copy(example("bellhop", "field-g", "json"), path).unwrap();
    }
    fs::write(cases.join("notes.txt"), b"not a case").unwrap();
    let output = root.join("results");
    let run = pelagic(&[&"bellhop", &"run-many", &cases, &"--output", &output]);
    assert!(run.status.success());
    assert_eq!(outcomes(&output).into_keys().collect::<Vec<_>>(), ["a"]);
    let mut child = Command::new(env!("CARGO_BIN_EXE_pelagic"))
        .args(["bellhop", "run-many", "--list", "-", "--output"])
        .arg(root.join("listed"))
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    writeln!(
        child.stdin.take().unwrap(),
        "{}\r\n\n{}",
        cases.join("nested/z.json").display(),
        cases.join("a.json").display()
    )
    .unwrap();
    assert!(child.wait().unwrap().success());
    assert_eq!(
        outcomes(&root.join("listed"))
            .into_keys()
            .collect::<Vec<_>>(),
        ["a", "z"]
    );
    // Equal stems, even differing only in case, would share a result directory.
    fs::copy(cases.join("a.json"), cases.join("nested/A.json")).unwrap();
    let rejected = root.join("rejected");
    for args in [
        vec![cases.join("a.json"), cases.join("nested/A.json")],
        vec![cases.join("a.json"), cases.join("a.json")],
    ] {
        let run = pelagic(&[
            &"--diagnostics",
            &"json",
            &"bellhop",
            &"run-many",
            &args[0],
            &args[1],
            &"--output",
            &rejected,
        ]);
        assert_eq!(run.status.code(), Some(2));
        let stderr: Value = serde_json::from_slice(&run.stderr).unwrap();
        assert_eq!(stderr["diagnostics"][0]["code"], "PG0002");
        assert!(
            !rejected.exists(),
            "nothing is written before the inputs are accepted"
        );
    }
    for args in [
        vec!["bellhop", "run-many", "--output", "x"],
        vec!["bellhop", "run-many", "-", "--output", "x"],
        vec![
            "kraken", "run-many", "a.json", "--output", "x", "--shard", "3/3",
        ],
        vec![
            "kraken", "run-many", "a.json", "--output", "x", "--shard", "1",
        ],
        vec![
            "kraken",
            "run-many",
            "a.json",
            "--output",
            "x",
            "--workers",
            "0",
        ],
    ] {
        let run = Command::new(env!("CARGO_BIN_EXE_pelagic"))
            .args(&args)
            .current_dir(&root)
            .output()
            .unwrap();
        assert_eq!(run.status.code(), Some(2), "{args:?}");
        assert!(!root.join("x").exists(), "{args:?}");
    }
    fs::remove_dir_all(root).unwrap();
}

/// Turns the status file into a hazard: a link to a case, or a stranger's file.
type Setup = fn(&Path, &Path);

#[test]
fn the_status_file_is_never_a_link_an_input_or_someone_elses_file() {
    let root = directory("status-file");
    let case = root.join("a.json");
    fs::copy(example("kraken", "pekeris", "json"), &case).unwrap();
    let original = fs::read(&case).unwrap();
    let output = root.join("results");
    fs::create_dir(&output).unwrap();
    let status = output.join("status.ndjson");
    let mut setups: Vec<(&str, Setup)> = vec![
        ("hard link", |case, status| {
            fs::hard_link(case, status).unwrap();
        }),
        ("foreign file", |_, status| {
            fs::write(status, b"my notes\n").unwrap();
        }),
    ];
    #[cfg(unix)]
    setups.push(("symbolic link", |case, status| {
        std::os::unix::fs::symlink(case, status).unwrap();
    }));
    let run = || {
        pelagic(&[
            &"--diagnostics",
            &"json",
            &"kraken",
            &"run-many",
            &case,
            &"--output",
            &output,
        ])
    };
    for (name, setup) in setups {
        let _ = fs::remove_file(&status);
        setup(&case, &status);
        let refused = run();
        assert_eq!(refused.status.code(), Some(4), "{name}");
        let stderr: Value = serde_json::from_slice(&refused.stderr).unwrap();
        assert_eq!(stderr["diagnostics"][0]["code"], "PG0404", "{name}");
        assert_eq!(fs::read(&case).unwrap(), original, "{name}");
        assert!(!output.join("a").exists(), "{name}: nothing is solved");
    }
    // Another batch's status file, hard-linked in, belongs to that batch.
    let other = root.join("other");
    assert!(
        pelagic(&[&"kraken", &"run-many", &case, &"--output", &other])
            .status
            .success()
    );
    let theirs = fs::read(other.join("status.ndjson")).unwrap();
    fs::remove_file(&status).unwrap();
    fs::hard_link(other.join("status.ndjson"), &status).unwrap();
    let refused = run();
    assert_eq!(refused.status.code(), Some(4));
    assert!(String::from_utf8_lossy(&refused.stderr).contains("PG0404"));
    assert_eq!(fs::read(other.join("status.ndjson")).unwrap(), theirs);
    assert!(!output.join("a").exists());
    // A status file run-many created is appended to.
    fs::remove_file(&status).unwrap();
    assert!(run().status.success());
    assert!(run().status.success());
    assert_eq!(records(&output).len(), 2);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn resume_reruns_a_legacy_case_whose_arrival_encoding_changed() {
    // JSON export leaves out the legacy ASCII/binary arrival choice; the
    // fingerprint covers the input bytes instead.
    let root = directory("encoding");
    let case = root.join("arrivals.env");
    let ascii = fs::read_to_string(example("bellhop", "munk-arrivals", "env")).unwrap();
    fs::write(&case, &ascii).unwrap();
    let output = root.join("results");
    let resume = || {
        pelagic(&[
            &"bellhop",
            &"run-many",
            &case,
            &"--output",
            &output,
            &"--resume",
        ])
    };
    assert!(resume().status.success());
    let before = files(&output.join("arrivals"));
    let binary = ascii.replace("'A'\t", "'a'\t");
    assert_ne!(binary, ascii);
    fs::write(&case, &binary).unwrap();
    assert!(resume().status.success());
    assert_eq!(outcomes(&output)["arrivals"], "ok");
    let single = root.join("single");
    assert!(
        pelagic(&[&"bellhop", &"run", &case, &"--output", &single])
            .status
            .success()
    );
    let after = files(&output.join("arrivals"));
    assert_eq!(after, files(&single));
    assert_ne!(after, before);
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn a_case_is_fingerprinted_and_solved_from_one_read() {
    use sha2::{Digest, Sha256};
    // A FIFO hands each reader new bytes, as a file replaced between two reads
    // would. The record and the products must both come from the one read.
    let root = directory("snapshot");
    let fifo = root.join("p.json");
    assert!(
        Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .unwrap()
            .success()
    );
    let mut document: Value =
        serde_json::from_slice(&fs::read(example("kraken", "pekeris", "json")).unwrap()).unwrap();
    let mut versions = Vec::new();
    for title in ["first read", "second read"] {
        document["title"] = title.into();
        versions.push(serde_json::to_vec(&document).unwrap());
    }
    let writer = {
        let (fifo, first) = (fifo.clone(), versions[0].clone());
        std::thread::spawn(move || fs::write(&fifo, first).unwrap())
    };
    let output = root.join("results");
    let mut child = Command::new(env!("CARGO_BIN_EXE_pelagic"))
        .args(["kraken", "run-many"])
        .arg(&fifo)
        .arg("--output")
        .arg(&output)
        .stdout(Stdio::null())
        .spawn()
        .unwrap();
    let start = std::time::Instant::now();
    while child.try_wait().unwrap().is_none() {
        if start.elapsed() > std::time::Duration::from_secs(10) {
            // Still running: it is waiting to read the case again. Let it, so
            // that the assertions below fail rather than hang.
            fs::write(&fifo, &versions[1]).unwrap();
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert!(child.wait().unwrap().success());
    writer.join().unwrap();
    let mut hash = Sha256::new();
    hash.update((versions[0].len() as u64).to_le_bytes());
    hash.update(&versions[0]);
    assert_eq!(
        records(&output)[0]["input_sha256"],
        format!("{:x}", hash.finalize())
    );
    let report = fs::read_to_string(output.join("p/p.prt")).unwrap();
    assert!(report.contains("first read"), "{report}");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn resume_continues_after_a_first_record_cut_short() {
    let root = directory("first-record");
    let cases = root.join("cases");
    fs::create_dir(&cases).unwrap();
    for name in ["a", "b"] {
        fs::copy(
            example("kraken", "pekeris", "json"),
            cases.join(format!("{name}.json")),
        )
        .unwrap();
    }
    let output = root.join("results");
    let run = || {
        pelagic(&[
            &"kraken",
            &"run-many",
            &cases,
            &"--output",
            &output,
            &"--workers",
            &"1",
            &"--resume",
        ])
    };
    assert!(run().status.success());
    let status = output.join("status.ndjson");
    let text = fs::read_to_string(&status).unwrap();
    let first = text.lines().next().unwrap();
    // Cut inside the record, and inside its leading `{"status_version":`.
    for cut in [first.len() / 2, 5] {
        fs::write(&status, &first[..cut]).unwrap();
        let resumed = run();
        assert!(
            resumed.status.success(),
            "{}",
            String::from_utf8_lossy(&resumed.stderr)
        );
        let text = fs::read_to_string(&status).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines[0], &first[..cut], "the fragment keeps its own line");
        let outcomes: Vec<Value> = lines[1..]
            .iter()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(outcomes.len(), 2);
        // Both records were lost, so both cases ran again.
        assert!(
            outcomes.iter().all(|r| r["outcome"] == "ok"),
            "{outcomes:?}"
        );
        // With complete records now, a further resume skips both.
        assert!(run().status.success());
        let last = records(&output);
        assert!(
            last[last.len() - 2..]
                .iter()
                .all(|r| r["outcome"] == "skipped")
        );
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn resume_reruns_a_kraken_case_whose_output_quota_changed() {
    let root = directory("quota");
    let case = root.join("p.json");
    fs::copy(example("kraken", "pekeris", "json"), &case).unwrap();
    let output = root.join("results");
    let resume = |quota: &str| {
        pelagic(&[
            &"kraken",
            &"run-many",
            &case,
            &"--output",
            &output,
            &"--resume",
            &"--max-output-bytes",
            &quota,
        ])
    };
    let single = root.join("single");
    let run = |quota: &str| {
        pelagic(&[
            &"kraken",
            &"run",
            &case,
            &"--output",
            &single,
            &"--max-output-bytes",
            &quota,
        ])
    };
    assert!(resume("1000000").status.success());
    // The quota is recorded in the results, so another quota that fits reruns
    // the case and gives what `run` gives with it.
    assert!(resume("2000000").status.success());
    assert_eq!(outcomes(&output)["p"], "ok");
    assert!(run("2000000").status.success());
    assert_eq!(files(&output.join("p")), files(&single));
    // A quota too small for the FIELD fails the case, as it fails `run`.
    assert_eq!(run("1").status.code(), Some(4));
    assert_eq!(resume("1").status.code(), Some(4));
    assert_eq!(outcomes(&output)["p"], "failed");
    fs::remove_dir_all(root).unwrap();
}

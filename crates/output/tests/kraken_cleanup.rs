use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use kraken::ModeSolver;
use output::directory::{Format, verified_manifest_sha256};
use output::kraken::{DEFAULT_MAX_OUTPUT_BYTES, RunError, run_directory};

#[test]
fn field_payload_admission_precedes_solving_and_publication_for_every_source_kind() {
    let directory = std::env::temp_dir().join(format!("kraken-admission-{}", std::process::id()));
    fs::create_dir(&directory).unwrap();
    let fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../kraken/tests/fixtures/PekerisBroadband");
    let env = directory.join("case.env");
    let flp = directory.join("case.flp");
    let json = directory.join("case.json");
    let output = directory.join("result");
    // Valid input, but 5 Hz cannot produce modes: admission must reject before solving.
    let source = fs::read_to_string(fixture.with_extension("env")).unwrap();
    fs::write(&env, source.replace("75.0 50.0 62.5 /", "5.0 50.0 62.5 /")).unwrap();
    fs::copy(fixture.with_extension("flp"), &flp).unwrap();
    let input = kraken::input::load_legacy(&env, &flp, ModeSolver::Kraken).unwrap();
    fs::write(
        &json,
        serde_json::to_vec(&input.document().unwrap()).unwrap(),
    )
    .unwrap();
    fs::write(&output, b"previous complete result").unwrap();

    // Three frequencies × one source × three depths × three ranges × 8 bytes = 216.
    // 215 admits each individual frequency but must reject the complete sequence.
    for maximum in [0, 215] {
        let check = |result: Result<_, RunError>| {
            let expected = if maximum == 0 {
                "max_output_bytes must be positive".to_owned()
            } else {
                format!("FIELD payload exceeds {maximum} bytes")
            };
            assert!(
                matches!(result, Err(RunError::Output(ref message)) if *message == expected),
                "{result:?}"
            );
            assert_eq!(fs::read(&output).unwrap(), b"previous complete result");
            assert_eq!(
                fs::read_dir(&directory).unwrap().count(),
                4,
                "admission created scratch or locks"
            );
        };
        for format in [Format::Legacy, Format::Hdf5, Format::Both] {
            for input in [&env, &json] {
                let input = kraken::input::load(input, None, None).unwrap();
                check(run_directory(&input, &output, format, maximum));
            }
        }
    }
    fs::remove_dir_all(directory).unwrap();
}

/// Artifact bytes by name. Reading fails on a directory, so left-behind scratch panics here.
fn snapshot(root: &Path) -> BTreeMap<String, Vec<u8>> {
    fs::read_dir(root)
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
fn failed_runs_release_scratch_before_retry_in_the_same_process() {
    let directory = std::env::temp_dir().join(format!("kraken-cleanup-{}", std::process::id()));
    fs::create_dir(&directory).unwrap();
    let fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../kraken/tests/fixtures/PekerisBroadband");
    let source = fs::read_to_string(fixture.with_extension("env")).unwrap();
    let env = directory.join("case.env");
    let flp = directory.join("case.flp");
    let output = directory.join("result");
    fs::copy(fixture.with_extension("flp"), &flp).unwrap();
    for format in [Format::Hdf5, Format::Both] {
        let run = |quota| {
            let input = kraken::input::load(&env, None, None).unwrap();
            run_directory(&input, &output, format, quota)
        };
        for simulation_failure in [false, true] {
            // Different input bytes, so that the retry must replace these artifacts.
            fs::write(&env, source.replacen("Derived", "Previous", 1)).unwrap();
            run(DEFAULT_MAX_OUTPUT_BYTES).unwrap();
            let previous = snapshot(&output);
            let previous_manifest = verified_manifest_sha256(&output).unwrap();
            fs::write(
                &env,
                if simulation_failure {
                    source.replace("75.0 50.0 62.5 /", "75.0 5.0 62.5 /")
                } else {
                    source.clone()
                },
            )
            .unwrap();
            let error = run(if simulation_failure {
                DEFAULT_MAX_OUTPUT_BYTES
            } else {
                4096
            })
            .unwrap_err();
            if simulation_failure {
                assert!(matches!(
                    error,
                    RunError::Simulation {
                        frequency_index: 1,
                        ..
                    }
                ));
            } else {
                assert!(
                    matches!(error, RunError::Output(ref message) if message.contains("HDF5 file exceeds"))
                );
            }
            assert_eq!(snapshot(&output), previous, "failed run changed its output");
            // Do not rely on subprocess exit to release HDF5 resources before retry. Only Windows
            // can observe a leak here, because Unix removes a file that is still open.
            fs::write(&env, &source).unwrap();
            assert_eq!(run(DEFAULT_MAX_OUTPUT_BYTES).unwrap().frequency_count, 3);
            assert_ne!(
                verified_manifest_sha256(&output).unwrap(),
                previous_manifest
            );
            assert_eq!(
                hdf5::File::open(output.join("case.h5"))
                    .unwrap()
                    .attr("frequency_count")
                    .unwrap()
                    .read_scalar::<u64>()
                    .unwrap(),
                3
            );
        }
    }
    fs::remove_dir_all(directory).unwrap();
}

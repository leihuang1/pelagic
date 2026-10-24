use std::fs;
use std::path::Path;

use kraken::ModeSolver;
use output::kraken::{DEFAULT_MAX_OUTPUT_BYTES, RunError, run_legacy};

#[test]
fn field_payload_admission_precedes_solving_and_publication_for_every_entry() {
    use output::directory::Format;
    use output::kraken::{run_json, run_json_directory, run_legacy_directory};

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
        check(run_legacy(
            &env,
            &flp,
            &output,
            ModeSolver::Kraken,
            true,
            maximum,
        ));
        check(run_json(&json, &output, None, true, maximum));
        for format in [Format::Legacy, Format::Hdf5, Format::Both] {
            check(run_legacy_directory(
                &env,
                &flp,
                &output,
                ModeSolver::Kraken,
                format,
                maximum,
            ));
            check(run_json_directory(&json, &output, None, format, maximum));
        }
    }
    fs::remove_dir_all(directory).unwrap();
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
    let output = directory.join("result.h5");
    let scratch = directory.join("result.h5.tmp");
    fs::copy(fixture.with_extension("flp"), &flp).unwrap();
    let run = |quota| run_legacy(&env, &flp, &output, ModeSolver::Kraken, true, quota);
    for simulation_failure in [false, true] {
        fs::write(&output, b"previous complete result").unwrap();
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
        assert!(!scratch.exists(), "failed run left its scratch file");
        assert_eq!(fs::read(&output).unwrap(), b"previous complete result");
        // Do not rely on subprocess exit to release HDF5 resources before retry.
        fs::write(&env, &source).unwrap();
        assert_eq!(run(DEFAULT_MAX_OUTPUT_BYTES).unwrap().frequency_count, 3);
        assert!(!scratch.exists());
    }
    fs::remove_dir_all(directory).unwrap();
}

//! Results and diagnostics do not depend on the thread count. Every fixture and
//! example is solved with one and with several threads and compared bit for bit:
//! `Debug` prints the shortest decimal that round-trips each float, so equal
//! text means equal values, signed zeros included.
use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};

use kraken::parallelism::with_threads;
use kraken::{FieldCase, ModeSolver, legacy, solve_frequencies};

fn outcome(cases: &[FieldCase], threads: usize) -> String {
    with_threads(NonZeroUsize::new(threads).unwrap(), || {
        format!("{:?}", solve_frequencies(cases).collect::<Vec<_>>())
    })
}

fn assert_thread_invariant(path: &Path, cases: &[FieldCase]) {
    let single = outcome(cases, 1);
    for threads in [2, 5] {
        assert!(
            outcome(cases, threads) == single,
            "{} differs with {threads} threads",
            path.display()
        );
    }
}

#[test]
fn every_fixture_is_identical_for_any_thread_count() {
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let mut paths: Vec<PathBuf> = std::fs::read_dir(fixtures)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "env"))
        .collect();
    paths.sort();
    let mut solved = 0;
    for env in paths {
        for solver in [ModeSolver::Kraken, ModeSolver::Krakenc] {
            // Some fixtures are valid for one engine only.
            if let Ok(cases) = legacy::load_field_cases(&env, env.with_extension("flp"), solver) {
                assert_thread_invariant(&env, &cases);
                solved += 1;
            }
        }
    }
    assert!(solved > 100, "only {solved} fixtures loaded");
}

#[test]
fn every_example_is_identical_for_any_thread_count() {
    let examples = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/cases/kraken");
    let mut paths: Vec<PathBuf> = std::fs::read_dir(examples)
        .unwrap()
        .flat_map(|entry| std::fs::read_dir(entry.unwrap().path()).unwrap())
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "json")
        })
        .collect();
    paths.sort();
    for path in paths {
        let document =
            kraken::json::decode_case_document(&std::fs::read(&path).unwrap()[..], &path).unwrap();
        let cases = document.cases(&path).unwrap();
        assert_thread_invariant(&path, &cases);
        let largest = with_threads(NonZeroUsize::MAX, || {
            format!("{:?}", solve_frequencies(&cases).collect::<Vec<_>>())
        });
        assert!(
            largest == outcome(&cases, 1),
            "{} differs with usize::MAX threads",
            path.display()
        );
    }
}

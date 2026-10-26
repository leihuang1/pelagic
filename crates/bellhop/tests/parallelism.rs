//! Results and diagnostics do not depend on the thread count. Every fixture and
//! example is solved with one and with several threads and compared bit for bit:
//! `Debug` prints the shortest decimal that round-trips each float, so equal
//! text means equal values, signed zeros included.
use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};

use bellhop::model::RunKind;
use bellhop::parallelism::with_threads;
use bellhop::solver::{SimulationLimits, run};

fn cases() -> Vec<(PathBuf, bellhop::Case)> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let fixtures = root.join("tests/fixtures");
    let mut paths: Vec<PathBuf> = [fixtures.clone(), fixtures.join("golden")]
        .iter()
        .flat_map(|directory| std::fs::read_dir(directory).unwrap())
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "env"))
        .collect();
    let examples = root.join("../../examples/cases/bellhop");
    for entry in std::fs::read_dir(examples).unwrap() {
        let directory = entry.unwrap().path();
        let name = directory.file_name().unwrap().to_owned();
        paths.push(directory.join(name).with_extension("json"));
    }
    paths.sort();
    paths
        .into_iter()
        .map(|path| {
            let case = if path
                .extension()
                .is_some_and(|extension| extension == "json")
            {
                bellhop::json::load_case_document(&std::fs::read(&path).unwrap()[..])
                    .unwrap()
                    .value
            } else {
                bellhop::legacy::load_case(&path).unwrap().value
            };
            (path, case)
        })
        .collect()
}

fn outcome(case: &bellhop::Case, threads: usize, limits: SimulationLimits) -> String {
    with_threads(NonZeroUsize::new(threads).unwrap(), || {
        format!("{:?}", run(case, limits))
    })
}

fn assert_thread_invariant(path: &Path, case: &bellhop::Case, limits: SimulationLimits) {
    let single = outcome(case, 1, limits);
    for threads in [2, 5] {
        assert!(
            outcome(case, threads, limits) == single,
            "{} differs with {threads} threads and {limits:?}",
            path.display()
        );
    }
}

#[test]
fn every_case_is_identical_for_any_thread_count() {
    for (path, case) in cases() {
        assert_thread_invariant(&path, &case, SimulationLimits::default());
    }
}

#[test]
fn resource_limits_apply_at_the_same_ray_for_any_thread_count() {
    let default = SimulationLimits::default();
    let traces = [
        SimulationLimits {
            max_steps_per_ray: 40,
            ..default
        },
        SimulationLimits {
            max_total_ray_points: 4_000,
            ..default
        },
    ];
    let arrivals = [
        SimulationLimits {
            max_arrivals_per_receiver: 2,
            ..default
        },
        SimulationLimits {
            max_total_arrivals: 30,
            ..default
        },
    ];
    let eigenrays = [
        SimulationLimits {
            max_eigenrays: 3,
            ..default
        },
        SimulationLimits {
            max_total_eigenray_points: 300,
            ..default
        },
    ];
    for (path, case) in cases() {
        let specific: &[_] = match case.environment.run.kind {
            RunKind::Arrivals => &arrivals,
            RunKind::Eigenrays => &eigenrays,
            _ => &[],
        };
        for &limits in traces.iter().chain(specific) {
            assert_thread_invariant(&path, &case, limits);
        }
    }
}

#[test]
fn the_largest_thread_count_runs_like_one_thread() {
    for (path, case) in cases() {
        if path
            .extension()
            .is_some_and(|extension| extension == "json")
        {
            let limits = SimulationLimits::default();
            let largest = with_threads(NonZeroUsize::MAX, || format!("{:?}", run(&case, limits)));
            assert!(
                largest == outcome(&case, 1, limits),
                "{} differs with usize::MAX threads",
                path.display()
            );
        }
    }
}

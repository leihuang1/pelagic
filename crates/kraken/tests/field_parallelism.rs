//! Focused checks for ordered, bounded block/profile solve-ahead.
use std::num::NonZeroUsize;
use std::path::Path;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use kraken::parallelism::with_threads;
use kraken::{
    Case, FieldCase, FieldPropagation, ModeSolver, legacy, solve_field, solve_frequencies,
};

fn load(stem: &str, solver: ModeSolver) -> Vec<FieldCase> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(stem);
    legacy::load_field_cases(
        root.with_extension("env"),
        root.with_extension("flp"),
        solver,
    )
    .unwrap()
}

fn threads<T>(count: usize, work: impl FnOnce() -> T) -> T {
    with_threads(NonZeroUsize::new(count).unwrap(), work)
}

fn single(profile: Case) -> FieldCase {
    FieldCase::new(vec![profile], vec![0.0], FieldPropagation::RangeIndependent).unwrap()
}

#[test]
fn ordered_repeated_frequencies_cross_solve_windows() {
    for solver in [ModeSolver::Kraken, ModeSolver::Krakenc] {
        let base = load("Pekeris", solver).remove(0);
        let cases: Vec<_> = [75.0, 50.0, 62.5, 50.0]
            .into_iter()
            .cycle()
            .take(20)
            .map(|frequency| {
                let mut input = base.profiles()[0].clone().into_definition();
                input.frequency_hz = frequency;
                single(Case::from_definition(input).unwrap())
            })
            .collect();
        let expected = threads(1, || {
            cases
                .iter()
                .map(|case| solve_field(case).unwrap())
                .collect::<Vec<_>>()
        });
        assert_eq!(expected[1], expected[3]);
        for count in [1, 2, 5, 8, 16] {
            let actual = threads(count, || {
                solve_frequencies(&cases)
                    .collect::<Result<Vec<_>, _>>()
                    .unwrap()
            });
            assert_eq!(
                format!("{actual:?}"),
                format!("{expected:?}"),
                "{solver:?}, {count}"
            );
        }
    }
}

#[test]
fn profiles_and_nested_frequency_windows_keep_the_serial_field() {
    for solver in [ModeSolver::Kraken, ModeSolver::Krakenc] {
        for stem in ["ProfilesAd", "ProfilesCm"] {
            let case = load(stem, solver).remove(0);
            let expected = threads(1, || solve_field(&case).unwrap());
            let cases = vec![case.clone(); 3];
            for count in [2, 5, 8, 16] {
                threads(count, || {
                    assert_eq!(
                        format!("{:?}", solve_field(&case).unwrap()),
                        format!("{expected:?}")
                    );
                    for result in solve_frequencies(&cases) {
                        assert_eq!(format!("{:?}", result.unwrap()), format!("{expected:?}"));
                    }
                });
            }
        }
    }
}

#[test]
fn first_and_middle_block_errors_stop_in_input_order() {
    for solver in [ModeSolver::Kraken, ModeSolver::Krakenc] {
        let good = load("Pekeris", solver).remove(0);
        let mut input = good.profiles()[0].clone().into_definition();
        input.mesh_points = 10;
        let bad = single(Case::from_definition(input).unwrap());
        let expected = threads(1, || solve_field(&bad).unwrap_err());
        let mut input = good.profiles()[0].clone().into_definition();
        input.frequency_hz = f64::MAX;
        let later_bad = single(Case::from_definition(input).unwrap());
        for failed in [0, 1, 9] {
            let mut cases = vec![good.clone(); failed];
            cases.extend([bad.clone(), later_bad.clone(), good.clone()]);
            for count in [1, 4, 8, 16] {
                threads(count, || {
                    let mut results = solve_frequencies(&cases);
                    for _ in 0..failed {
                        assert!(results.next().unwrap().is_ok());
                    }
                    assert_eq!(results.next().unwrap().unwrap_err(), expected);
                    assert!(results.next().is_none());
                    assert!(results.next().is_none());
                });
            }
        }
    }
}

#[test]
fn first_and_middle_profile_errors_keep_their_input_index() {
    for solver in [ModeSolver::Kraken, ModeSolver::Krakenc] {
        let base = load("ProfilesAd", solver).remove(0);
        for failed in [0, 1, 9] {
            let mut profiles: Vec<_> = base.profiles().iter().cycle().take(12).cloned().collect();
            for profile in &mut profiles[failed..=failed + 1] {
                let mut input = profile.clone().into_definition();
                input.mesh_points = 10;
                *profile = Case::from_definition(input).unwrap();
            }
            let ranges = (0..12).map(|index| f64::from(index) * 800.0).collect();
            let case = FieldCase::new(profiles, ranges, base.propagation()).unwrap();
            let expected = threads(1, || solve_field(&case).unwrap_err());
            assert_eq!(
                expected.diagnostics()[0].field,
                format!("profiles[{failed}].mesh_points")
            );
            for count in [4, 8, 16] {
                assert_eq!(threads(count, || solve_field(&case).unwrap_err()), expected);
            }
        }
    }
}

#[test]
fn independent_mixed_solvers_are_rejected_before_the_mismatched_solve() {
    for solver in [ModeSolver::Kraken, ModeSolver::Krakenc] {
        let first = load("Pekeris", solver).remove(0);
        let mut input = first.profiles()[0].clone().into_definition();
        input.mode_solver = if solver == ModeSolver::Kraken {
            ModeSolver::Krakenc
        } else {
            ModeSolver::Kraken
        };
        input.mesh_points = 10; // Solving this instead of rejecting it would report KR0302.
        let cases = [
            first.clone(),
            single(Case::from_definition(input).unwrap()),
            first,
        ];
        threads(8, || {
            let mut results = solve_frequencies(&cases);
            assert!(results.next().unwrap().is_ok());
            let report = results.next().unwrap().unwrap_err();
            assert_eq!(report.diagnostics()[0].code, "KR0201");
            assert_eq!(report.diagnostics()[0].field, "mode_solver");
            assert!(results.next().is_none());
        });
    }
}

#[test]
fn frequency_iteration_is_lazy_and_cancellation_discards_buffered_successes() {
    for solver in [ModeSolver::Kraken, ModeSolver::Krakenc] {
        let cases = vec![load("Pekeris", solver).remove(0); 12];
        let token = Arc::new(AtomicBool::new(true));
        let mut lazy =
            kraken::cancellation::with_token(token.clone(), || solve_frequencies(&cases));
        assert!(
            threads(8, || lazy.next().unwrap()).is_ok(),
            "construction must not solve"
        );
        threads(8, || {
            kraken::cancellation::with_token(token.clone(), || {
                let mut cancelled = solve_frequencies(&cases);
                assert_eq!(
                    cancelled.next().unwrap().unwrap_err().diagnostics()[0].code,
                    "KR0304"
                );
                assert!(cancelled.next().is_none());
                token.store(false, Ordering::Relaxed);
                let mut results = solve_frequencies(&cases);
                // The second pull solves ahead, leaving later results buffered.
                assert!(results.next().unwrap().is_ok());
                assert!(results.next().unwrap().is_ok());
                token.store(true, Ordering::Relaxed);
                assert_eq!(
                    results.next().unwrap().unwrap_err().diagnostics()[0].code,
                    "KR0304"
                );
                token.store(false, Ordering::Relaxed);
                assert!(results.next().is_none());
            });
        });
    }
}

#[test]
fn cancellation_between_blocks_precedes_cached_solver_errors_but_not_mixed_solver_rejection() {
    let good = load("Pekeris", ModeSolver::Kraken).remove(0);
    for solver in [ModeSolver::Kraken, ModeSolver::Krakenc] {
        let mut input = good.profiles()[0].clone().into_definition();
        input.mesh_points = 10;
        input.mode_solver = solver;
        // The first block solves alone; the second pull then solves the failing one too.
        let cases = [
            good.clone(),
            good.clone(),
            single(Case::from_definition(input).unwrap()),
        ];
        let token = Arc::new(AtomicBool::new(false));
        threads(8, || {
            kraken::cancellation::with_token(token.clone(), || {
                let mut results = solve_frequencies(&cases);
                assert!(results.next().unwrap().is_ok());
                assert!(results.next().unwrap().is_ok());
                token.store(true, Ordering::Relaxed);
                assert_eq!(
                    results.next().unwrap().unwrap_err().diagnostics()[0].code,
                    if solver == ModeSolver::Kraken {
                        "KR0304"
                    } else {
                        "KR0201"
                    }
                );
                assert!(results.next().is_none());
            });
        });
    }
}

#[test]
fn elastic_frequency_and_profile_sequences_keep_sequential_search_bounds() {
    for (stem, counts, profile_counts) in [
        ("FiniteElasticPower", [6, 6], [8, 5, 5]),
        ("ElasticHalfTopBroadband", [3, 3], [4, 2, 4]),
    ] {
        let cases = load(stem, ModeSolver::Kraken);
        let expected = threads(1, || {
            solve_frequencies(&cases)
                .collect::<Result<Vec<_>, _>>()
                .unwrap()
        });
        assert_eq!(expected[1].modes[0].modes.len(), counts[0]);
        assert_eq!(expected[2].modes[0].modes.len(), counts[1]);
        for count in [4, 8, 16] {
            let actual = threads(count, || {
                solve_frequencies(&cases)
                    .collect::<Result<Vec<_>, _>>()
                    .unwrap()
            });
            assert_eq!(format!("{actual:?}"), format!("{expected:?}"));
        }
        // The narrower middle profile leaves fewer modes. Finite elastic layers carry
        // that count into the third profile's search; an elastic top alone does not.
        let profile = cases[0].profiles()[0].clone();
        let mut narrow = profile.clone().into_definition();
        narrow.c_high_mps = 1600.0;
        let field = FieldCase::new(
            vec![
                profile.clone(),
                Case::from_definition(narrow).unwrap(),
                profile,
            ],
            vec![0.0, 500.0, 1000.0],
            FieldPropagation::Adiabatic,
        )
        .unwrap();
        let expected = threads(1, || solve_field(&field).unwrap());
        assert_eq!(
            expected
                .modes
                .iter()
                .map(|set| set.modes.len())
                .collect::<Vec<_>>(),
            profile_counts
        );
        // At and above the thread count where windows open.
        for count in [8, 16] {
            assert_eq!(
                format!("{:?}", threads(count, || solve_field(&field).unwrap())),
                format!("{expected:?}")
            );
        }
    }
}

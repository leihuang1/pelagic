use std::sync::{Arc, atomic::AtomicBool};

use bellhop::solver::{SimulationLimits, run};

fn case() -> bellhop::Case {
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/golden/Field_G.env");
    bellhop::legacy::load_case(&path).unwrap().value
}

#[test]
fn cancelled_token_stops_the_run_with_a_stable_code() {
    let error = bellhop::cancellation::with_token(Arc::new(AtomicBool::new(true)), || {
        run(&case(), SimulationLimits::default())
    })
    .unwrap_err();
    let codes: Vec<_> = error.diagnostics().iter().map(|d| d.code).collect();
    assert_eq!(codes, ["BH0304"]);
}

#[test]
fn token_is_scoped_to_its_thread_and_restored_afterwards() {
    let case = case();
    bellhop::cancellation::with_token(Arc::new(AtomicBool::new(true)), || {
        std::thread::scope(|scope| {
            let other = scope.spawn(|| run(&case, SimulationLimits::default()));
            assert!(other.join().unwrap().is_ok());
        });
    });
    assert!(!bellhop::cancellation::cancelled());
    assert!(run(&case, SimulationLimits::default()).is_ok());
}

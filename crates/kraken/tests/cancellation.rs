use std::path::Path;
use std::sync::{Arc, atomic::AtomicBool};

use kraken::{ModeSolver, legacy, solve_frequencies};

fn cases() -> Vec<kraken::FieldCase> {
    let env = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/Pekeris.env");
    legacy::load_field_cases(&env, env.with_extension("flp"), ModeSolver::Kraken).unwrap()
}

#[test]
fn cancelled_token_stops_both_engines_with_a_stable_code() {
    for solver in [ModeSolver::Kraken, ModeSolver::Krakenc] {
        let env = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/Pekeris.env");
        let cases = legacy::load_field_cases(&env, env.with_extension("flp"), solver).unwrap();
        let error = kraken::cancellation::with_token(Arc::new(AtomicBool::new(true)), || {
            solve_frequencies(&cases).next().unwrap()
        })
        .unwrap_err();
        let codes: Vec<_> = error.diagnostics().iter().map(|d| d.code).collect();
        assert_eq!(codes, ["KR0304"], "{solver:?}");
    }
}

#[test]
fn uncancelled_token_does_not_change_results() {
    let cases = cases();
    let plain: Vec<_> = solve_frequencies(&cases).map(Result::unwrap).collect();
    let scoped: Vec<_> = kraken::cancellation::with_token(Arc::new(AtomicBool::new(false)), || {
        solve_frequencies(&cases).map(Result::unwrap).collect()
    });
    assert_eq!(plain, scoped);
}

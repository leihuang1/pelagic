//! Run from the repository root:
//! `cargo run --release -p kraken --example prepared_modes -- crates/kraken/tests/fixtures/Pekeris.env`
use std::error::Error;
use std::hint::black_box;
use std::path::PathBuf;
use std::time::Instant;

use kraken::{Case, PressureField, legacy, prepare_modes, solve};

fn check_bits(actual: &PressureField, expected: &PressureField) {
    assert_eq!(actual.source_depths_m, expected.source_depths_m);
    assert_eq!(actual.receiver_depths_m, expected.receiver_depths_m);
    assert_eq!(actual.receiver_ranges_m, expected.receiver_ranges_m);
    assert_eq!(actual.receiver_offsets_m, expected.receiver_offsets_m);
    assert_eq!(actual.pressure.len(), expected.pressure.len());
    for (actual, expected) in actual.pressure.iter().zip(&expected.pressure) {
        assert_eq!(actual.re.to_bits(), expected.re.to_bits());
        assert_eq!(actual.im.to_bits(), expected.im.to_bits());
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    let env_path = PathBuf::from(std::env::args_os().nth(1).ok_or("pass a .env path")?);
    let case = legacy::load_case(&env_path, env_path.with_extension("flp"))?;
    let queries: Vec<Vec<f64>> = (0..8)
        .map(|index| {
            case.receiver_ranges_m
                .iter()
                .map(|range| range + f64::from(index) * 100.0)
                .collect()
        })
        .collect();
    let start = Instant::now();
    let prepared = prepare_modes(&case)?;
    let prepare_time = start.elapsed();
    let start = Instant::now();
    let first = prepared.field(&queries[0], &case.receiver_offsets_m)?;
    let first_query_time = start.elapsed();
    let start = Instant::now();
    let second = prepared.field(&queries[1], &case.receiver_offsets_m)?;
    let warm_query_time = start.elapsed();

    // Independently solve both queries; reusing modes must not change any pressure bit.
    for (ranges, field) in [(&queries[0], &first), (&queries[1], &second)] {
        let mut definition = case.clone().into_definition();
        definition.receiver_ranges_m.clone_from(ranges);
        let full = solve(&Case::from_definition(definition)?)?;
        check_bits(field, &full.field);
    }
    drop((first, second, prepared));

    // Illustrative timings only: use repeated, interleaved trials for benchmarks.
    // Drop each output before the next query, rather than retaining eight grids.
    let start = Instant::now();
    for ranges in &queries {
        let mut definition = case.clone().into_definition();
        definition.receiver_ranges_m.clone_from(ranges);
        black_box(solve(&Case::from_definition(definition)?)?);
    }
    let full_batch_time = start.elapsed();
    let start = Instant::now();
    let prepared = prepare_modes(&case)?;
    for ranges in &queries {
        black_box(prepared.field(ranges, &case.receiver_offsets_m)?);
    }
    let reused_batch_time = start.elapsed();
    println!(
        "modes={}, pressure_bits_equal=true",
        prepared.modes().modes.len()
    );
    println!(
        "prepare={prepare_time:?}, first_query={first_query_time:?}, warm_query={warm_query_time:?}"
    );
    println!("eight_full={full_batch_time:?}, prepare_plus_eight_queries={reused_batch_time:?}");
    Ok(())
}

//! Prepared FIELD queries must preserve the ordinary solve's arithmetic and validation.
use std::num::NonZeroUsize;
use std::path::Path;
use std::sync::{Arc, atomic::AtomicBool};

use kraken::{
    Case, DiagnosticReport, ModeSet, PressureField, SourcePatternPoint, legacy, prepare_modes,
    solve,
};

fn load(stem: &str) -> Case {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(stem);
    legacy::load_case(root.with_extension("env"), root.with_extension("flp")).unwrap()
}

fn threads<T>(count: usize, operation: impl FnOnce() -> T) -> T {
    kraken::parallelism::with_threads(NonZeroUsize::new(count).unwrap(), operation)
}

fn query_case(case: &Case, ranges: &[f64], offsets: &[f64]) -> Result<Case, DiagnosticReport> {
    let mut definition = case.clone().into_definition();
    definition.receiver_ranges_m = ranges.to_vec();
    definition.receiver_offsets_m = offsets.to_vec();
    Case::from_definition(definition)
}

fn bits(values: &[f64]) -> Vec<u64> {
    values.iter().map(|value| value.to_bits()).collect()
}

fn pressure_bits(field: &PressureField) -> Vec<(u64, u64)> {
    field
        .pressure
        .iter()
        .map(|p| (p.re.to_bits(), p.im.to_bits()))
        .collect()
}

fn same_field(actual: &PressureField, expected: &PressureField) {
    assert_eq!(
        bits(&actual.source_depths_m),
        bits(&expected.source_depths_m)
    );
    assert_eq!(
        bits(&actual.receiver_depths_m),
        bits(&expected.receiver_depths_m)
    );
    assert_eq!(
        bits(&actual.receiver_ranges_m),
        bits(&expected.receiver_ranges_m)
    );
    assert_eq!(
        bits(&actual.receiver_offsets_m),
        bits(&expected.receiver_offsets_m)
    );
    assert_eq!(pressure_bits(actual), pressure_bits(expected));
}

fn same_modes(actual: &ModeSet, expected: &ModeSet) {
    // Debug uses round-tripping floats, including signed zeros, for every modal member.
    assert_eq!(format!("{actual:?}"), format!("{expected:?}"));
}

#[test]
fn prepared_queries_match_full_solve_bit_for_bit_across_threads() {
    for stem in ["Pekeris", "MunkLossless", "SductTrapped"] {
        let base = load(stem);
        let reference = threads(1, || solve(&base).unwrap());
        for count in [1, 4] {
            let prepared = threads(count, || prepare_modes(&base).unwrap());
            same_modes(prepared.modes(), &reference.modes);
            assert_eq!(prepared.case(), &base);
            for range_count in [1, 7, 8, 17] {
                let ranges: Vec<_> = (0..range_count)
                    .map(|i| 1000.0 + f64::from(i) * 123.0)
                    .collect();
                let mut offsets = vec![40.0; base.receiver_depths_m.len()];
                offsets[0] = -25.0;
                let query = query_case(&base, &ranges, &offsets).unwrap();
                let expected = threads(1, || solve(&query).unwrap());
                let actual = threads(count, || prepared.field(&ranges, &offsets).unwrap());
                same_field(&actual, &expected.field);
                same_modes(prepared.modes(), &expected.modes);
                same_field(
                    &threads(count, || prepared.field(&ranges, &offsets).unwrap()),
                    &actual,
                );
            }
        }
    }
}

#[test]
fn invalid_queries_return_the_same_diagnostics_as_case_validation() {
    let mut definition = load("Pekeris").into_definition();
    definition.source_geometry = kraken::SourceGeometry::Point;
    let base = Case::from_definition(definition).unwrap();
    let prepared = prepare_modes(&base).unwrap();
    let good_offsets = base.receiver_offsets_m.clone();
    let bad_ranges = [
        vec![],
        vec![-1.0],
        vec![f64::NAN],
        vec![f64::INFINITY],
        vec![2.0, 1.0],
        vec![1.0, 1.0],
        vec![0.0; 100_001],
    ];
    for ranges in bad_ranges {
        assert_eq!(
            prepared.field(&ranges, &good_offsets).unwrap_err(),
            query_case(&base, &ranges, &good_offsets).unwrap_err()
        );
    }
    let bad_offsets = [
        vec![],
        vec![0.0; good_offsets.len() + 1],
        vec![f64::NAN; good_offsets.len()],
        vec![f64::INFINITY; good_offsets.len()],
        vec![-1001.0; good_offsets.len()],
    ];
    for offsets in bad_offsets {
        assert_eq!(
            prepared.field(&[1000.0], &offsets).unwrap_err(),
            query_case(&base, &[1000.0], &offsets).unwrap_err()
        );
    }
    // Failed validation does not mutate or poison the reusable product.
    same_field(
        &prepared
            .field(&base.receiver_ranges_m, &good_offsets)
            .unwrap(),
        &solve(&base).unwrap().field,
    );
}

#[test]
fn query_field_grid_limit_is_not_bypassed() {
    let mut definition = load("Pekeris").into_definition();
    definition.source_depths_m.truncate(1);
    definition.receiver_depths_m = vec![definition.receiver_depths_m[0]; 1001];
    definition.receiver_offsets_m = vec![0.0; 1001];
    definition.receiver_ranges_m = vec![1000.0];
    let base = Case::from_definition(definition).unwrap();
    let prepared = prepare_modes(&base).unwrap();
    let ranges: Vec<_> = (1..=1000).map(f64::from).collect();
    let error = prepared
        .field(&ranges, &base.receiver_offsets_m)
        .unwrap_err();
    assert_eq!(
        error,
        query_case(&base, &ranges, &base.receiver_offsets_m).unwrap_err()
    );
    assert!(
        error
            .diagnostics()
            .iter()
            .any(|d| d.code == "KR0201" && d.field == "field_grid")
    );
}

#[test]
fn complex_modes_are_explicitly_rejected() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let case = legacy::load_complex_case(root.join("PekerisComplex.env"), root.join("Pekeris.flp"))
        .unwrap();
    let error = prepare_modes(&case).unwrap_err();
    assert!(
        error
            .diagnostics()
            .iter()
            .any(|d| d.code == "KR0201" && d.field == "mode_solver")
    );
}

#[test]
fn cancellation_does_not_leave_a_partial_or_poisoned_preparation() {
    let base = load("Pekeris");
    let cancelled = || Arc::new(AtomicBool::new(true));
    let error = kraken::cancellation::with_token(cancelled(), || prepare_modes(&base)).unwrap_err();
    assert_eq!(error.diagnostics()[0].code, "KR0304");
    let prepared = prepare_modes(&base).unwrap();
    let error = kraken::cancellation::with_token(cancelled(), || {
        prepared.field(&base.receiver_ranges_m, &base.receiver_offsets_m)
    })
    .unwrap_err();
    assert_eq!(error.diagnostics()[0].code, "KR0304");
    same_field(
        &prepared
            .field(&base.receiver_ranges_m, &base.receiver_offsets_m)
            .unwrap(),
        &solve(&base).unwrap().field,
    );
}

#[test]
fn prepared_modes_are_owned_and_can_be_shared_by_independent_queries() {
    fn assert_owned_and_shareable<T: Send + Sync + 'static>() {}
    assert_owned_and_shareable::<kraken::PreparedModes>();

    let base = load("Pekeris");
    let prepared = Arc::new(prepare_modes(&base).unwrap());
    let expected = solve(&base).unwrap();
    // Compiles only because the prepared value keeps no borrow of its input.
    drop(base);
    std::thread::scope(|scope| {
        let handles: Vec<_> = (0..4)
            .map(|_| {
                let prepared = Arc::clone(&prepared);
                scope.spawn(move || {
                    threads(2, || {
                        prepared
                            .field(
                                &prepared.case().receiver_ranges_m,
                                &prepared.case().receiver_offsets_m,
                            )
                            .unwrap()
                    })
                })
            })
            .collect();
        for handle in handles {
            same_field(&handle.join().unwrap(), &expected.field);
        }
    });
    same_modes(prepared.modes(), &expected.modes);
}

#[test]
fn query_modal_work_limit_is_not_bypassed() {
    let mut definition = load("Pekeris").into_definition();
    definition.frequency_hz = 10_000.0;
    definition.mesh_reference_frequency_hz = None;
    // At 10 kHz the mesh policy requires at least 6666 intervals.
    definition.mesh_points = 8192;
    definition.max_range_m = 0.0;
    definition.source_depths_m.truncate(1);
    definition.receiver_depths_m = vec![definition.receiver_depths_m[0]; 10];
    definition.receiver_offsets_m = vec![0.0; 10];
    definition.receiver_ranges_m = vec![1000.0];
    let base = Case::from_definition(definition).unwrap();
    let prepared = prepare_modes(&base).unwrap();
    assert!(prepared.modes().modes.len() > 550);
    let ranges: Vec<_> = (1..=100_000).map(f64::from).collect();
    let query = query_case(&base, &ranges, &base.receiver_offsets_m).unwrap();
    let error = prepared
        .field(&ranges, &base.receiver_offsets_m)
        .unwrap_err();
    assert_eq!(error, solve(&query).unwrap_err());
    assert_eq!(error.diagnostics()[0].code, "KR0302");
    assert_eq!(error.diagnostics()[0].field, "field_grid");
}

#[test]
fn frozen_field_options_and_signed_zero_metadata_match_full_solve() {
    for geometry in [
        kraken::SourceGeometry::Line,
        kraken::SourceGeometry::Point,
        kraken::SourceGeometry::ScaledCylindrical,
    ] {
        for addition in [
            kraken::ModeAddition::Coherent,
            kraken::ModeAddition::Incoherent,
        ] {
            let mut definition = load("Pekeris").into_definition();
            definition.source_geometry = geometry;
            definition.mode_addition = addition;
            let base = Case::from_definition(definition).unwrap();
            let prepared = prepare_modes(&base).unwrap();
            let ranges = [
                -0.0, 500.0, 750.0, 1000.0, 1500.0, 1750.0, 2000.0, 2500.0, 3000.0,
            ];
            let offsets = vec![40.0; base.receiver_depths_m.len()];
            let query = query_case(&base, &ranges, &offsets).unwrap();
            let expected = solve(&query).unwrap();
            let actual = threads(4, || prepared.field(&ranges, &offsets).unwrap());
            same_field(&actual, &expected.field);
            same_modes(prepared.modes(), &expected.modes);
            assert_eq!(actual.receiver_ranges_m[0].to_bits(), (-0.0_f64).to_bits());
            assert_eq!(prepared.case().source_geometry, geometry);
            assert_eq!(prepared.case().mode_addition, addition);
        }
    }
}

#[test]
fn frozen_source_pattern_matches_full_solve_and_scales_only_the_first_source() {
    let omnidirectional = load("MunkLossless");
    assert_eq!(omnidirectional.source_depths_m.len(), 2);
    let mut definition = omnidirectional.clone().into_definition();
    definition.source_pattern = [(0.0, 1.0), (15.0, 0.25), (90.0, 0.1)]
        .map(|(angle_degrees, amplitude)| SourcePatternPoint {
            angle_degrees,
            amplitude,
        })
        .to_vec();
    let base = Case::from_definition(definition).unwrap();
    let prepared = prepare_modes(&base).unwrap();
    let prepared_without_pattern = prepare_modes(&omnidirectional).unwrap();
    let offsets = vec![40.0; base.receiver_depths_m.len()];
    // One range takes the scalar path; seventeen take the 8-lane path with a tail.
    for range_count in [1, 17] {
        let ranges: Vec<_> = (0..range_count)
            .map(|i| 1000.0 + f64::from(i) * 123.0)
            .collect();
        let query = query_case(&base, &ranges, &offsets).unwrap();
        let expected = threads(1, || solve(&query).unwrap());
        let actual = threads(4, || prepared.field(&ranges, &offsets).unwrap());
        same_field(&actual, &expected.field);
        same_modes(prepared.modes(), &expected.modes);

        // Parity alone would also pass if both routes ignored the pattern.
        let first_source = base.receiver_depths_m.len() * ranges.len();
        let scaled = pressure_bits(&actual);
        let unscaled = pressure_bits(&prepared_without_pattern.field(&ranges, &offsets).unwrap());
        assert_ne!(scaled[..first_source], unscaled[..first_source]);
        assert_eq!(scaled[first_source..], unscaled[first_source..]);
    }
}

#[test]
fn frozen_mode_limit_caps_the_field_sum_but_keeps_all_prepared_modes() {
    let all_modes = load("MunkLossless");
    let mut definition = all_modes.clone().into_definition();
    definition.mode_limit = 5;
    let base = Case::from_definition(definition).unwrap();
    let prepared = prepare_modes(&base).unwrap();
    assert!(prepared.modes().modes.len() > base.mode_limit);
    let ranges = [1000.0, 2000.0, 4000.0];
    let query = query_case(&base, &ranges, &base.receiver_offsets_m).unwrap();
    let expected = threads(1, || solve(&query).unwrap());
    let actual = threads(4, || {
        prepared.field(&ranges, &base.receiver_offsets_m).unwrap()
    });
    same_field(&actual, &expected.field);
    same_modes(prepared.modes(), &expected.modes);

    // Parity alone would also pass if both routes ignored the limit.
    let unlimited = prepare_modes(&all_modes)
        .unwrap()
        .field(&ranges, &base.receiver_offsets_m)
        .unwrap();
    assert_ne!(pressure_bits(&actual), pressure_bits(&unlimited));
}

#[test]
fn large_query_crosses_phase_table_blocks_without_changing_bits() {
    let base = load("MunkLossless");
    let prepared = prepare_modes(&base).unwrap();
    let ranges: Vec<_> = (1..=20_000).map(|i| f64::from(i) * 10.0).collect();
    // Keep this fixture large enough to exercise more than one phase-table block.
    assert!(ranges.len() * prepared.modes().modes.len().min(base.mode_limit) > 1 << 20);
    let query = query_case(&base, &ranges, &base.receiver_offsets_m).unwrap();
    let expected = threads(1, || solve(&query).unwrap());
    let actual = threads(4, || {
        prepared.field(&ranges, &base.receiver_offsets_m).unwrap()
    });
    same_field(&actual, &expected.field);
    same_modes(prepared.modes(), &expected.modes);
}

//! Characterization of receiver-depth pairing under irregular receiver grids
//! (finding P1 of the 2026-10 architecture review).
//!
//! Two influence styles exist in `solver/influence.rs`:
//!
//! - paired (`geo_hat_cartesian`, `geo_gaussian_cartesian`): for irregular
//!   grids the receiver depth is `depths[receiver_index]`;
//! - leading (`cerveny_cartesian`, `cerveny_ray_centered`, `simple_gaussian`,
//!   `geo_hat_ray_centered`): irregular grids iterate `depths.iter().take(1)`,
//!   i.e. every receiver is influenced as if it sat at `depths[0]`.
//!
//! Each family below runs three coherent-field cases with the same receiver
//! ranges (500 m, 2500 m) but different depth vectors:
//!
//! - P:  `[20, 90]` — receiver 1 is labelled 90 m;
//! - D1: `[20, 20]` — receiver 1 is at (and labelled) 20 m;
//! - D2: `[90, 90]` — receiver 1 is at (and labelled) 90 m.
//!
//! The depths are deliberately asymmetric about the 50 m source depth:
//! Cerveny image terms make mirrored receiver depths (20/80) produce
//! identical coherent sums, which would make D1 and D2 indistinguishable.
//!
//! Ray tracing is independent of receiver depths, so receiver 1's pressure is
//! bit-identical to D1 exactly when the family evaluates the leading depth,
//! and bit-identical to D2 exactly when it evaluates the paired depth. The
//! asserts pin v2023.5 reference compatibility, not a Rust porting defect.
//! In the pinned Fortran source, `bellhop.f90:202` sets `NRz_per_range=1`;
//! `influence.f90:64,252,322,693` use `Rz(iz)` (the leading depth), while
//! `influence.f90:458,578` explicitly use the paired `Rz(ir)`.
//! The ignored test below independently checks all six reference families.
//!
//! The environment is `calibB.env` (isovelocity 1500 m/s water over an
//! acoustic half-space) with the receivers and run options replaced.

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use bellhop::legacy::load_case;
use bellhop::result::FieldSample;
use bellhop::solver::{SimulationLimits, run};

static NEXT_DIRECTORY: AtomicUsize = AtomicUsize::new(0);

fn temporary_directory() -> PathBuf {
    let sequence = NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "bellhop-irregular-pairing-{}-{sequence}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&path);
    fs::create_dir_all(&path).unwrap();
    path
}

fn environment(beam_family: char, receiver_depths: &str) -> String {
    // Cerveny families need the extra epsilon and image records.
    let cerveny = if matches!(beam_family, 'C' | 'R') {
        "'MS' 2.0 1.0, 0
1 50 'P'
"
    } else {
        ""
    };
    format!(
        "'Irregular paired-receiver characterization'
250.0
1
'CVW'
101 0.0 100.0
0.0 1500.0 /
100.0 1500.0 /
'A' 0.0
100.0 1590.0 0.0 1.2 0.5 0.0 /
1
50.0 /
2
{receiver_depths} /
2
0.5 2.5 /
'C{beam_family} RI2 '
361
-89.0 89.0 /
0.0 101.0 5.1
{cerveny}"
    )
}

fn receiver_one_pressure(beam_family: char, receiver_depths: &str) -> FieldSample {
    let directory = temporary_directory();
    let path = directory.join("pairing.env");
    fs::write(&path, environment(beam_family, receiver_depths)).unwrap();
    let case = load_case(&path).unwrap().value;
    let result = run(&case, SimulationLimits::default()).unwrap();
    let _ = fs::remove_dir_all(&directory);
    assert_eq!(
        case.environment.run.receiver_grid,
        bellhop::model::ReceiverGrid::Irregular
    );
    result.field_sources[0].samples[1]
}

/// Receiver 1's sample in the mixed case (`[20, 90]`).
fn paired(beam_family: char) -> FieldSample {
    receiver_one_pressure(beam_family, "20.0 90.0")
}

/// Receiver 1's sample in the both-at-`depth` case.
fn uniform(beam_family: char, depth: f64) -> FieldSample {
    receiver_one_pressure(beam_family, &format!("{depth:.1} {depth:.1}"))
}

/// Receiver 1's labeled depth is always the paired one (`mod.rs` builds
/// result coordinates through the same paired mapping for every family).
#[allow(clippy::float_cmp)]
fn assert_baseline(leading: FieldSample, paired_depth: FieldSample) {
    assert_ne!(
        leading.pressure, paired_depth.pressure,
        "leading and paired baselines must differ for a meaningful comparison"
    );
    assert_eq!(leading.depth_m, 20.0);
}

#[test]
#[allow(clippy::float_cmp)]
fn geometric_hat_cartesian_pairs_receiver_depths() {
    // Control family: the paired depth (90 m) decides the pressure.
    let mixed = paired('G');
    let leading = uniform('G', 20.0);
    let paired_depth = uniform('G', 90.0);
    assert_baseline(leading, paired_depth);
    assert_eq!(mixed.depth_m, 90.0);
    assert_eq!(mixed.pressure, paired_depth.pressure);
    assert_ne!(mixed.pressure, leading.pressure);
}

#[test]
#[allow(clippy::float_cmp)]
fn geometric_gaussian_cartesian_pairs_receiver_depths() {
    // Control family, see the `G` case above.
    let mixed = paired('B');
    let leading = uniform('B', 20.0);
    let paired_depth = uniform('B', 90.0);
    assert_baseline(leading, paired_depth);
    assert_eq!(mixed.depth_m, 90.0);
    assert_eq!(mixed.pressure, paired_depth.pressure);
}

#[test]
#[allow(clippy::float_cmp)]
fn geometric_hat_ray_centered_uses_leading_depth_for_irregular_grids() {
    // Reference-compatible limitation: receiver 1 is influenced at depths[0]
    // (20 m), not at its labelled 90 m. Do not silently change this to pairing.
    let mixed = paired('g');
    let leading = uniform('g', 20.0);
    let paired_depth = uniform('g', 90.0);
    assert_baseline(leading, paired_depth);
    assert_eq!(mixed.depth_m, 90.0);
    assert_eq!(mixed.pressure, leading.pressure);
    assert_ne!(mixed.pressure, paired_depth.pressure);
}

#[test]
#[allow(clippy::float_cmp)]
fn simple_gaussian_uses_leading_depth_for_irregular_grids() {
    // Reference-compatible leading depth, see the `g` case above.
    let mixed = paired('S');
    let leading = uniform('S', 20.0);
    let paired_depth = uniform('S', 90.0);
    assert_baseline(leading, paired_depth);
    assert_eq!(mixed.depth_m, 90.0);
    assert_eq!(mixed.pressure, leading.pressure);
    assert_ne!(mixed.pressure, paired_depth.pressure);
}

#[test]
#[allow(clippy::float_cmp)]
fn cerveny_cartesian_uses_leading_depth_for_irregular_grids() {
    // Reference-compatible leading depth, see the `g` case above.
    let mixed = paired('C');
    let leading = uniform('C', 20.0);
    let paired_depth = uniform('C', 90.0);
    assert_baseline(leading, paired_depth);
    assert_eq!(mixed.depth_m, 90.0);
    assert_eq!(mixed.pressure, leading.pressure);
    assert_ne!(mixed.pressure, paired_depth.pressure);
}

#[test]
#[ignore = "requires the pinned Fortran Docker image"]
fn irregular_pairing_matches_pinned_reference() {
    let retained = std::env::var_os("PELAGIC_REFERENCE_DIR");
    let directory = retained.as_ref().map_or_else(temporary_directory, |root| {
        let path = PathBuf::from(root).join("irregular-pairing");
        fs::create_dir(&path).unwrap();
        path
    });
    let runner =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tools/reference/run-case.sh");
    for family in ['G', 'B', 'g', 'S', 'C', 'R'] {
        let mut pressures = Vec::new();
        for (name, depths) in [("p", "20.0 90.0"), ("d1", "20.0 20.0"), ("d2", "90.0 90.0")] {
            // G/g must not alias on case-insensitive host filesystems.
            let stem = format!("beam-{}-{name}", u32::from(family));
            let path = directory.join(format!("{stem}.env"));
            fs::write(&path, environment(family, depths)).unwrap();
            let output = std::process::Command::new(&runner)
                .arg(&path)
                .arg(&directory)
                .output()
                .unwrap();
            assert!(output.status.success(), "reference {stem}: {output:?}");

            // Fixture-specific SHD readback: one source, two paired receivers,
            // two metadata depths but only one pressure row for an irregular grid.
            let bytes = fs::read(directory.join(format!("{stem}.shd"))).unwrap();
            let integer =
                |offset| i32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap());
            let record_bytes = 4 * usize::try_from(integer(0)).unwrap();
            assert!(record_bytes >= 164);
            assert_eq!(bytes.len(), 11 * record_bytes);
            assert_eq!(&bytes[record_bytes..record_bytes + 10], b"irregular ");
            let counts: Vec<_> = (0..7).map(|i| integer(2 * record_bytes + 4 * i)).collect();
            assert_eq!(counts, [1, 1, 1, 1, 1, 2, 2]);
            let pressure: [u8; 8] = bytes[10 * record_bytes + 8..10 * record_bytes + 16]
                .try_into()
                .unwrap();
            pressures.push(pressure);

            let case = load_case(&path).unwrap().value;
            let result = run(&case, SimulationLimits::default()).unwrap();
            let actual = result.field_sources[0].samples[1].pressure;
            for (component, expected) in [actual.re, actual.im]
                .into_iter()
                .zip(pressure.as_chunks::<4>().0)
            {
                let expected = f32::from_le_bytes(*expected);
                assert!(
                    (f64::from(component) - f64::from(expected)).abs() <= 5.0e-8,
                    "{stem}: Rust {component}, reference {expected}"
                );
            }
        }
        assert_ne!(
            pressures[1], pressures[2],
            "{family}: indistinguishable baselines"
        );
        let baseline = if matches!(family, 'G' | 'B') { 2 } else { 1 };
        assert_eq!(
            pressures[0], pressures[baseline],
            "{family}: reference pairing"
        );
        eprintln!(
            "{family}: reference uses {} depth; Rust agrees within 5e-8",
            if baseline == 2 { "paired" } else { "leading" }
        );
    }
    if retained.is_none() {
        fs::remove_dir_all(directory).unwrap();
    }
}

#[test]
#[allow(clippy::float_cmp)]
fn cerveny_ray_centered_uses_leading_depth_for_irregular_grids() {
    // Reference-compatible leading depth, see the `g` case above.
    let mixed = paired('R');
    let leading = uniform('R', 20.0);
    let paired_depth = uniform('R', 90.0);
    assert_baseline(leading, paired_depth);
    assert_eq!(mixed.depth_m, 90.0);
    assert_eq!(mixed.pressure, leading.pressure);
    assert_ne!(mixed.pressure, paired_depth.pressure);
}

# Testing layers

## Daily development and PRs

Use the exact Rust version in `rust-toolchain.toml` (1.99.0), with the locked
dependencies. This does not raise the declared Rust 1.88 MSRV. Deliberate
toolchain upgrades require full numerical acceptance, not only ordinary tests.
The critical-ray Linux container intentionally retains its separate fixed 1.88
image/digest.

```sh
cargo fmt --all -- --check
cargo clippy --locked --workspace --all-targets --all-features -- -D warnings
cargo test --locked --workspace --all-features
python3 tools/reference/verify-goldens.py
python3 tools/reference/test-tools.py
RUSTDOCFLAGS='-D warnings' cargo doc --locked --workspace --all-features --no-deps
```

These tests run real BELLHOP, KRAKEN and KRAKENC against committed Fortran
references, without Docker or solver mocks. Only the two solver packages use
`opt-level=2` in the test profile; debug assertions and overflow checks remain
on. Adapters/dependencies retain the ordinary test profile. The 500 Hz / 1,023
mode work-budget regression stays in the daily suite.

Linux CI retains format, Clippy, all-feature workspace tests, reference hashes
and documentation. Windows retains its existing release-profile serial tests
(native HDF5 handle inheritance), DLL checks, actual extracted ZIP executions
and independent output readback. The Windows workflow name/build profile does
not mean a PR publishes a release. Both install the repository-pinned toolchain.
PRs run once: branch pushes no longer duplicate PR jobs; pushes to `main` and
manual workflow dispatch still run. Superseded runs are cancelled, but published
release runs are not cancelled by this policy.

Targets are about 30 seconds for dependency-ready local regression after an edit
and 3 minutes PR wall time; they are measurement goals, not timeouts or excuses
to disable checks. Cold compilation/downloads are separate. Host timings do not
prove Linux/Windows hosted-runner timing.

### Local measurements for this change

Measured on Apple arm64 / Darwin 25.6.0 with Rust 1.99.0, using
`cargo test --locked --workspace --all-features` and `/usr/bin/time -p`.
The baseline is an unchanged archive of `2128f10`; both versions had dependencies
and compilation caches prepared before three consecutive warm measurements.

| Run | Wall time |
|---|---:|
| Baseline warm (three runs) | 42.22 / 41.69 / 42.70 s |
| Updated warm (three runs) | 7.21 / 7.21 / 7.42 s |
| Updated after invalidating `kraken/src/lib.rs` with `touch` | 25.43 s (2.76 s Cargo build) |

Warm medians are 42.22 → 7.21 s (83% less wall time). The post-edit probe
invalidates a solver source fingerprint without changing its contents; it is
not a claim about every possible edit. Ordinary tests changed from 232 passed /
12 ignored to 231 passed / 12 ignored, with numerical coverage added inside
existing tests and redundant count-only tests removed.

The local full fresh-reference diagnostic passed all 25 stages in 568 s. It ran
on an uncommitted worktree and correctly exited 2, so it is **not release
acceptance**. The named repository toolchain was also installed in an isolated
`RUSTUP_HOME` and passed fmt, Clippy, ordinary tests and documentation checks.
Windows ZIP execution and the 3-minute hosted PR target still require an actual
CI run.

## Coverage responsibilities

Core solver ENV/golden matrices remain intact. CLI representatives check input
resources and serialized structure rather than repeating the whole physical
matrix. Every retained round-trip still checks native readers, HDF5 shape/type/
unit metadata and all stored Rust values exactly.

| CLI positive group | Before | Now | Representatives / retained branches |
|---|---:|---:|---|
| Single/multiple frequency | 22 | 7 | PekerisFiltered, ComplexBlank, MunkLeakyPchipBroadband, FieldScaled, FieldIncoherent, PekerisBroadband + duplicate frequencies |
| Water material | 27 | 2 | WaterLossPower, both solvers |
| Smooth boundaries | 24 | 7 | VR/RA/AV through both solvers (V/R/A at each end), Air through KRAKENC |
| Layered fluids | 45 | 2 | LayeredFluidPower, both solvers |
| Finite elastic | 72 | 12 | BottomN/TopN, Stack/Power, GradedStack/Power, both solvers |
| Elastic half-spaces | 37 | 4 | BothS/TopBroadband, both solvers |
| **Total in these six groups** | **227** | **34** | 193 fewer CLI+API duplicate pairs |

The existing resource-protection test still round-trips FluidTrcC, TabRefBrcC,
TabRefIrcC and FieldPattern (TRC/BRC/IRC/SBP); the separate ProfilesAd/Cm test
retains profile metadata, modes, pressures and late-failure protection. JSON
relocation/self-containment, independent frequency geometry/repetitions,
input/alias/scratch/rollback protection are not removed.

Removed count-only solves are superseded by `kraken/tests/differential_reference.rs`:

- `elastic.rs::real_elastic_top_and_both_boundaries_keep_complete_pinned_spectra`:
  eight Top/Both N/C/P/S cases → full elastic-half-space mode/FIELD goldens.
  Single-frequency `solve` and `solve_frequencies` both start fresh search state.
- `finite_elastic.rs::real_multifluid_finite_elastic_cases_keep_complete_pinned_spectra`:
  TopN, ShearOnly and ordered Power → full finite-elastic goldens using the same
  sequential frequency API. Independent reset/order/failure-stop checks remain.
- `layered.rs` LayeredFluidN's 63-pressure count → full layered-fluid goldens;
  its distinct input layer/depth assertions remain.
- Simple-Gaussian eigenray counts → all three trajectories / 314 coordinates
  from fixed Linux Fortran, sharing the existing eigenray reader.
- Nonnegative/real-only adiabatic incoherent pressure check → ProfilesAdIncoherent
  full fixed-Fortran comparison, with an all-zero corruption control.

New evidence also covers coherent/incoherent BELLHOP line scaling, Thorp and
overlapping biological loss (33 pressure samples each), and beam-shift trajectory
coordinates. Option-removal controls prove these paths affect outputs. All new
references are from triplicate unmodified pinned Fortran runs, never Rust output.
This is representative coverage, not a claim that every option combination is
accepted.

GradedElasticStack/Power ordinary CLI checks now compare all six material-profile
columns against canonical legacy inputs, including dimensions, f64 types, units,
point counts and empty-profile absence. A last-layer/last-node corruption control
checks each of the six columns independently. Both solver material semantics and
per-frequency loss conversion remain exercised.

Reference integrity now covers all 739 previous KRAKEN numerical artifacts
(instead of 480), plus the three new incoherent artifacts. `readme.sha256`
transcribes existing hashes rather than regenerating baselines. The verifier
also rejects an unmanifested numerical artifact, and validates the BELLHOP
README records. Tooling checks reject damaged hashes, missing records, zero/
multiple selected tests, nonzero process exits and false-green failure reports.

Independent Pekeris analytic evidence, scaling/group-velocity metamorphic tests,
cutoffs, FIELD cap independence, mesh 5→4 / 43→42 refinement relations, and
phase-aligned-but-not-amplitude-rescaled comparator corruption tests remain.

## Before every release

Run [`tools/reference/full-acceptance.sh`](../../tools/reference/full-acceptance.sh)
on the release commit. See [the full workflow and report contract](reference-workflows.md#full-pre-release-acceptance).
This retains the old fresh-reference matrix, adds all 18 irregular-pairing
comparisons, and uses unique output directories. A failed prefix or a successful
focused retry cannot constitute a complete pass. Match the clean report SHA and
Windows ZIP CI result manually; no release/push is performed by the local entry.

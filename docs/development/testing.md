# Testing policy

## Current numerical-validation scope

Full numerical validation uses the existing macOS host and pinned Linux-container
workflows. Record the actual host/container architecture and toolchain. Reference
validity, numerical criteria and clean-commit requirements are defined separately
from whether a process finishes successfully.

| Layer | Purpose | Entry point |
|---|---|---|
| Daily / PR | Real solvers, committed goldens, adapters and failure protection | Commands below; `.github/workflows/ci.yml` |
| Configured full acceptance | Fresh pinned references and complete configured API/CLI workflows | `tools/reference/full-acceptance.sh` |
| Broad business diagnostic | Inventory original workflows and additional combinations, retaining exceptions | `bellhop-business.py` / `kraken-business.py` |
| Focused diagnosis | Reproduce one failure or independent control | [Reference workflows](reference-workflows.md) |

A broad diagnostic or focused retry does not replace full acceptance. Conversely,
a successful configured run does not establish support for every path in the
broader inventory. Current results belong in the
[BELLHOP](bellhop-business-coverage.md) and [KRAKEN](kraken-business-coverage.md)
coverage records, not in this policy's test counts.

## Daily development and PRs

Use the exact Rust version in `rust-toolchain.toml`, with locked dependencies.
This does not raise the declared Rust 1.88 MSRV. Deliberate toolchain upgrades
require full numerical acceptance. The critical-ray Linux comparison retains
its separate pinned Rust 1.88 environment.

```sh
cargo fmt --all -- --check
cargo clippy --locked --workspace --all-targets --all-features -- -D warnings
cargo test --locked --workspace --all-features
python3 tools/reference/verify-goldens.py
python3 tools/reference/test-tools.py
python3 tools/check-docs.py
RUSTDOCFLAGS='-D warnings' cargo doc --locked --workspace --all-features --no-deps
```

These tests run real BELLHOP, KRAKEN and KRAKENC against committed Fortran
references, without Docker or solver mocks. Only the two solver packages use
`opt-level=2` in the test profile; debug assertions and overflow checks remain
on. The 500 Hz / 1,023-mode work-budget regression stays in the daily suite.
Ignored external-reference tests require their explicit reference workflows;
a successful ordinary suite does not imply they ran.

Linux CI runs the checks above. PRs run once rather than also running duplicate
branch-push jobs; pushes to `main` and manual dispatch remain. Superseded CI runs
are cancelled, but published-release runs are not cancelled by that policy.

Dependency-ready local regression and PR wall-time goals are about 30 seconds
and 3 minutes. They are measurement goals, not permission to skip checks; cold
builds/downloads are separate. Earlier optimization timings and before/after
counts are [historical measurements](https://github.com/leihuang1/pelagic/blob/95b8969/docs/development/testing.md#local-measurements-for-this-change),
not current suite-size or hosted-runner guarantees.

## Coverage responsibilities

- **Solver comparisons:** every declared mode, sampled shape and pressure over
  all sources/frequencies/profiles, with [fixed comparison rules](reference.md#comparison-rules).
  Phase alignment must not hide amplitude/normalization errors.
- **Independent numerical controls:** analytic Pekeris spectra and normalized
  fields, contour/Sturm counts, scaling/group-speed relations, separated ducts,
  roughness cancellation/height scaling, refinement and spectral cutoffs.
  These support their tested cases, not arbitrary complex-root completeness.
- **CLI serialization:** actual native/HDF5/both products, schema identity,
  dimensions, types, units, all stored values and independent native readback.
  Representatives cover output branches; targeted regressions also retain the
  repaired lossy Pekeris and complex multi-profile paths.
- **JSON:** complete legacy-definition equality, relocated self-contained
  documents, resource ownership and numerical/native/HDF5 round-trips.
- **Failure safety:** later-frequency/profile failure, resource/alias protection,
  scratch ownership, rollback and preservation of existing outputs. Resource
  rejection is protection evidence, not a numerical pass.
- **Reference integrity:** hashes and manifests cover every committed numerical
  artifact. Corruption controls exercise coordinates, records, loss, shapes,
  pressure and material metadata. Tool guards reject empty/ambiguous test
  selections, failed processes and false-green acceptance reports.

Detailed fixtures and derivations stay in the golden ledgers and
[KRAKEN checkpoint history](../history/kraken-checkpoints.md); the executable full matrix
stays in the acceptance script. Do not duplicate their changing case lists here.

## Language adapter checks

Build the extension in a virtual environment with `maturin develop --release`,
build the CLI with `cargo build --release -p cli`, then run:

```sh
PELAGIC_CLI="$PWD/target/release/pelagic" python -m pytest python/tests -q
```

With MATLAB installed locally (it need not be on `PATH`):

```sh
PELAGIC_CLI="$PWD/target/release/pelagic" matlab -batch \
  "addpath('matlab/tests'); test_bindings; test_curated; test_examples"
```

`python/tests/test_examples.py` and `matlab/tests/test_examples.m` keep
`examples/` working. They check that every case's JSON equals its Toolbox
files, run every case and every script, and run the command-line scripts
(bash, or PowerShell on Windows) when `PELAGIC_CLI` is set. After changing a
case's Toolbox files, regenerate its JSON with `python3 tools/export-examples.py`.

Install the `test` extra for h5py, notebook and optional conversion checks. The
`arlpy` extra enables its separate bridge test (otherwise explicitly skipped).
The bindings workflow builds abi3 wheels on the five requested OS/architecture
combinations, exercises Python 3.9/NumPy 1.26 and current Python/NumPy, runs the
notebook, and runs `matlab/tests/test_bindings.m` in MATLAB and Octave. MATLAB
packages bundle the checksum-verified CLI from the same build matrix. Workflow
definitions are not evidence of successful remote runs; record actual results.

## Before every release

Run full numerical acceptance on the merged release commit, then follow the
rest of the [release checklist](releasing.md). The acceptance contract is in
[reference workflows](reference-workflows.md#full-pre-release-acceptance). A
successful prefix, a focused retry or an `--allow-dirty` run is not acceptance.

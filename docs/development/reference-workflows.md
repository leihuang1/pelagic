# Reference reproduction workflows

Run commands from the repository root with the toolchain in `rust-toolchain.toml`.
See [reference identity and comparison rules](reference.md) and
[testing responsibilities](testing.md). This page documents entry points, not a
second copy of the executable acceptance matrix.

## Full pre-release acceptance

From a clean release commit, run before every release and deliberate Rust upgrade:

```sh
tools/reference/full-acceptance.sh
```

The script runs the full configured sequence: ordinary checks, fresh KRAKEN/
KRAKENC modes and FIELD, roughness, separated ducts, lossy rigid-top and complex
multi-profile regressions, material/elastic/table cases, broadband, ENV/JSON,
actual CLI native/HDF5 products and BELLHOP regressions. Its
[case lists](../../tools/reference/full-acceptance.sh) are authoritative, including
original inputs, labelled derivatives, modal-only cases and exclusions.

Each invocation writes `target/acceptance/<UTC>-<pid>/report.md`, `run.log` and
`reference/` without overwriting earlier runs. The report records the commit,
platform/toolchain, stage results and explicit NOT RUN stages. The traced log
records every invoked case, engine and test. Failure stops the run and retains
the completed prefix and failure evidence; a focused retry cannot complete it.

The script clears external comparator overrides, uses the pinned reference image
and enables release debug assertions/overflow checks. Rust runs on the host
except for the separate fixed-Linux critical-ray checks. Named reference tests
must execute exactly once; empty or ambiguous selections fail.

`--allow-dirty` is diagnostic only: even successful numerical stages exit 2 and
report **NOT RELEASE ACCEPTANCE**. For a release, match a successful report's
**clean, unchanged SHA** to the release commit. The script does not push or
publish. Daily PR CI remains the Docker-free committed-golden suite.

## Broad KRAKEN business-case diagnostic

This independent inventory exercises official active MATLAB calls, explicit
loops/shared FLPs and repository FIELD fixtures through both engines. Every PASS
requires valid fresh reference modes/FIELD, ENV/JSON API comparison, actual
ENV/JSON CLI `both` products, HDF5 comparison and independent native readback.

```sh
tools/reference/build-image.sh
docker build --platform linux/amd64 -t pelagic-business-linux:local \
  -f tools/reference/Dockerfile.business tools/reference

# A Linux x86-64 toolchain with bin/rustc and bin/cargo, matching rust-toolchain.toml.
LINUX_RUST_TOOLCHAIN=/absolute/path/to/linux-rust-toolchain
docker run --rm --platform linux/amd64 \
  --volume "$PWD:/repo" --volume "$LINUX_RUST_TOOLCHAIN:/opt/rust:ro" \
  --volume pelagic-business-cargo:/usr/local/cargo \
  --env CARGO_HOME=/usr/local/cargo \
  --env CARGO_TARGET_DIR=/repo/target/linux-business-build \
  --workdir /repo pelagic-business-linux:local /bin/bash -c \
  'git config --global --add safe.directory /repo;
   python3 tools/reference/kraken-business.py \
     --output target/kraken-business/new-run'
```

The output directory must be new. Default timeout is 600 seconds per stage.
`--only REGEX` selects a diagnostic subset, not full acceptance. The runner
retains the exact runner/binaries, source/input/resource hashes, reference
warnings, outputs, stage logs and complex/relative/TL/phase metrics. It continues
after case failures and returns nonzero for any non-PASS classification.
Unsupported inputs, failed/nonfinite/unconverged references and missing originals
are not numerical passes. See [current results and exceptions](kraken-business-coverage.md).

## Broad BELLHOP business-case diagnostic

```sh
# Same Linux environment for Rust and the unchanged Fortran oracle.
tools/reference/bellhop-business-linux.sh /absolute/path/to/linux-rust-toolchain \
  --output target/bellhop-business/new-run --timeout 600

# Host Rust against Linux Fortran, a separately identified comparison.
python3 tools/reference/bellhop-business.py

# Focused retry only.
python3 tools/reference/bellhop-business.py --only 'constructed/.*shelf-ssp'
```

The Linux wrapper builds the same business image and mounts the supplied Linux
toolchain; it does not replace the pinned oracle. Runs retain pristine inputs,
canonical JSON, hashes, binary snapshots, products and per-case outcomes in fresh
output directories. Default timeout is 120 seconds; the complete recorded Linux
runs used 600 seconds. Timeouts and resource rejections remain explicit outcomes,
not permission to raise solver quotas. See [coverage and accepted exceptions](bellhop-business-coverage.md).

Both business runners' `--self-test` guards are included in
`python3 tools/reference/test-tools.py`. These inventories remain separate from
the configured release gate; neither silently waives its recorded failures.

## Pinned Fortran differential reference

For focused work, prepare a fresh reference destination and the actual CLI:

```sh
tools/reference/build-image.sh
export PELAGIC_REFERENCE_DIR="$PWD/target/reference/manual-$(date -u +%Y%m%dT%H%M%SZ)-$$"
mkdir -p "$(dirname "$PELAGIC_REFERENCE_DIR")"
mkdir "$PELAGIC_REFERENCE_DIR"
export CARGO_TARGET_DIR="$PWD/target"  # CLI helpers read target/release/pelagic.
cargo build --locked --release -p cli
```

The build helper verifies the pinned source archive hash. Standalone helpers
otherwise default to `target/reference/`; select a fresh directory to retain old
evidence. `BELLHOP_REFERENCE_IMAGE` can select an existing image for focused
work, but its source/compiler/runtime must match the oracle. Full acceptance and
the business diagnostics use their fixed oracle.

### KRAKEN/KRAKENC modes and FIELD

```sh
# Reference only; same-stem FLP triggers FIELD, otherwise this is modes-only.
tools/reference/run-kraken-case.sh krakenc crates/kraken/tests/fixtures/PekerisComplex.env

# Real-engine API against fresh modes/FIELD.
tools/reference/compare-kraken.sh crates/kraken/tests/fixtures/Pekeris.env

# API, actual CLI both products, HDF5 versus reference and native readback.
tools/reference/compare-kraken-hdf5.sh krakenc crates/kraken/tests/fixtures/PekerisRigidLoss.env
```

The last helper uses the single-profile multi-frequency comparator, including
all ordered/repeated frequency blocks. It also serves material, layered-fluid,
roughness and supported elastic/table fixtures. Use the profile entry below
for multi-profile FIELD. Modal-only and original-environment comparators are
selected explicitly in `full-acceptance.sh`.

### Multi-profile FIELD

```sh
name=ProfilesAd
engine=krakenc
env="$PWD/crates/kraken/tests/fixtures/$name.env"
reference="$PELAGIC_REFERENCE_DIR/$name-$engine/$name"
tools/reference/run-kraken-case.sh "$engine" "$env"
KRAKEN_PROFILE_ENV="$env" KRAKEN_PROFILE_ROOT="$reference" KRAKEN_PROFILE_SOLVER="$engine" \
  tools/reference/test-one.sh --release -p kraken --test differential_reference \
    profile_fields_match_fresh_reference -- --ignored --exact --nocapture
tools/reference/run-cli-kraken.sh "$env" "$reference.cli" --solver "$engine"
KRAKEN_HDF5_RESULT="$reference.cli/$name.h5" KRAKEN_PROFILE_ENV="$env" \
KRAKEN_PROFILE_ROOT="$reference" KRAKEN_PROFILE_SOLVER="$engine" \
  tools/reference/test-one.sh --release -p kraken --test differential_reference \
    profile_fields_match_fresh_reference -- --ignored --exact --nocapture
```

`ProfilesCm` exercises coupled propagation. Full acceptance additionally pairs
the original Gulf ENV with its two official FLPs using working filename aliases,
without changing their bytes. See [profile semantics](../kraken/field.md).

### Self-contained JSON and actual products

```sh
name=PekerisRigidLoss
engine=krakenc
env="$PWD/crates/kraken/tests/fixtures/$name.env"
reference="$PELAGIC_REFERENCE_DIR/$name-$engine/$name"
json="$PELAGIC_REFERENCE_DIR/$name.json"
tools/reference/run-kraken-case.sh "$engine" "$env"
target/release/pelagic kraken export "$env" --solver "$engine" > "$json"
KRAKEN_JSON_INPUT="$json" KRAKEN_JSON_ENV="$env" KRAKEN_JSON_REFERENCE_ROOT="$reference" \
  tools/reference/test-one.sh --release -p kraken --test differential_reference \
    json_fields_match_fresh_reference -- --ignored --exact --nocapture
tools/reference/run-cli-kraken.sh "$json" "$reference.json-cli"
KRAKEN_HDF5_RESULT="$reference.json-cli/$name.h5" KRAKEN_JSON_INPUT="$json" \
KRAKEN_JSON_ENV="$env" KRAKEN_JSON_REFERENCE_ROOT="$reference" \
  tools/reference/test-one.sh --release -p kraken --test differential_reference \
    json_fields_match_fresh_reference -- --ignored --exact --nocapture
```

The JSON comparator first checks complete equality with the accepted legacy
case, including solver identity, then compares every declared result. HDF5 and
JSON products are not new Fortran goldens.

### Special reference preparation and independent controls

- `tools/reference/prepare-tabref.sh` rebuilds pinned BOUNCE from source and
  generates the original TabRefCoef resources in three runs. Tables and MOD/SHD
  must be identical. The full script then checks each original workflow against
  its own reference; generated tables are not upstream-original inputs.
- Original MunkS and MunkAnalytic ENV files use separately labelled derived FLPs
  because the shipped three-line FLPs fail in pinned FIELD. The script retains
  this distinction; these are not original-pair or SCOOTER acceptance.
- `cargo test --locked -p kraken --test numerical_regressions` runs committed
  independent roughness, separated-duct and lossy-Pekeris controls without Docker.
  Their bounded evidence does not replace the strict official differentials.
- `probe-layered-refinement.py` is a retained historical diagnostic, not an
  acceptance oracle. Its altered products cannot replace goldens; see the
  [three-layer diagnosis](../history/krakenc-three-layer-refinement.md).

## BELLHOP focused comparisons

```sh
tools/reference/compare-ray.sh path/to/ray-case.env
tools/reference/compare-ray-linux.sh path/to/ray-case.env
tools/reference/compare-arrival.sh path/to/arrival-case.env
tools/reference/compare-field.sh path/to/field-case.env
tools/reference/check-critical-rays.sh
```

The ray comparator checks launch angles, bounce counts and coordinates, with
isolated minimum-step alignment reported separately. The Linux ray helper uses
its pinned Rust 1.88 environment. Arrival comparisons traverse all sources and
receivers; pressure comparisons use the default `5e-8` per-component criterion.
See [comparison rules](reference.md#comparison-rules) for the distinct criteria.

Comparator environment overrides are diagnostic controls, not adjustable
acceptance shortcuts. No command here changes input bounds, quotas or the
[BELLHOP deferred-boundary decision](bellhop-business-coverage.md#operator-acceptance-and-deferred-boundary-work).
Fresh outputs remain local artifacts; curated goldens and their manifests are
verified with `python3 tools/reference/verify-goldens.py`.

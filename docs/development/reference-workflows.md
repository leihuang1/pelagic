# Reference reproduction workflows

## Full pre-release acceptance

From a clean commit, run **before every release** (also on deliberate Rust
upgrades):

```sh
tools/reference/full-acceptance.sh
```

This is the former heavy CI sequence, not a reduced smoke suite. It adds the
18 irregular BELLHOP comparisons and the new small coverage cases. It stops
on the first failure, retains completed outputs and failure work/logs, and
writes `target/acceptance/<UTC>-<pid>/report.md` plus `run.log` and `reference/`.
The report records SHA, platform/toolchain, stage results and explicit NOT RUN
stages; the traced log records each case/engine/test and its result. Focused
tests must run exactly once: renamed/missing filters fail instead of passing
with zero tests. Each invocation has fresh destinations, without overwriting
previous evidence. `--allow-dirty` is diagnostic only and can never report
release acceptance (even an otherwise successful run exits 2).

Manually match the report's **clean, unchanged commit** to the release commit
and that commit's existing Windows test/ZIP verification. A focused retry is
not complete acceptance. No automatic credentials gate or cross-machine
artifact system is involved. The oracle remains Linux amd64 Fortran; Rust runs
on the host except the existing fixed Linux Rust 1.88 critical-ray checks.

Daily Linux/Windows CI uses real Rust solvers with committed references, without
Docker. The historical “fresh CI” sequences described below now belong to this
local full-acceptance entry, not every PR. See [testing layers](testing.md).

## Broad BELLHOP business-case diagnostic

```sh
# Both implementations on Linux, Rust 1.99.0:
tools/reference/bellhop-business-linux.sh /absolute/path/to/linux-rust-toolchain \
  --output target/bellhop-business/my-linux-run --timeout 600

# Host Rust against Linux Fortran (not a same-platform comparison):
python3 tools/reference/bellhop-business.py
# Focused retry; not a complete matrix run:
python3 tools/reference/bellhop-business.py --only 'constructed/.*shelf-ssp'
```

Requires the pinned image and project Rust toolchain. The Linux wrapper requires
a Linux x86-64 toolchain directory containing `bin/rustc` and `bin/cargo`; it builds
`Dockerfile.business` without replacing the pinned Fortran binary or runtime.
Each invocation creates a new directory under `target/bellhop-business/`, retaining inputs, hashes, fresh
Fortran products, canonical JSON, actual CLI products, logs and `report.md` /
`results.json`. It continues after case failures, returns nonzero if any case is
not accepted, and lists unrun cases explicitly. Tolerances and solver quotas are
unchanged; no failure is an expected-pass waiver. See [coverage and measured
limitations](bellhop-business-coverage.md). This diagnostic is separate from the
release acceptance gate above and does not add a heavy PR job. Its lightweight
`--self-test` runs with the ordinary reference tooling guards.

## Pinned Fortran differential reference

`tools/reference/Dockerfile` builds the official Acoustics Toolbox `v2023.5`
BELLHOP, KRAKEN, KRAKENC, BOUNCE, and 2D FIELD programs at commit
`475108519289c6fb488b58980c644ea14eccc604` for Linux x86-64. The Debian base
image, source-archive SHA-256, GNU Fortran version, and compiler flags are
fixed in the image definition.

Build the image:

```sh
tools/reference/build-image.sh
```

The build script downloads the commit archive with retries, verifies
SHA-256 `f8a7a2c1e80a73431cd230a10bef5fcfc996c88889a0e1540771c3922ee2a21f`,
and removes the temporary archive after the image is built.

Run the BELLHOP reference for one case:

```sh
tools/reference/run-case.sh path/to/case.env
```

Run KRAKEN or KRAKENC on a legacy environment. If a same-stem `.flp` exists,
the helper also runs 2D FIELD; it saves the reference `.mod` and `.shd` files
under `target/reference/`. The planned Rust support matrix is in
[the KRAKEN compatibility document](../kraken/compatibility.md):

```sh
tools/reference/run-kraken-case.sh kraken path/to/MunkK.env
tools/reference/run-kraken-case.sh krakenc path/to/MunkKleaky.env
```

Numerically compare a supported Rust fluid `.env`/`.flp` pair
against fresh KRAKEN/FIELD output (not merely a smoke test):

```sh
for case in Pekeris PekerisFiltered PekerisDense PekerisDenseLoss PekerisRefined PekerisSpline3 PekerisRigid PekerisRigidLoss PekerisHard PekerisHardBoth PekerisRigidPlane MunkLossless MunkBottomLoss MunkAnalytic SductTrapped SductPchip SductSpline; do
  tools/reference/compare-kraken.sh "crates/kraken/tests/fixtures/$case.env"
done
```

For a same-stem pair using the multi-frequency comparator, compare the full
Rust API result and then the actual legacy CLI/HDF5 result with one helper:

```sh
cargo build --release -p cli
tools/reference/compare-kraken-hdf5.sh krakenc crates/kraken/tests/fixtures/LayeredFluidPower.env
```

The helper runs fresh Fortran first and writes CLI `both` products into a
separate result directory, then independently reads every native value and
compares HDF5 with the oracle. Full acceptance shares this sequence across the
six material/FIELD groups; their case lists, exclusions, reference checks and
tolerances remain independent.

For the derived KRAKENC Pekeris and reduced 1 km, seven-knot MunkLeakyPartial
cases (`PekerisComplexBlank`, `PekerisComplexRefined`,
`MunkLeakyPartialLoss` and `MunkLeakyPartialC` also have separately derived
coherent FIELD `.flp` files), run the same pinned Fortran calculation
and comparator used by full acceptance:

```sh
for case in PekerisComplex PekerisComplexBlank PekerisComplexSlow PekerisComplexCLow PekerisComplexRefined PekerisComplexGradient PekerisComplexReverseGradient MunkLeakyPartial MunkLeakyPartialLoss MunkLeakyPartialC MunkLeakyPartialP MunkLeakyPartialS PekerisComplexSpline3 MunkAnalyticComplex TabRefBrcN TabRefBrcC TabRefIrcN TabRefIrcC; do
  tools/reference/run-kraken-case.sh krakenc "crates/kraken/tests/fixtures/$case.env"
  KRAKEN_COMPLEX_CASE="$case" \
  KRAKEN_COMPLEX_REFERENCE_ROOT="$PWD/target/reference/$case-krakenc/$case" \
    cargo test --release -p kraken --test differential_reference \
      complex_fluid_matches_fresh_reference -- --ignored --exact --nocapture
done

# Extract the unmodified upstream inputs from the pinned image, not the
# derived fixtures. The Rust comparison validates modes and all FIELD samples.
mkdir -p target/reference/cases
docker run --rm --platform linux/amd64 --volume "$PWD/target/reference/cases:/out" \
  --entrypoint /bin/sh pelagic-reference:v2023.5-amd64 -c '
    for case in MunkKleaky MunkKwb MunkKbb; do
      cp /opt/acoustics-toolbox/tests/MunkLeaky/$case.env /opt/acoustics-toolbox/tests/MunkLeaky/$case.flp /out/
    done
    cp /opt/acoustics-toolbox/tests/sduct/sductK.env /opt/acoustics-toolbox/tests/sduct/sductK.flp /out/
    cp /opt/acoustics-toolbox/tests/calib/calibK.env /opt/acoustics-toolbox/tests/calib/calibK.flp /out/'
for case in MunkKleaky MunkKwb MunkKbb sductK calibK; do
  tools/reference/run-kraken-case.sh krakenc "target/reference/cases/$case.env"
  KRAKEN_ORIGINAL_COMPLEX_ENV="$PWD/target/reference/cases/$case.env" \
  KRAKEN_ORIGINAL_COMPLEX_ROOT="$PWD/target/reference/$case-krakenc/$case" \
    cargo test --release -p kraken --test differential_reference \
      complex_original_fluid_matches_fresh_reference -- --ignored --exact --nocapture
done
```

Reproduce the three fixed single-profile FIELD-extension paths (the helper
copies `.sbp` along with other same-stem resources):

```sh
for mapping in 'FieldScaled kraken' 'FieldPattern krakenc' 'FieldIncoherent krakenc'; do
  set -- $mapping
  tools/reference/run-kraken-case.sh "$2" "crates/kraken/tests/fixtures/$1.env"
  KRAKEN_FREQUENCY_SOLVER="$2" \
  KRAKEN_DIFFERENTIAL_ENV="$PWD/crates/kraken/tests/fixtures/$1.env" \
  KRAKEN_DIFFERENTIAL_ROOT="$PWD/target/reference/$1-$2/$1" \
    cargo test --release -p kraken --test differential_reference \
      multifrequency_fluid_matches_fresh_reference -- --ignored --exact --nocapture
done
```

Reproduce the fixed multi-profile AD/CM paths without modifying original Gulf
inputs. Only working filenames are aliased to pair each original FLP with the
same original ENV. Small goldens remain independent of Docker.

```sh
(cd crates/kraken/tests/fixtures && shasum -a 256 -c golden/multi-profile.sha256)
mkdir -p target/reference/profile-cases
for mapping in 'GulfAd gulf_ad' 'GulfCm gulf_cm'; do
  set -- $mapping
  cp crates/kraken/tests/fixtures/original/Gulf/gulf_rd.env "target/reference/profile-cases/$1.env"
  cp "crates/kraken/tests/fixtures/original/Gulf/$2.flp" "target/reference/profile-cases/$1.flp"
done
cargo build --release -p cli
for case in ProfilesAd ProfilesCm GulfAd GulfCm; do
  case "$case" in
    Gulf*) env="$PWD/target/reference/profile-cases/$case.env" ;;
    *) env="$PWD/crates/kraken/tests/fixtures/$case.env" ;;
  esac
  reference="$PWD/target/reference/$case-kraken/$case"
  tools/reference/run-kraken-case.sh kraken "$env"
  KRAKEN_PROFILE_ENV="$env" KRAKEN_PROFILE_ROOT="$reference" \
    cargo test --release -p kraken --test differential_reference \
      profile_fields_match_fresh_reference -- --ignored --exact --nocapture
  tools/reference/run-cli-kraken.sh "$env" "$reference.cli"
  KRAKEN_HDF5_RESULT="$reference.cli/$case.h5" KRAKEN_PROFILE_ENV="$env" KRAKEN_PROFILE_ROOT="$reference" \
    cargo test --release -p kraken --test differential_reference \
      profile_fields_match_fresh_reference -- --ignored --exact --nocapture
done
```

See [multi-profile semantics/limits](../kraken/field.md) and
[additive HDF5 profile layout](../guide/kraken-hdf5.md).

For [self-contained KRAKEN JSON](../kraken/json.md), first generate
an existing representative's pinned reference, then compare both JSON API
and actual JSON CLI/HDF5 through the same comparator:

```sh
cargo build --release -p cli
env="$PWD/crates/kraken/tests/fixtures/WaterLossPower.env"
reference="$PWD/target/reference/WaterLossPower-kraken/WaterLossPower"
mkdir -p target/reference/json
tools/reference/run-kraken-case.sh kraken "$env"
json="$PWD/target/reference/json/WaterLossPower.json"
target/release/pelagic kraken export "$env" > "$json"
KRAKEN_JSON_INPUT="$json" KRAKEN_JSON_ENV="$env" KRAKEN_JSON_REFERENCE_ROOT="$reference" \
  cargo test --release -p kraken --test differential_reference \
    json_fields_match_fresh_reference -- --ignored --exact --nocapture
tools/reference/run-cli-kraken.sh "$json" "${json%.json}.cli"
KRAKEN_HDF5_RESULT="${json%.json}.cli/WaterLossPower.h5" KRAKEN_JSON_INPUT="$json" \
KRAKEN_JSON_ENV="$env" KRAKEN_JSON_REFERENCE_ROOT="$reference" \
  cargo test --release -p kraken --test differential_reference \
    json_fields_match_fresh_reference -- --ignored --exact --nocapture
```

Full acceptance repeats the fixed
[JSON validation workflow list](../kraken/json.md#validation), including
original BRC/IRC, BroadBand/MunkK and both Gulf paths. JSON/HDF5 are not Fortran
goldens; the numerical tolerances and source provenance remain unchanged.

KRAKENC P/S/fixed-Munk-A interpolation uses the same lossless `Profile`; four
cubic/analytic fixtures above compare full modes and FIELD. `MunkS.env` is an
upstream **SCOOTER environment**, not an original KRAKENC/FIELD pair. It and
original `MunkAnalytic.env` can additionally be run with KRAKENC and a separately
**derived** coherent `.flp`; neither original three-line `.flp` parses in FIELD.
To reproduce direct and actual CLI-HDF5 comparisons without changing either env:

```sh
mkdir -p target/reference/profile-environments
docker run --rm --platform linux/amd64 --volume "$PWD/target/reference/profile-environments:/out" \
  --entrypoint /bin/sh pelagic-reference:v2023.5-amd64 -c \
  'cp /opt/acoustics-toolbox/tests/Munk/MunkS.env /opt/acoustics-toolbox/tests/Munk/MunkAnalytic.env /out/'
cargo build --release -p cli
for case in MunkS MunkAnalytic; do
  env="$PWD/target/reference/profile-environments/$case.env"
  cp crates/kraken/tests/fixtures/MunkAnalytic.flp "${env%.env}.flp"
  root="$PWD/target/reference/profile-environments/$case/$case"
  tools/reference/run-kraken-case.sh krakenc "$env" "$(dirname "$root")"
  KRAKEN_ORIGINAL_COMPLEX_ENV="$env" KRAKEN_ORIGINAL_COMPLEX_ROOT="$root" \
    cargo test --release -p kraken --test differential_reference \
      complex_original_fluid_matches_fresh_reference -- --ignored --exact --nocapture
  tools/reference/run-cli-kraken.sh "$env" "$root.cli" --solver krakenc
  KRAKEN_HDF5_RESULT="$root.cli/$case.h5" \
  KRAKEN_ORIGINAL_COMPLEX_ENV="$env" KRAKEN_ORIGINAL_COMPLEX_ROOT="$root" \
    cargo test --release -p kraken --test differential_reference \
      complex_original_fluid_matches_fresh_reference -- --ignored --exact --nocapture
done
```

Both have 102 modes and 25 pressures. Their `.mod/.shd` and all five new derived
cubic/analytic/broadband workflows were identical in three pinned runs. Hashes,
source and explicit input changes accompany the goldens. This is environment
acceptance with derived FIELD geometry, not original-pair or SCOOTER acceptance.

For original **KRAKENC** TabRefCoef, rebuild the image to include source-built
BOUNCE (the archive's `bounce.exe`/`bounce.o` are deleted before compilation).
The upstream inputs contain no tables; the helper generates resources from
original `neggradB.env` in three independent runs, requires identical tables and
`.mod/.shd`, and retains build information and SHA-256 manifests. This reference
preparation alone is not Rust acceptance. Compare the original unchanged pairs
and actual CLI output with the same strict comparator:

```sh
tools/reference/build-image.sh
tools/reference/prepare-tabref.sh
cargo build --release -p cli
for case in neggradC_geo neggradC_brc neggradC_irc; do
  env="$PWD/target/reference/TabRefCoef/run1/$case.env"
  root="$PWD/target/reference/TabRefCoef/run1/$case"
  KRAKEN_ORIGINAL_COMPLEX_ENV="$env" KRAKEN_ORIGINAL_COMPLEX_ROOT="$root" \
    cargo test --release -p kraken --test differential_reference \
      complex_original_fluid_matches_fresh_reference -- --ignored --exact --nocapture
  tools/reference/run-cli-kraken.sh "$env" "$root.cli" --solver krakenc
  KRAKEN_HDF5_RESULT="$root.cli/$case.h5" \
  KRAKEN_ORIGINAL_COMPLEX_ENV="$env" KRAKEN_ORIGINAL_COMPLEX_ROOT="$root" \
    cargo test --release -p kraken --test differential_reference \
      complex_original_fluid_matches_fresh_reference -- --ignored --exact --nocapture
done
```

All 56/54/42 modes and 50,601 pressures per pair are checked, including original
FIELD endpoints. Generated `.brc/.irc` are not upstream-original resources;
Rust does not implement BOUNCE. Small `TabRefBrcN/C` and `TabRefIrcN/C` fixtures
above have **constructed** tables and derived reduced geometry, not the original
input/resource workflow. Full acceptance also checks their CLI/HDF5 output.

For multi-frequency runs, the same comparator reads every frequency block in
`.mod/.prt/.shd`. The two Pekeris inputs below are **derived**; the extracted
BroadBand/MunkK pair is **unmodified upstream**, distinct from tests/Munk/MunkK:

```sh
for case in PekerisBroadband PekerisComplexBroadband MunkLeakyPchipBroadband; do
  engine=kraken
  if [ "$case" != PekerisBroadband ]; then engine=krakenc; fi
  tools/reference/run-kraken-case.sh "$engine" "crates/kraken/tests/fixtures/$case.env"
  KRAKEN_FREQUENCY_SOLVER="$engine" \
  KRAKEN_DIFFERENTIAL_ENV="$PWD/crates/kraken/tests/fixtures/$case.env" \
  KRAKEN_DIFFERENTIAL_ROOT="$PWD/target/reference/$case-$engine/$case" \
    cargo test --release -p kraken --test differential_reference \
      multifrequency_fluid_matches_fresh_reference -- --ignored --exact --nocapture
done
mkdir -p target/reference/cases/BroadBand
docker run --rm --platform linux/amd64 --volume "$PWD/target/reference/cases/BroadBand:/out" \
  --entrypoint /bin/sh pelagic-reference:v2023.5-amd64 -c \
  'cp /opt/acoustics-toolbox/tests/BroadBand/MunkK.env /opt/acoustics-toolbox/tests/BroadBand/MunkK.flp /out/'
for engine in kraken krakenc; do
  tools/reference/run-kraken-case.sh "$engine" target/reference/cases/BroadBand/MunkK.env "target/reference/BroadBand-MunkK-$engine"
  KRAKEN_FREQUENCY_SOLVER="$engine" \
  KRAKEN_DIFFERENTIAL_ENV="$PWD/target/reference/cases/BroadBand/MunkK.env" \
  KRAKEN_DIFFERENTIAL_ROOT="$PWD/target/reference/BroadBand-MunkK-$engine/MunkK" \
    cargo test --release -p kraken --test differential_reference \
      multifrequency_fluid_matches_fresh_reference -- --ignored --exact --nocapture
done
```

`MunkLeakyPchipBroadband` above is **derived**, with 75/50/62.5/50 Hz in input
order, NG=803, meshes 1/2/4, 45/30/37/30 modes and 36 pressures per block.
The full original BroadBand/MunkK comparison now passes **both engines** at
50/500 Hz (102/1,023 modes and 1,003,002 pressures per engine), including
legacy/JSON CLI-HDF5. KRAKENC's narrow lossless-fluid predictor, bounded by
bottom cp, uses 153M of the unchanged 300M ceiling at 500 Hz. Three unmodified
MOD/SHD runs agree. Full acceptance separately derives a 7500 Hz late failure
after both original blocks succeed to retain atomic-output protection:

```sh
KRAKEN_DIFFERENTIAL_ENV="$PWD/target/reference/cases/BroadBand/MunkK.env" \
  cargo test --release -p cli --test kraken_run \
    original_complex_spline_broadband_is_complete_and_preserves_late_failure -- --ignored --exact --nocapture
```

To validate the actual Rust [CLI/HDF5 product](../guide/kraken-hdf5.md),
set `KRAKEN_HDF5_RESULT` for the same comparator. With the fresh reference above:

```sh
cargo build --release -p cli
tools/reference/run-cli-kraken.sh target/reference/cases/BroadBand/MunkK.env \
  target/reference/BroadBand-MunkK-kraken/MunkK.cli
KRAKEN_HDF5_RESULT="$PWD/target/reference/BroadBand-MunkK-kraken/MunkK.cli/MunkK.h5" \
KRAKEN_DIFFERENTIAL_ENV="$PWD/target/reference/cases/BroadBand/MunkK.env" \
KRAKEN_DIFFERENTIAL_ROOT="$PWD/target/reference/BroadBand-MunkK-kraken/MunkK" \
  cargo test --release -p kraken --test differential_reference \
    multifrequency_fluid_matches_fresh_reference -- --ignored --exact --nocapture
```

Full acceptance additionally runs this serialized-output path for both
broadband Pekeris derivatives, original single-frequency MunkK and all five original KRAKENC
pairs, the four cubic/analytic derivatives, broadband PCHIP Munk and unmodified
MunkS/analytic environments with derived FIELD geometry. The optional reader
checks schema identity, native datatypes, axes and
all values; numerical tolerances are shared with the direct solver path.

Water-material fixtures also use the multi-frequency comparator, even for one
frequency. Thirteen `WaterLoss*` inputs run through both engines; `WaterLossLeaky`
runs through KRAKENC only. For example:

```sh
case=WaterLossPower
engine=krakenc
env="$PWD/crates/kraken/tests/fixtures/$case.env"
reference="$PWD/target/reference/$case-$engine/$case"
tools/reference/run-kraken-case.sh "$engine" "$env"
cargo build --release -p cli
tools/reference/run-cli-kraken.sh "$env" "$reference.cli" --solver "$engine"
KRAKEN_HDF5_RESULT="$reference.cli/$case.h5" KRAKEN_FREQUENCY_SOLVER="$engine" \
KRAKEN_DIFFERENTIAL_ENV="$env" KRAKEN_DIFFERENTIAL_ROOT="$reference" \
  cargo test --release -p kraken --test differential_reference \
    multifrequency_fluid_matches_fresh_reference -- --ignored --exact --nocapture
```

These are derived material/volume-loss cases, not original VolAtt acceptance.
The full-acceptance step checks all 27 workflows through both direct and
serialized paths.

The same command also accepts derived `FluidBoundary{V,R,A}{V,R,A}` and
`FluidRigidPlaneLoss` through either engine; `FluidBoundaryAir` and
`FluidTrcN/C/Rigid` use KRAKENC only (24 more workflows). Top TRC input is
single-frequency lossless N/C, RMax=0/no B, blank restart and cLow >= last-node
speed, with a smooth V/R/A bottom. The runner consumes the fixture's `.trc`;
CLI/HDF5 hashes that same bounded snapshot and protects its input path/aliases.
These are constructed/derived tests, not original TRC input-pair acceptance.

Layered-fluid fixtures use the same commands and comparator (for example,
`case=LayeredFluidPower` and either engine). Twenty-two **derived** pairs provide
43 API/actual-CLI-HDF5 workflows, including wide refined three-layer KRAKENC:
N/C/P/S interfaces, all V/R/A combinations, unequal/fractional meshes,
per-layer power laws, biological loss, broadband
leaky roots and cross-layer FIELD. `LayeredDoubleRefined` is a denser-mesh
**derivative**, separately labelled from original TLslices `double`. The
byte-identical `OriginalLayeredDouble.env` plus official shared `fieldbat.flp`
now passes both engines (42 modes and 501 pressures each), including actual
legacy/JSON CLI-HDF5. `golden/original-double.sha256` records both inputs and six
new artifacts; three unmodified MOD/SHD runs agree. Refinement retains surviving
first-mesh data after a spectral exit, never an oracle count. Tests still check
count-increase rejection, failure after three successful frequency blocks and
old-output protection.
Analytic Munk and F/P/TRC remain single-layer. The test-only `.mod` reader walks
all finite fluid media and has a last-medium corruption regression.

Elastic half-spaces use the same full comparator and actual-CLI-HDF5 commands
(for example, `case=ElasticHalfTopBroadband`). Nineteen pairs give 37 workflows:
16 derived inputs plus original TLslices scholte/normal/flused with the official
shared fieldbat.flp. Full acceptance verifies those originals against the
installed fixed source, then compares every mode/pressure and elastic P/S
material record.
The HDF5 reader checks solve-frequency shear-loss and attenuation-model metadata.
Both engines accept top/bottom elasticity. Real tops retain shared isolation,
Brent and non-deflated Solve2, with top-A compressional but not shear/bottom
elastic absorption. Finite solid layers remain outside this half-space block.
See [scope/provenance](../kraken/elastic-halfspaces.md) and
`golden/elastic-halfspace.sha256` (149 input/artifact records).

Homogeneous finite caps additionally use 25 pairs / 50 full workflows / 56
frequency blocks (933 modes, 10,536 pressures), including byte-identical original
TLslices elsed/ice with official fieldbat.flp. Their 200-record
`golden/finite-elastic.sha256` locks input/artifact provenance. The existing
multi-frequency comparator now checks absolute fluid intervals and all finite
solid HDF5 material attributes; finite loss uses the existing lossy tolerance,
not a relaxed comparison. Both engines support contiguous fluid stacks;
comparisons use `solve_frequencies` to preserve real Solve2's search bound. See
[finite-cap limits and real-stiffness semantics](../kraken/finite-elastic-layers.md).

Depth-varying finite material adds 12 derived input pairs / 23 workflows / 29
frequency blocks / 158 modes / 1,827 pressures. `golden/graded-elastic.sha256`
locks 93 input/artifact records. The same comparator checks complete declared
results and all six solid-profile HDF5 datasets; JSON comparisons use the same
unmodified Fortran oracle. Triplicate MOD/SHD byte controls, tolerances and
work budgets are unchanged.

`python3 tools/reference/probe-layered-refinement.py` is a **diagnostic-only**
experiment for the historical three-layer refinement gap. It runs the unmodified
oracle, a zero-perturbation control and ±256-ULP seed probes in a disposable
container. The control binaries must match the oracle; altered results are
never acceptance goldens. It requires the pinned image and refuses to overwrite
`target/reference/layered-refinement-gap`. See the
[fix, ordinary regression and historical diagnosis](krakenc-three-layer-refinement.md).
The minimal N² arithmetic fix now passes the unmodified oracle; perturbed
reference products remain diagnostic only.

This compares every mode and pressure sample, including modal print precision,
mode-shape phase alignment, dimensions, and coordinate vectors. Full acceptance
runs all seventeen single-frequency KRAKEN cases, the KRAKENC derivatives above
and three derived broadband cases with fixed
tolerances, plus the full unmodified upstream MunkK and BroadBand/MunkK pairs
and the unmodified upstream MunkAnalytic `.env` with a derived coherent
FIELD `.flp`. The original MunkAnalytic three-line `.flp` fails in v2023.5
FIELD itself and is not an accepted upstream pair. Raw small reference goldens
and their [hashes/provenance](../../crates/kraken/tests/fixtures/golden/README.md)
also run in ordinary tests without Docker. MunkBottomLoss, MunkLossless and
SductTrapped are explicitly derived, reduced-grid fixtures. Unmodified MunkK
and all five original KRAKENC pairs (MunkKleaky, MunkKwb, MunkKbb, sductK,
calibK) are compared end-to-end; the derived trapped sduct is not the original
sduct.

Compare an `R` run against Rust on the host, or on the authoritative pinned
Linux x86-64 Rust 1.88 environment:

```sh
tools/reference/compare-ray.sh path/to/case.env
tools/reference/compare-ray-linux.sh path/to/case.env
```

Compare an arrivals (`A`/`a`) or pressure-field (`C`/`S`/`I`) run the same
way. The arrival comparator checks receiver counts and all eight arrival
fields at the committed single-precision storage tolerances; the pressure
comparator parses the fixed-record `.shd` layout, checks `LRecl` and the
receiver vectors, and compares every complex sample with `5e-8` absolute
pressure (override with `BELLHOP_DIFFERENTIAL_PRESSURE_TOLERANCE` and
relative tolerance `BELLHOP_DIFFERENTIAL_PRESSURE_RELATIVE_TOLERANCE`):

```sh
tools/reference/compare-arrival.sh path/to/case.env
tools/reference/compare-field.sh path/to/case.env
```

The semantic comparator checks launch angles, bounce counts, and trajectory
coordinates. It aligns isolated `1e-4 × base step` vertices because a value
within a few ulps of an SSP or boundary interface can make one compiler take
one minimum step while another reflects or changes segment immediately. Strict
aligned coordinates still use `1e-5 m`; the branch alignment window defaults
to `4.1` minimum steps and is reported separately. Override with:

```sh
BELLHOP_DIFFERENTIAL_POSITION_TOLERANCE_M=1e-6 \
BELLHOP_DIFFERENTIAL_MINIMUM_STEP_FACTOR=1 \
  tools/reference/compare-ray-linux.sh path/to/case.env
```

Run the committed critical boundary/interface cases with:

```sh
tools/reference/check-critical-rays.sh
```

Fresh reference outputs are written below `target/reference/` and are not
committed automatically. Ordinary parser and numerical tests use curated
fixtures under `crates/bellhop/tests/fixtures` and `crates/kraken/tests/fixtures`.

The complete acceptance sequence is
[`full-acceptance.sh`](../../tools/reference/full-acceptance.sh); the focused
commands above are reproduction aids, not a substitute for a complete run.
Standalone helpers still default to `target/reference/`; the full entry isolates
them using `PELAGIC_REFERENCE_DIR`. Verify all committed references with
`python3 tools/reference/verify-goldens.py`, which also rejects missing numerical
manifest entries. Numerical tolerances, case lists, exclusions and quotas are
not adjustable acceptance shortcuts.

# BELLHOP business-case coverage

## Latest conclusion: same-version Linux comparison (2026-10-14 UTC)

The latest investigation runs **both Rust and Fortran in the same Linux x86-64
container**, using Rust **1.99.0**, LLVM 23.1.1, release builds, and the unchanged
[pinned GNU Fortran 12.2.0 oracle](reference.md). The container runs through
Docker Desktop on an Apple ARM64 host; this is not a physical x86-server test.

Five numerical implementation defects and one non-finite-result publication
hole were fixed. Eight small, two-source Fortran pressure regressions were
added; no existing golden, pressure/ray tolerance, or resource quota was changed.
**Unconditional numerical replacement of every 2D input is still not established.**

The complete final diagnostic is retained at:

- `target/bellhop-business/linux-final/report.md`
- `target/bellhop-business/linux-final/results.json`
- `target/bellhop-business/linux-investigation/final.log` / `final.exit`

| Input group | End-to-end PASS | FAIL | Non-finite oracle | Explicit 3D exclusion | Total |
|---|---:|---:|---:|---:|---:|
| Unmodified official ENV inputs | 60 | 9 | 0 | 1 | 70 |
| Constructed two-source cases | 102 | 0 | 4 | 0 | 106 |
| Golden environments, fresh Fortran runs | 37 | 0 | 0 | 0 | 37 |
| Explicitly adjusted alternatives | 6 | 0 | 0 | 0 | 6 |
| **Total** | **205** | **9** | **4** | **1** | **219** |

This is a **complete diagnostic attempt, not release acceptance**. A Fortran
failure or timeout prevents that case's numerical comparison. The runner exits
nonzero for any non-pass. Counts are input paths, not unique physical problems:
seven new regressions repeat constructed inputs, and four adjusted alternatives
reuse already-tested geometric-Gaussian configurations. They do not create eleven
new physical scenarios.

Passing products contain **10,064,992 complex pressure samples**, **1,023,955
arrivals across 100,477 source/receiver cells**, and **7,862 ray/eigenray
trajectories**. These count the ENV differential once, not the repeated JSON and
CLI checks. 115 passing paths have multiple sources. The final exit code is 1,
with no `NOT RUN` cases.

## Reproduction and retained evidence

```sh
tools/reference/build-image.sh   # if the pinned oracle image is absent

# This must contain Linux x86-64 rustc/cargo 1.99.0, not macOS executables.
tools/reference/bellhop-business-linux.sh /absolute/path/to/linux-rust-toolchain \
  --output target/bellhop-business/my-linux-run --timeout 600

# Smaller real-reference diagnostic:
tools/reference/bellhop-business-linux.sh /absolute/path/to/linux-rust-toolchain \
  --only 'constructed/.*cerveny-.*WZ|adjusted/'

python3 tools/reference/bellhop-business.py --self-test
```

The measured toolchain directory was
`target/reference/three-layer-fix/linux-rustup/toolchains/stable-x86_64-unknown-linux-gnu`.
`Dockerfile.business` extends the pinned oracle image with build tools, retaining
its Fortran binary, libc and libgfortran versions. Rust reports
`1.99.0 (b940084d7 2026-09-28)`, target `x86_64-unknown-linux-gnu`;
the measured platform is `Linux-6.12.76-linuxkit-x86_64-with-glibc2.36`.
The oracle retains its original `-O1 -ffast-math -funroll-all-loops
-fomit-frame-pointer -std=gnu` flags. Rust uses Cargo's release configuration.

The original host-mode runner remains available. It does **not** put Rust in
Linux; only the Linux wrapper / `--native-reference` mode does. Each invocation
requires a new output directory. Default timeout is 120 seconds per stage; both
complete Linux investigations used 600 seconds. Increasing this operational time
allowance does not change a numerical tolerance or the solver's point budget.

The runner retains ENV/resources, canonical JSON, input and output hashes,
compiler identities, source-file hashes, executable hashes, and logs. Executables
are copied into each run's `bin/` before cases begin, so subsequent builds cannot
silently change the binaries partway through a matrix. Native Fortran execution
uses a short stem with the case directory as cwd: its `FileRoot` is only 80
characters, so passing an arbitrarily long absolute path is unsafe.

### Input and product coverage

- **70 official paths** come from literal active `bellhop` calls in pinned MATLAB
  test scripts. Bytes and same-stem resources are copied unchanged. There are 69
  2D inputs and the explicitly rejected 3D `MunkRot` input. MATLAB and boundary
  generators are not executed; shipped resource files are tested.
- **106 constructed shelf inputs** cover six field beam families, C/S/I modes,
  point/line sources, ASCII/binary arrivals, eigenrays, N/C/P/S/Q SSPs, V/R/A
  boundary pairs, attenuation units, Thorp/Francois-Garrison/biological losses,
  irregular grids, Cerveny width/curvature/image/component options, linear and
  curvilinear ATI/BTY, and beam patterns. All have two sources.
- **37 golden ENV inputs** are rerun with fresh Fortran. `InternalReflection` is
  excluded because original BELLHOP has no P-reflection implementation; its
  separate BOUNCE amplitude/phase regression remains authoritative.
- **Six adjusted alternatives** are additional labelled inputs, never silent
  replacements for failures; their changes and limitations are described below.

For each PASS, both ENV and exported JSON undergo the real Rust API differential,
actual CLI `--format both` execution, and independent native-versus-HDF5 readback.
SHD checks every source plane, ARR checks every source block in ASCII and Fortran
sequential-binary encodings, and eigenrays are deterministically ordered without
dropping duplicates. Last-source corruption controls must reject damaged SHD,
ASCII ARR, and binary ARR. Zero/multiple-test filters cannot count as success.

Pressure tolerance remains **`5e-8` absolute per component**, with no relative
allowance. Ray tolerance remains **`1e-5 m`**, plus the pre-existing separately
reported minimum-step branch allowance. The documented four SBCX arrival-count
exceptions remain narrowly scoped; no new exception was added. CLI native/HDF5
agreement is product consistency, not a second independent Fortran solver check.

## Fixed Rust numerical defects

| Root cause | Smallest fix | Reference regression / result |
|---|---|---|
| Scalar/complex division loses the negative-zero imaginary part on the WKB cut | Use a complex numerator before the square root in both Cerveny implementations | `Field_WkbGradientC/R`: both now exact on Linux |
| Simple-Gaussian BETA was interpreted as a double literal | Promote `0.98_f32`, as Fortran does | `Field_LineS`: exact |
| Line-source scaling used double pi/sqrt instead of ScalePressure's local default-real pi/sqrt | Evaluate that prefactor in f32, then promote | `Field_LineCervenyC/R`: exact |
| Semi-coherent source weighting evaluated sqrt(2) in double precision | Promote the default-real square root | `Field_LineSemiG/B`: exact |
| Near-field geometric-Gaussian diffraction floor used a double `0.2` | Promote `0.2_f32` | `Field_NearGaussian`: exact; official `calibB_gb` also passes |

The eight regression CSVs contain all **240 complex pressures**, including both
sources. They were extracted from unchanged pinned Fortran, never from Rust.
Their input/output hashes are in the fixture README and manifest. Existing
single-source field tests now also check all source blocks when present.

The WKB discrepancy was substantial, not ordinary rounding noise. Before the
fix, the two graded-water cases had maximum component errors `1.1113238055e-2`
and `1.1921957135e-2`, with 18 and 24 of 30 samples outside tolerance. On Linux,
the scalar probe gives:

```text
Fortran:                1 / (-1 + 0i) = -1 - 0i; sqrt = -i
Rust scalar numerator:  1 / (-1 + 0i) = -1 + 0i; sqrt = +i
Rust complex numerator:                -1 - 0i; sqrt = -i
```

`num-complex`'s scalar division computes the imaginary part as `0 - a*d/norm`;
that subtraction produces +0 here. Complex division preserves the negative zero.
The observed branch flip, failing regression before the edit, and exact result
afterwards identify the cause. Three original Fortran WKB repetitions also had
identical SHD bytes. Evidence is under `linux-investigation/`, including
`signed-zero-linux.log`, `field-red.log`, and staged `field-after-*.log` files.
The near-field regression likewise failed on Linux before its constant fix
(maximum `2.3841857910e-7`) and then agreed exactly.

## Remaining discrepancies: causes, decisions, and leads

### 1. Four fields intentionally retain double-precision receiver geometry

Production source/receiver geometry remains f64, as required by the existing
[precision decision](../bellhop/deviations.md). This preserves modern JSON
coordinates and avoids moving a boundary-depth input outside its f64 water
column. It was not downgraded merely to obtain green comparisons.

A controlled JSON experiment replaced **only receiver depths** with the exact
grid read from Fortran SHD; source depths and ranges were already identical.
No sound-speed, boundary, beam, tolerance, or solver setting changed:

| Original case | Maximum receiver-depth difference (m) | Maximum pressure error with matched depths |
|---|---:|---:|
| EllipseTLGB | `1.4880952367e-3` | `0` |
| EllipseTLGeom | `1.4880952367e-3` | `0` |
| SBCX pressure (`sbcx`) | `1.3871626436e-5` | `5.8207660913e-11` |
| arcticB | `2.8268914457e-4` | `2.9103830457e-11` |

All four controlled comparisons pass. Both original Ellipse ENV cases still
fail coordinate checks; original SBCX/Arctic fields still exceed pressure
tolerance. Controls **do not replace those results**. The table describes
in-memory coordinates, not the separate f32 native-output rounding differences.

Evidence: `linux-fix-probe3/case-*/matched-coordinates.{json,log}` and
`linux-investigation/matched-coordinates-results.json`.

### 2. Three Gaussian fields hit rounding-sensitive reflection branches

These failures persist with identical OS, Rust version, and oracle:

| Original case | Maximum component error | Pressures outside tolerance / total |
|---|---:|---:|
| DickinsB | `1.5493242245e-7` | 545 / 601,601 |
| DickinsFlatB | `9.6304574981e-7` | 386 / 201,201 |
| ParaBotTLGB | `2.9472284950e-6` | 15,849 / 501,501 |

Each was reduced to its first failing receiver and swept through **every original
launch angle**, retaining the original angular spacing via development-mode
single-beam selection. The significant discrepancies isolate to:

| Case | Beam (one-based) | Receiver (range, depth), m | Single-beam component error |
|---|---:|---|---:|
| DickinsB | 2836 / 4600 | `(16400, 10)` | `5.1288468583e-8` |
| DickinsFlatB | 52 / 500 | `(8500, 0)` | `1.1538804756e-7` |
| ParaBotTLGB | 49 / 1440 | `(80, -560)` | `1.4311942209e-7` |

Instrumented copies of the implementations show the shared failure mechanism:

- **DickinsFlatB:** Fortran reaches the surface at depth `+7.1054273576e-15 m`,
  whereas Rust reaches `0`. Fortran does not reflect yet; its minimum step is
  then `0.03 m`, reaching depth `-0.01754941055 m` before reflection. Rust reflects
  at zero. At the receiver, travel times differ by about `1.3904e-5 s`.
- **DickinsB:** the same `+7.1054273576e-15` versus zero surface decision adds a
  `0.03 m` Fortran minimum step, reaching `-0.01722216797 m`.
- **ParaBotTLGB:** at the curved top boundary, Fortran's signed inside distance is
  `-3.8191672047e-14 m`; Rust's is `+4.6629367034e-14 m`. Here **Rust** takes the
  extra `0.102 m` minimum step. The reflected path near the receiver moves about
  `0.10 m` in depth, changing both its direction and the Gaussian tail weight.

Thus tiny rounding differences are amplified into finite path differences by
`begin_distance > 0 && end_distance <= 0` and the `1e-4 * base_step` floor. This
is not explained by pressure accumulation, nor fixed by matching coordinate grids.
Instrumented Fortran SHD was byte-compared with its uninstrumented original to
ensure that the probes did not change the observed result. Scratch instrumentation
is retained under `target/`, not in production solver code.

A compiler control rebuilt **only Fortran Step.f90 without fast-math**. The
DickinsFlat single-beam example then agreed exactly. However, the complete
DickinsFlat field still had 7 failures (maximum `5.7153e-8`), DickinsB changed to
1,052 failures, and ParaBotTLGB was unchanged. This alternate oracle is diagnostic
only and was **not** used to award any matrix PASS.

**Decision:** do not add a guessed epsilon, perturb the input, mimic a particular
compiler's overshoot, or widen the field tolerance. A robust boundary-hit and
minimum-step policy needs a separate compatibility decision and validation over
critical rays, SSP interfaces, reflection curvature and p/q jumps. Relevant code:
`solver/integrator.rs::{step_2d,reduce_step}`, `solver/mod.rs::trace_ray`,
`solver/boundary.rs::signed_inside_distance`, and the reference `Step.f90`,
`Distances2D`, `Reflect2D`. The original strict failures remain visible.

Evidence: `linux-investigation/single-beams/`, `more-beams/`,
`*-intermediates*`, `strict-step/`, `strict-fields/`, and the retained probe scripts.
These isolate a failing path in each field; they do not claim every failing cell
has been exhaustively instrumented.

### 3. Four SGB noncoherent combinations have no meaningful reference field

The original `SS` / `IS` point/line shelf cases remain `ORACLE_NONFINITE`.
`InfluenceSGB` uses the coherent complex-amplitude contribution for these modes;
`ScalePressure` subsequently takes `sqrt(real(U))` as though it were intensity.
Negative real values become NaN. Merely choosing receivers/frequency so that the
real part happens to be positive would not repair the physical calculation.

Rust previously returned a `SimulationResult` containing NaNs; native output
rejected it later, but HDF5-only output could still publish it. The solver now
checks the **scaled** field and returns `BH0302` before publication. All four
cases are tested through ENV and JSON with both `both` and `hdf5` CLI formats:
nonzero exit, non-finite diagnostic, and no HDF5 product. This is safe rejection,
**not a numerical PASS**.

Four explicitly labelled alternatives change `S` beams to geometric-Gaussian
`B` beams while preserving S/I mode, point/line geometry, environment and grid.
All four yield finite reference fields and pass. This changes the beam model;
it does not make the original SGB combinations supported.

### 4. Two original ATOC cases exceed practical budgets

- `aetB`: Fortran finishes; Rust rejects the unchanged **20,000,000 traced-point
  budget** with `BH0303`. This is not a measured pressure mismatch.
- `aetB_TL`: original Fortran exceeds the **600-second** stage timeout. A separate
  Rust CLI execution also reaches `BH0303` (about 166 seconds in that run).
  No complete original-case numerical comparison is claimed.
- `aet_VLA_B` passes; it is a different input.

Neither quotas nor the CLI were expanded to disguise these failures. Existing
Rust `SimulationLimits` lets API consumers make explicit resource decisions;
the CLI does not expose that override.

Two labelled regional alternatives retain the original ATOC SSP, frequency,
boundary/material data, source depth and beam families, but use **50 km**, **3001
beams spanning -89 to 89 degrees**, **100 m steps**, and **101×101 receivers**.
Both pass. They demonstrate usable combinations within budget, not convergence
or acceptance of the original 500 km / 30,000-beam workload.

## Validation and historical comparisons

- **233 passed / 12 ignored** in ordinary workspace tests on macOS Rust 1.99.0.
- **233 passed / 12 ignored** in Linux Rust 1.99.0 release workspace tests.
  The first Linux workspace build lacked `curl` for Swagger UI; the successful
  rerun used the existing v5.17.14 archive via its supported `file://` override,
  SHA-256 `481244d0812097b11fbaeef79f71d942b171617f9c9f9514e63acbe13e71ccdc`.
  This is a build asset, not numerical reference data.
- Workspace all-target/all-feature Clippy with `-D warnings`, formatting, golden
  integrity, tooling guards and `git diff --check` passed.
- All added pressure goldens agree exactly on Linux; last-source corruption
  controls and the four original non-finite safety controls remain active.

Earlier evidence is preserved, not overwritten:

| Run | Platform / Rust | PASS / FAIL / oracle-nonfinite / 3D |
|---|---|---|
| `full/` | macOS ARM64 / 1.99.0 against Linux Fortran | 176 / 24 / 4 / 1 (205 paths) |
| `linux-baseline/` | Linux x86-64 / 1.99.0, before solver fixes | 182 / 18 / 4 / 1 (205 paths) |
| `linux-final/` | Linux x86-64 / 1.99.0, after fixes and additions | 205 / 9 / 4 / 1 (219 paths) |

The baseline includes one **harness** failure: the long Francois-Garrison
absolute path was truncated by Fortran's 80-character FileRoot. Short-stem
execution fixes it; it is not a solver discrepancy. Among the original 205 paths,
the final run has 191 PASS, 9 FAIL, 4 oracle-nonfinite and 1 exclusion. All 205
original input/resource hash sets are unchanged, and all 202 oracle artifacts
produced by both Linux runs are byte-identical (`linux-final/baseline-comparison.json`).
Earlier 20-case Linux Rust 1.88 controls remain historical only; they no longer
stand in for a same-version full Linux run.

Not covered: physical x86-server or Windows execution, private customer data,
exhaustive option Cartesian products, regeneration of all MATLAB boundaries,
beam/step convergence, independent wave-equation accuracy, or a fresh full KRAKEN
release-acceptance campaign. Passing implementation parity does not establish
those properties.

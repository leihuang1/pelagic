# KRAKEN business coverage and numerical findings

## Current numerical findings (2026-10-16)

The configured fresh-reference workflow passes **27/27 stages**. The broader
same-Linux inventory completes **345 paths: 306 PASS, 4 strict differential
failures and 35 separately classified unsupported/reference/missing-input paths**.
The four mismatches have supporting independent controls; they have not been
relabeled as official-reference passes. This is not full capability acceptance
or an arbitrary-spectrum completeness claim, and does not inherit BELLHOP's
acceptance decision.

These measurements used changes on base commit
`10d5afd44ec45e9ccb7c0e7cd7243041f7d585dc`, Rust 1.99.0 and the unchanged
[AT v2023.5 oracle](reference.md#pinned-linux-x86-64-oracle), in the same Linux
amd64 business container on an Apple ARM64 host. No reference patch, larger
solver quota, changed original input or replacement of an old golden was used.
The recorded full-workflow run was a **dirty-tree diagnostic**, not release
acceptance; a release requires its own clean-commit run.

## Complete business inventory

| Category | PASS | FAIL | Unsupported | Oracle nonfinite | Oracle failed | Oracle unconverged | Missing input | Total |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| Official active calls, explicit script loops/shared FLPs | 34 | 0 | 2 | 4 | 1 | 3 | 2 | 46 |
| FIELD fixtures, both engines and incoherent profile alias | 270 | 4 | 21 | 0 | 1 | 1 | 0 | 297 |
| Explicit KRAKENC controls: original BroadBand/MunkK and sductK | 2 | 0 | 0 | 0 | 0 | 0 | 0 | 2 |
| **Total** | **306** | **4** | **23** | **4** | **2** | **4** | **2** | **345** |

Every PASS requires valid fresh official modes/FIELD, complete ENV and exported
JSON API comparison, actual ENV/JSON CLI `--format both`, HDF5 comparison to the
reference and independent native MOD/SHD readback. Every declared binary mode,
sampled shape and pressure is traversed over all sources/profiles/frequencies.
[Comparison rules](reference.md#comparison-rules) remain distinct from BELLHOP's.

Passing coverage is **11,648 modes, 5,962,078 pressures, 367 frequency blocks and
396 profile-frequency blocks**, without multiplying counts by serialization
repetitions. There are no NOT_RUN entries. Missing inputs were not executed;
the 21 unsupported fixture paths stop at export, without a numerical solve.
Original official paths still run the reference even if Rust rejects the input.
Six fixtures without FLP are listed as modal-only exclusions from this runner;
configured full acceptance checks their modes separately.

This inventory intentionally tries additional engine/fixture combinations; it is
not an option Cartesian product or a general MATLAB interpreter. Its executable
discovery and accounting rules live in
[`kraken-business.py`](../../tools/reference/kraken-business.py).

## Numerical changes and regression evidence

### Rough fluid interfaces

Both engines retain the complete complex first-mesh Kuperman–Ingenito scattering
perturbation through Richardson refinement. The [roughness guide](../kraken/roughness.md)
owns the input/model contract, API migration and supported fluid V/R/A boundaries.
Elastic/table roughness remains rejected.

Four ENV/FLP pairs provide eight workflows, **14 frequency blocks, 322 modes and
3,510 pressures**. Three are byte-original TLslices surface/bottom/internal cases
with their official shared FIELD input; the broadband derivative adds repeated
frequencies, material losses and three sources. Triplicate official MOD/SHD files
are identical. Independent controls check surface loss, quadratic height scaling,
identical-material cancellation and unsupported combinations.

### Separated-duct roots

Spacing prediction can skip interleaved, nearly coincident families in separated
ducts. The corrected seed selection recognizes a sampled inter-duct speed barrier
and uses previous-root continuation there; single-duct Munk retains prediction.

| Input / interpolation | Earlier KRAKENC count | Corrected count / both official engines |
|---|---:|---:|
| SductTrapped / C | 71 | 194 |
| SductPchip / P | 80 | 194 |
| SductSpline / S | 92 | 184 |

Three new goldens cover **572 modes and 75 pressures**. Six additional lossless
C/S controls at 8,000/12,000/16,000 intervals agree with the separate real Sturm
counts and k within `5e-10`. Original BroadBand/MunkK still passes 50/500 Hz
(102/1,023 modes, 1,003,002 pressures) under the **unchanged 300-million root-work
ceiling**. Continuation for every stratified profile exceeded that budget;
first-two-seed and midpoint experiments remained incomplete and were not adopted.

### Lossy Pekeris roots and normalization

Independent continuum dispersion and contour counts establish **seven modes in
each original DenseLoss and RigidLoss case**. Rigid-top continuation could skip a
root on the first mesh, then find it on a finer mesh and trigger count-increase
protection. Corrected initial guesses and spacing prediction avoid that failure;
count-increase rejection, spectral limits and work quotas remain intact.

A large first inverse solve could still leave shape error above `1e-6` on a
12,000-interval control. Accepted complex shapes now receive at least two inverse
iterations, within the existing three-pass ceiling. First-mesh shapes/group speeds
are still retained; they are not Richardson-extrapolated.

Committed [numerical regressions](../../crates/kraken/tests/numerical_regressions.rs)
check complete roots, normalized shapes, group speeds and all line-source
pressures at 4,000/8,000/12,000 intervals, including a 50-Hz rigid-top derivative.
An additional recorded diagnostic covered **72 frequency/loss/mesh combinations**,
with independent complete counts and maximum k error **2.04e-11 m⁻¹**. Both
original lossy cases' ENV/JSON CLI native/HDF5 controls pass; maximum pressure
error against continuum predictions is **2.51e-7**, below the existing `2e-6`.

### Comparator corrections

Printed attenuation accepts ordinary decimals as well as exponent notation,
using the same half-last-printed-place rule. For binary imaginary k, only an
**exact reference zero** receives the existing printed-loss floor of
**1e-12 m⁻¹**. Production roots are not clamped; physical-loss, count, shape and
pressure criteria are unchanged. Focused controls accept machine-scale residuals
and reject changed attenuation/nonfinite values. `ProfilesAd` and `ProfilesCm`
now pass the complete KRAKENC business chain, not just the former assertion.

## Four retained original-KRAKENC differences

| Input | Reference difference and independent evidence |
|---|---|
| PekerisDense | Official KRAKENC declares 6 modes; Rust retains 7. The independently checked lossless Pekeris spectrum and fresh real-reference modes/FIELD support all seven. |
| PekerisFiltered | Official KRAKENC includes a 1512.936694 m/s mode below the requested 1520 m/s lower bound. Rust's two in-band modes agree with independent/real-reference evidence. |
| PekerisRigidPlane | Official KRAKENC declares 1 mode; Rust retains 2, including the analytic rigid-rigid plane mode, and matches real-reference products. |
| PekerisDenseLoss | Official KRAKENC declares 4 modes and leaves stale records. Independent complex dispersion, shapes and FIELD support Rust's full seven-mode spectrum. The undeclared records are not used to turn the original reference into a pass. |

Fresh controls for the first three compare the KRAKENC API and actual ENV/JSON
CLI HDF5/native products against the **real** reference, explicitly labelled as
an alternate-engine control. The original complex-reference mismatch remains.
DenseLoss uses independent continuum evidence instead of treating real KRAKEN's
perturbative loss as an exact lossy oracle. No valid mode is dropped to imitate
an incomplete reference.

## Other retained outcomes and accuracy limits

- Four real-engine originals (`DickinsFlatK`, `calibK`, `calibKgrad`, `sductK`)
  contain NaN group speeds. Rust's trapped-interval restriction remains.
- Original Noise `Iso_C`, `DownwdRef_C`, `Sduct_C` fail reference inverse iteration;
  KRAKENC `GradedElasticTopNRefined` exceeds reference mesh convergence.
- Original DownwdRef has 501 receiver depths but 101 offsets, rejected by FIELD.
  Real-engine `ElasticHalfLeaky` has no valid spectral interval.
- Original Iso has unsupported ENV option characters; Arctic has trailing FLP
  records. The other 21 unsupported paths mainly attempt real-engine leaky/table
  combinations intended for KRAKENC. Original files were not rewritten.
- Stale scripts refer to absent `MunkLeaky/MunkK` and `PekerisRD/DickinsFlatK`;
  no similarly named file from another directory substitutes for them.

The largest passing pressure norm error is **3.332000937312528e-7**. Absolute
agreement does not establish universal relative/TL/phase accuracy:

- Real SductSpline: **97.409% relative L2**, reference max magnitude `8.9856e-11`,
  max absolute error `8.8472e-11`. Complex SductSpline: **31.997% relative L2**,
  reference max `1.1092e-11`, absolute error `3.3306e-12`. All other passing
  frequency blocks have relative L2 at most `7.7918e-7`.
- Original Noise Sduct's real-engine surface null has **81.231 dB** TL difference
  at source 0.5 m, receiver 0 m, range 40.272 m: magnitudes `3.7996e-14` versus
  `3.2975e-18`. Whole-field relative L2 at 200/400/800 Hz is approximately
  `1.17e-7 / 9.72e-8 / 8.24e-8`. Its roughness also exceeds the reference model's
  stated perturbation-validity region.
- Raw near-null TL differences can exceed **300 dB**, and phase differences can
  approach 180 degrees. One-sided zeros and reference warnings are retained.

No amplitude floor hides these metrics. Weak-field relative accuracy and
business-specific TL/phase requirements remain distinct from absolute parity.

## Verification, history and reproduction

The recorded macOS debug and same-container Linux release workspace runs each
passed **242 tests / 0 failures / 12 ignored**; external-reference entry points
ran separately. Clippy, fmt, documentation, tool guards, golden integrity and
diff checks passed. KRAKEN integrity is **1,089 manifest records / 775 numerical
artifacts**; BELLHOP remains 83 / 38. Configured acceptance passed **27/27** in
535 seconds, with diagnostic exit 2. The business inventory exits 1 for its
retained exceptions.

Earlier outcomes are preserved, not rewritten as current results:

| Run | Paths | PASS | FAIL | Unsupported | Nonfinite | Failed oracle | Unconverged | Missing |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| Initial business run | 343 | 297 | 14 | 25 | 4 | 1 | 0 | 2 |
| Roughness / separated-duct follow-up | 345 | 303 | 7 | 23 | 4 | 2 | 4 | 2 |
| Lossy-Pekeris / reference-zero follow-up | 345 | 306 | 4 | 23 | 4 | 2 | 4 | 2 |

The middle run covered 11,615 modes / 5,962,000 pressures and retained seven
additional KRAKENC failures. All seven also reproduced on the frozen pre-work
CLI; six successful native pairs were then byte-identical, and RigidLoss rejected
the run. The latest changes convert RigidLoss and the two profile comparisons to
PASS. All 303 earlier passes remain passes; **606 common official MOD/SHD products
are byte-identical**, with unchanged input hashes and oracle binaries. Complex
SductSpline relative L2 was 81.849% before the inverse-iteration change; the
remaining weak-field limit is not waived by its improvement.

Use [reference workflows](reference-workflows.md) for fresh reproduction and
`cargo test --locked -p kraken --test numerical_regressions` for committed
independent controls. Detailed historical derivations stay in the
[checkpoint history](kraken-checkpoints.md) and golden ledger.

Local run identifiers, not files supplied by the repository:

- Initial and middle inventories: `target/kraken-numerics/{linux-full,linux-final}/`.
- Latest inventory, additional controls and summaries: `target/kraken-loss-diagnosis/`,
  especially `linux-final/`, `product-controls-final/` and `final-summary.json`.
- Earlier baseline CLI controls: `target/kraken-numerics/{baseline-main,remaining-controls}/`.
- Latest configured diagnostic: `target/acceptance/20261015T235148Z-46446/`;
  earlier 27-stage diagnostic: `target/acceptance/20261015T141602Z-18079/`.

No latest-official KRAKEN upgrade, independent full wave-equation certification
or private customer-case acceptance is claimed. BELLHOP's deferred-boundary
acceptance decision is unchanged.

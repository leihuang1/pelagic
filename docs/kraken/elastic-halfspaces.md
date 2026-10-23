# Elastic half-space checkpoint

The oracle is unmodified Acoustics Toolbox v2023.5, commit
`475108519289c6fb488b58980c644ea14eccc604`, Linux x86-64 / GNU Fortran 12.2.0,
with the existing pinned flags and unchanged numerical tolerances. This is
half-space support, not graded finite elasticity or arbitrary-spectrum certification.

## Supported boundary subset

- N/C/P/S constant-density finite fluid stacks with elastic A bottom, top or
  simultaneous top/bottom: both KRAKEN and KRAKENC, modes, FIELD and actual CLI/HDF5.
- Positive shear speed, positive density and bulk modulus (`cp² > 4/3 cs²`),
  finite nonnegative compressional/shear losses with `Im(c) <= Re(c)`.
- Existing frequency ordering, independent fluid meshes, refinement and shared
  budgets remain in force. No new root-work allowance or partial publication.
- Source/receiver/modal depths stay inside the finite **fluid** stack. These are
  pressure eigenfunctions, not elastic displacement/strain samples.
- Analytic Munk, F/P/TRC combinations, roughness and elastic coupled FIELD remain
  excluded. Real elastic top half-spaces combined with finite solids remain
  explicitly unvalidated; the separate [finite-cap block](finite-elastic-layers.md)
  retains its published scope.

`Boundary::ElasticHalfSpace { shear_sound_speed_mps,
shear_attenuation_db_per_wavelength }` supplies shear material; the existing
boundary sound-speed, density and loss fields supply compressional material.
Losses are canonical solve-frequency dB/wavelength. Legacy A records with cs>0
select this variant; conversion is repeated per frequency. A fluid cs=0 record
still forbids shear loss. No elastic boundary becomes a fictitious fluid.

## Reference arithmetic and search

`BCImpedance{c}Mod` provides pressure/normal-velocity impedance and its
normalization derivative. Real KRAKEN includes each elastic boundary's mode-count
contribution, caps cHigh at cs and retains the default-kind 0.85*cMin adjustment.
KRAKENC retains its separate complex radiation roots, deflation and work limits.

Real elastic tops use Solve1's **shared** isolating intervals and extended-range
ZBRENTX on meshes one/two, then non-deflated Solve2 with raw Neville history.
Independent per-root inertia bisections can follow an impedance pole instead.
The pinned same-sign warning retains the preceding initialized root; these raw
repeats are not silently removed. Solve's MINLOC selects M from the previous
Richardson row before adding the current mesh, with first-index tie handling.
Surviving shapes, group speeds and loss remain first-mesh data. No reference
count is read by the solver or used as a root cap.

`solve_frequencies` provides ordered run-local execution and stops on the first
error. Solve1 recounts modes on the first two meshes of each half-space frequency;
finite-solid Solve2 runs instead retain their preceding search bound. Independent
`solve`/`solve_field` calls start fresh runs. Blocks must share one backend.

**Reference loss limitations:** real elastic impedance ignores P/S material
loss. Bottom elastic absorption is omitted. However `Normalize` still applies
the generic top-A **compressional** loss perturbation (default-kind complex
square root), even when the top is elastic; top shear loss is ignored. Fluid
volume loss remains active. This is not a full elastic absorption model: use
KRAKENC for that. Group speeds follow the pinned compressional half-space
slowness formula, not independently verified elastic energy/group dispersion.
HDF5 records requested losses and `*_elastic_attenuation_model=reference_real|complex`;
`reference_real` identifies these reference rules, not zero modal attenuation.

## Complete acceptance evidence

19 input pairs: **16 derived** and **3 unmodified original environments with
their official shared FIELD geometry**. They produce **37 workflows, 49 frequency
blocks, 473 modes and 5,715 pressures**. Every declared mode, sampled shape and
pressure is compared through the API and actual CLI/HDF5. MOD/SHD are byte-identical
in three independent pinned runs. Local maximum pressure error remains 6.67e-8;
the ten added workflows have maximum |dp|=1.877140660839749e-9. Tolerances, budgets
and earlier goldens are unchanged. Rust HDF5 is readback evidence, never a golden.

| Inputs | Engines | Modes per frequency | Pressures per engine |
|---|---|---|---:|
| ElasticHalfBottomN/C/P/S | both, each input | 5 | 63 |
| ElasticHalfTopN/C/P/S | KRAKEN / KRAKENC | 4 / 6 | 63 |
| ElasticHalfBothN/C/P/S | KRAKEN / KRAKENC | 3 / 5 | 63 |
| ElasticHalfTopBroadband, 50/25/37.5/25 Hz | KRAKEN / KRAKENC | 4/3/3/3 / 6/3/5/3 | 252 |
| ElasticHalfLeaky | KRAKENC | 5 | 63 |
| ElasticHalfPower, 75/50/62.5/50 Hz | both | 7/5/6/5 | 252 |
| ElasticHalfShearOnly, coherent line source | both | 5 | 63 |
| OriginalElasticScholte | both | 45 | 501 |
| OriginalElasticNormal | both | 44 | 501 |
| OriginalElasticFlused | both | 46 | 501 |

TopBroadband explicitly derives from TopC by adding the B frequency vector;
materials, NG, RMax and FIELD geometry stay unchanged. It is not an upstream
original. The eight newly accepted real single-frequency cases retain their
existing input bytes. WRITE-only diagnostic Fortran rebuilds preserve MOD/SHD
byte controls; no seed or tolerance perturbation supplies acceptance evidence.

Original environments are byte copies of `tests/TLslices/{scholte,normal,flused}.env`.
Their FLPs are byte copies of `tests/TLslices/fieldbat.flp`, selected by upstream
`runtests.m`. Original normal retains bottom cs=2000, independently of the earlier
shear-removed LayeredNormalization. [Full pre-release acceptance](../development/reference-workflows.md#full-pre-release-acceptance)
checks source bytes before the complete comparisons. See [golden provenance](../../crates/kraken/tests/fixtures/golden/README.md)
and the **149-record** `golden/elastic-halfspace.sha256` manifest.

WriteMode can leave bounded stale first-mesh records after a smaller declared
spectrum; they are not additional modes. Finite-solid reference search starts
at M=3000; half-space Solve1 counts at most one sign change per acoustic interval,
two elastic-boundary contributions and the final dispersion sign. The comparator
checks declared modes, k, shapes and pressure strictly and bounds only this
writer-specific tail; other file-length checks stay exact.

Ordinary tests cover material diagnostics, per-frequency conversion/order,
repeated frequencies, reset, reference cutoff, top compressional versus shear
loss, decimal-exponent Brent arithmetic/work bounds, HDF5 metadata and output
preservation. Real TopN, BothS and TopBroadband additionally pass self-contained
JSON API and actual JSON CLI/HDF5 against the same oracle.

## Remaining limits

Successful execution is not a promise of all mathematical roots or fixed-oracle
parity for arbitrary branch-sensitive spectra. A 75/50/62.5/50 Hz top/both
broadband probe produced a first-root same-sign Brent warning and failed inverse
iteration in unmodified real Fortran; FIELD then failed reading the incomplete
MOD. Those probes are not accepted fixtures or goldens. Rust diagnoses a first
unbracketed root rather than retaining uninitialized reference output.

The older slow-shear Scholte probes remain unaccepted: cp=3000/cs=1000 reached
a group speed near 17344.62 m/s while pinned G14.6 prints 17344.6, outside the
existing 0.005 comparison; cp=1800 also failed strict checks. Neither is promoted
by this block. The former [wide layered refinement exception](../history/krakenc-three-layer-refinement.md)
is fixed, and original `double` now passes its separate fluid-stack checkpoint;
neither grants a waiver for elastic failures.

Depth-varying finite caps are covered by the separate [finite-layer checkpoint](finite-elastic-layers.md).
Interleaved solids, real elastic-top/finite-solid combinations and an elastic
multi-profile matrix remain later work. BOUNCE generation, HTTP, elastic
displacement and time-domain synthesis are not added.

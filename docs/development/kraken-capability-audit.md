# KRAKEN capability audit before business-case validation

**Historical first-stage assessment.** The subsequent
[rough-fluid-interface implementation](../kraken/roughness.md) closes gap 1 below.
The original rejection inventory remains unchanged as baseline evidence.
Fresh official probes also found NaN group speeds in all three real-KRAKEN
wide-interval cases from gap 2; Rust's safety restriction has not been removed.
The [subsequent business matrix](kraken-business-coverage.md) also records the
separated-duct root fix and complete outcomes; the input-only counts below are
not the newer numerical results.

## Decision (2026-10-15)

**The main KRAKEN/KRAKENC/FIELD numerical workflow is implemented, but full official
2D capability coverage is not complete.** Existing representative acceptance is
substantial; it is not exhaustive coverage of official business inputs or every
option combination. Do not start by declaring a larger test matrix an acceptance
pass, and do not inherit the operator's BELLHOP acceptance decision for KRAKEN.

This first stage checked the code at the baseline commit, pinned official v2023.5 source/tests,
existing numerical comparators and actual CLI input validation. It changes no
solver, tolerance, budget or golden. The original target at
`2a7a658:docs/kraken-compatibility.md` remains unchanged. No third-party solver or
patched reference is introduced. The official 2026 release has **not** been
numerically assessed for KRAKEN in this stage.

## Implemented capabilities at the audit baseline

These are bounded capabilities, not a Cartesian-product promise. Details and
historical evidence remain in [compatibility](../kraken/compatibility.md),
[checkpoints](kraken-checkpoints.md) and the [implementation map](kraken-internals.md).

| Area | Baseline implementation and representative evidence |
|---|---|
| Real and complex modes | KRAKEN real search/perturbative loss; KRAKENC complex roots including leaky modes. First-mesh shapes/group speeds and bounded Richardson eigenvalue refinement. Pekeris, Munk, leaky Munk, sduct, calib and broadband references. |
| Fluid profiles | Contiguous constant-density-per-medium stacks; N/C/P/S interpolation, independent layer meshes and material jumps. Fixed analytic Munk A is single-layer/lossless. |
| Attenuation | Legacy N/M/m/F/W/Q/L material units and T/F/B volume loss; conversion at each solve frequency. Both fluid and supported elastic material paths have reference fixtures. |
| Boundaries and elasticity | Smooth V/R/A; fluid and elastic half-spaces; homogeneous/depth-varying finite elastic caps above/below fluids. Original TLslices double/scholte/flused/normal/elsed/ice evidence. |
| FIELD | X/R/S source geometry, patterns, coherent/incoherent synthesis; range-independent, adiabatic and restricted smooth-fluid coupled propagation. Original Gulf AD/CM evidence. |
| Ordered inputs | Multiple frequencies, repeated frequencies, multiple sources/receivers and profiles; legacy ENV/FLP/resources and self-contained JSON. |
| Actual products | CLI native MOD/SHD, HDF5 and `--format both`; per-frequency native pairs retain profile order. Publication and readback tests exist. HTTP, FIELD3D, time-domain synthesis and BOUNCE generation are separate/out of scope. |

Real KRAKEN's omitted elastic absorption and approximate elastic group-speed
formula are inherited reference limitations, not complete physical models.
KRAKENC is the supported complex-loss path. Agreement with either program is
not independent physical truth.

## Gaps and restrictions observed at the baseline

1. **Fluid-interface roughness/scattering is missing.** Official
   `Kraken/Scattering.f90::KupIng` is called from both engines' `ScatterLoss`.
   Rust rejects nonzero surface, bottom and internal roughness in
   `input/legacy/mod.rs::read_finite_layer` and the bottom-options reader.
   Original `TLslices/kupert`, `kuperb`, `kuperm` exercise these three locations;
   all six engine/input paths are rejected. This is a missing numerical
   capability, not merely missing test coverage. Rough *elastic* interfaces are
   rejected by the official solver too; they are not part of this gap.
2. **Real-engine input coverage is narrower than official calls.** Original
   `sduct/sductK`, `calib/calibKgrad`, `Dickins/DickinsFlatK` request a fluid
   phase-speed upper bound above the bottom speed and are rejected by Rust
   KRAKEN. `Case::from_definition` enforces this deliberately. The official
   `kraken.f90::Initialize` has its acoustic cHigh clamp commented out; actual
   reference behavior must be measured before accepting or emulating it.
   KRAKENC acceptance of sduct does not establish acceptance of the original
   **KRAKEN** workflow. Do not silently switch engines or narrow the interval.
3. **Reflection/impedance tables are a limited slice.**
   `solver/reflection.rs::{validate,validate_surface}` permits single-frequency,
   single-fluid, lossless N/C KRAKENC with RMax=0. Bottom F/P requires vacuum top;
   top F also limits the spectral interval. Real KRAKEN F/P, top P and the other
   material/interpolation/refinement combinations are not implemented/accepted.
   Official BC routines contain additional branches, but their successful
   numerical behavior is not established here merely by reading the branches.
4. **Elastic and coupled combinations remain restricted.** Real elastic-top
   half-spaces plus finite solids are explicitly unvalidated/rejected. Coupling
   rejects elastic/table boundaries and certain interface sampling grids.
   Interleaved/pure solids and displacement/strain outputs are outside the
   current slice; this audit does not assert that every such geometry is valid
   in official KRAKEN either. Fluid density gradients are rejected deliberately:
   official acoustic shooting homogenizes density to each medium's top value,
   so accepting the syntax would not implement a physical density gradient.
5. **Legacy input compatibility needs separate classification.** Extra ENV
   option characters, trailing FLP records and malformed geometry can reject
   official distributed examples. These are not automatically missing physics.
   Fortran null/repetition syntax and continuation/search options are also not
   a general supported list-directed interface. In particular official
   KRAKENC's old `I` initial-mode-file option explicitly stops as unsupported;
   it must not be counted as functioning reference capability to port.

## Input-only baseline probe

Rebuilt the CLI from merged base
`10d5afd44ec45e9ccb7c0e7cd7243041f7d585dc` and called `pelagic kraken export` on
**42 selected original ENV/FLP/engine paths: 26 INPUT_ACCEPTED / 16 INPUT_REJECTED**.
This is **input validation only**, not 26 numerical passes or 16 numerical
mismatches. No modes or fields were solved by this probe. Selection includes all
11 official TLslices cases through both engines, selected literal calls in the
Munk/BroadBand/leaky/sduct/calib/arctic/Dickins scripts, both Gulf geometries and
six KupermanIngenito noise-case inputs. It is not an exhaustive discovery run.

| Rejected group | Paths | Observed cause / interpretation |
|---|---:|---|
| TLslices kupert/kuperb/kuperm, both engines | 6 | Missing roughness support, as above. |
| sductK/calibKgrad/DickinsFlatK, real KRAKEN | 3 | Deliberately unsupported phase-speed interval; reference behavior not yet rerun. |
| arctic/arcticK | 1 | Extra FLP records after receiver offsets; strict Rust parser rejects trailing input. Do not erase them from the original. |
| Noise Iso/Iso_C/DownwdRef_C | 3 | Extra top-option characters rejected; first error only, not proof that relaxing it makes the workflow supported. |
| Noise DownwdRef | 1 | FLP has 501 receiver depths but 101 offsets. Official `KrakenField/field.f90` also explicitly checks equality; this is not evidence of a missing Rust physics feature. |
| Noise Sduct/Sduct_C | 2 | Nonzero surface roughness rejected. Noise postprocessing itself is outside the FIELD product assessed here. |

Input bytes were hashed before/after each command and left unchanged. Shared
`TLslices/fieldbat.flp` and Gulf FLPs were supplied explicitly, following the
official scripts; no adjusted ENV/FLP replaced an original. Per-path stdout,
stderr, hashes, binary identity and probe source remain under
`target/kraken-capability-20261015/{input-results.json,probe.py,probe.log}`.
These are local diagnostic artifacts, not committed golden references.

## Verification evidence and its limits

Fresh checks in this stage:

- `cargo test --locked -p kraken --all-features` with installed stable Rust:
  **120 passed / 0 failed / 6 ignored**, including the doc test. The six ignored
  external-reference entry points were **not** run.
- Two actual CLI tests passed: default/native/HDF5/both directory behavior, and
  independent frequency geometry/repetitions with multi-profile native readback.
- Tool guards and golden integrity passed: KRAKEN **1,048 manifest records /
  742 numerical artifacts**, BELLHOP 83 / 38. These are integrity counts, not
  independent case counts.

Existing `differential_reference.rs` checks all declared modes' binary k and
sampled eigenfunctions, and every pressure over all sources/frequencies/profiles.
Mode phase alignment does not renormalize amplitude. Real k is exact after f32
storage conversion; shape tolerance is 1e-6 and complex-pressure norm tolerance
is **2e-6**, not BELLHOP's per-component 5e-8. Printed k uses 5e-10, phase speed
5e-6 and group speed 0.005; `.prt` prints only a stride of modes for large spectra,
so these high-precision/speed checks do **not** cover every mode in those cases.
No tolerance is changed by this audit.

Fresh-reference scripts already compare API and actual HDF5 values to official
MOD/SHD, and native readers independently compare CLI native products to HDF5.
Reuse that chain rather than inventing another output framework. The historical
`target/acceptance/20261011T140635Z-60375/report.md` has passing numerical stages
but explicitly says **NOT RELEASE ACCEPTANCE** for its dirty tree/older commit;
it is not fresh acceptance of current HEAD. Existing analytic Pekeris checks are
useful independent evidence, not a general elastic/leaky completeness proof.

## Plan recorded at that stage

The following was a proposal when this audit was written. Later implementation
and numerical outcomes are in the [business coverage record](kraken-business-coverage.md);
this is not the current work queue.

1. Resolve the capability scope first. The clearest missing official physics is
   rough-fluid-interface scattering; next, characterize the three original
   real-KRAKEN wide-interval inputs and the table restrictions. Any implementation
   should be a separate, minimal change with official and independent controls.
2. Inventory official active workflows, including script loops, shared FLPs and
   generated table resources. Keep engine identity, pristine hashes and original
   failures. Label invalid-reference input, unsupported capability, resource
   rejection, reference failure and numerical mismatch separately.
3. Then run the supported business matrix in one pinned Linux environment:
   Fortran + Rust ENV + exported JSON + actual CLI `--format both`, using existing
   comparators/readers. Retain mode counts, k/attenuation, aligned shapes, fields,
   TL/phase metrics and every-source/profile/frequency coverage. Never turn input
   acceptance or process exit zero into numerical acceptance.
4. Add focused analytic/refinement evidence for demonstrated sensitive cases.
   Keep official-reference parity separate from physical accuracy and report
   incomplete convergence rather than relaxing tolerances or increasing quotas.

No fresh full official numerical matrix, latest-official KRAKEN upgrade,
arbitrary-root completeness proof or private customer case validation is claimed
by this first-stage audit.

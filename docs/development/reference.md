# Numerical reference

This page defines the oracle, comparison rules and evidence requirements.
For commands, use [reference workflows](reference-workflows.md); for CI and
release responsibilities, use [testing policy](testing.md). Current capabilities
belong in the [BELLHOP](../bellhop/compatibility.md) and
[KRAKEN/KRAKENC](../kraken/compatibility.md) guides, not in a second case list here.

## Pinned Linux x86-64 oracle

The compatibility oracle is the unmodified official Acoustics Toolbox release:

| Item | Identity |
|---|---|
| Source | <https://github.com/oalib-acoustics/Acoustics-Toolbox> |
| Tag | `v2023.5` |
| Commit | `475108519289c6fb488b58980c644ea14eccc604` |
| Source archive SHA-256 | `f8a7a2c1e80a73431cd230a10bef5fcfc996c88889a0e1540771c3922ee2a21f` |
| License | GPL-3.0 |
| Compiler | GNU Fortran 12.2.0 |
| Flags | `-O1 -ffast-math -funroll-all-loops -fomit-frame-pointer -std=gnu` |
| Debian bookworm-slim image index | `sha256:abd67ffcfa541b485a3dff59865ab629aa048a6c613e639d36e7456b0b229241` |

[`tools/reference/Dockerfile`](../../tools/reference/Dockerfile) builds BELLHOP,
KRAKEN, KRAKENC, BOUNCE and 2D FIELD from those sources. Bundled executables are
not substitute oracles. The same-Linux business image extends this image without
replacing its Fortran binaries or runtime; it pins libc6 `2.36-9+deb12u14` and
libgfortran5 `12.2.0-14+deb12u1`.

Repository development uses the Rust version in `rust-toolchain.toml`. The
separate critical-ray comparison retains Rust 1.88.0 and its bookworm image index
`sha256:af306cfa71d987911a781c37b59d7d67d934f49684058f96cf72079c3626bfe0`.
Record the actual compiler, target, flags and binaries for each numerical run.
A newer official release or patched Fortran is a separate diagnostic, not an
implicit oracle upgrade.

## Comparison rules

Comparators check declared dimensions, coordinates, frequencies, profiles and
sources before accepting numerical values. They reject missing/nonfinite data;
matching process exits or mode counts alone are insufficient. Native readers
also check record structure and completeness. Actual CLI HDF5 values are compared
to the oracle, and independent native readers cross-check the serialized products.

### KRAKEN/KRAKENC

The authoritative implementation is
[`differential_reference.rs`](../../crates/kraken/tests/differential_reference.rs).

| Quantity | Criterion |
|---|---|
| Mode count and declared geometry | Exact |
| Real k in binary MOD | Exact after f32 storage conversion |
| Imaginary k with declared physical loss/scattering | Absolute error ≤ `1e-10 m⁻¹` |
| Otherwise, imaginary k when the binary reference is exactly zero | Absolute error ≤ `1e-12 m⁻¹` |
| Otherwise, nonzero binary imaginary k | Exact |
| Sampled complex mode shape | Norm error ≤ `1e-6`, after unit-phase alignment only |
| Complex FIELD pressure | Norm error ≤ `2e-6` |
| Printed k | Absolute error ≤ `5e-10 m⁻¹` |
| Printed phase speed / group speed | Absolute error ≤ `5e-6` / `0.005 m/s` |
| Printed attenuation | Half the last printed place, with a `1e-12 m⁻¹` floor; decimal and exponent formats supported |

Shape alignment never rescales amplitude. Every declared binary mode and every
pressure is checked; large PRT tables print only a stride of modes, so the
higher-precision printed k/speed checks do not cover every mode. The near-zero
rule belongs to the comparator: production roots are not clamped to zero.

### BELLHOP

The authoritative implementation is
[`differential_reference.rs`](../../crates/bellhop/tests/differential_reference.rs).

| Quantity | Default criterion |
|---|---|
| Aligned ray coordinates | Absolute error ≤ `1e-5 m` |
| Complex pressure | Absolute error ≤ `5e-8` **per component**, no relative allowance |
| Arrival amplitude / phase | Absolute error ≤ `5e-7` / `5e-6 rad` |
| Arrival source/receiver angle | Absolute error ≤ `5e-5` degrees |
| Arrival travel time / attenuation time | Absolute error ≤ `5e-6` / `1e-7 s` |
| Arrival top/bottom bounce counts | Exact |

The semantic ray comparator separately measures alignment of isolated
`1e-4 × base step` vertices. A few ulps at an SSP/boundary interface can change
whether the reference takes a minimum step before reflecting or changing
segments. This allowance is not a larger ordinary coordinate tolerance.

The pre-existing four-cell arrival-count exception is restricted to the original
single-source SBCX input. Phase storage differs between single-precision degrees
in Fortran and radians in Rust; the phase criterion includes that quantization.
Neither exception is a general waiver. See
[intentional deviations](../bellhop/deviations.md) and the
[business findings](bellhop-business-coverage.md) for measured effects.

## Reference validity and independent evidence

- A successful process and existing MOD/SHD files do not validate fatal errors,
  NaNs, failed inverse iteration or unconverged meshes. Keep those outcomes
  separate from numerical mismatches and unsupported Rust inputs.
- Preserve original input/resource bytes, engine identity and source/binary
  provenance. Derived inputs, changed bounds and alternate engines must be
  labelled separately; they cannot replace an original failing workflow.
- Agreement with official output is compatibility evidence, not independent
  physical proof. Analytic dispersion, contour counts, Sturm controls and
  refinement/scaling tests provide separate evidence for their bounded cases.
- A confirmed reference difference does not silently become a differential PASS.
  Keep both the original mismatch and the independent control. Do not discard
  valid roots, enlarge work quotas or relax meaningful criteria to imitate it.
- Absolute pressure agreement does not establish relative, TL or phase accuracy
  near weak fields/nulls. Retain those metrics without an amplitude floor.

## Committed evidence and run artifacts

Small, fixed Fortran artifacts and their input derivations/hashes are documented
in the [BELLHOP golden ledger](../../crates/bellhop/tests/fixtures/golden/README.md)
and [KRAKEN golden ledger](../../crates/kraken/tests/fixtures/golden/README.md).
New goldens require explicit provenance; Rust output is never a replacement
Fortran golden. `verify-goldens.py` checks hashes, missing records and numerical
artifacts absent from the manifests. Existing goldens are not regenerated to
make a changed implementation pass.

Large original products, logs, binary snapshots and generated acceptance reports
live under `target/` and are not supplied by a fresh checkout. Recorded run paths
in investigation notes identify local evidence, not files required to use the
library. Reproduce them with the committed [workflow entry points](reference-workflows.md).
Generated BOUNCE tables are labelled generated resources, not shipped original
tables; the TabRefCoef preparation requires triplicate identical tables/MOD/SHD.

Current results and exceptions have one home per model:

- [BELLHOP business coverage and acceptance with exceptions](bellhop-business-coverage.md)
- [KRAKEN business coverage and independent controls](kraken-business-coverage.md)

Detailed implementation-stage counts remain in the
[KRAKEN checkpoint history](kraken-checkpoints.md) and golden ledgers. The full
configured case lists are executable in
[`full-acceptance.sh`](../../tools/reference/full-acceptance.sh); broader inventories
are discovered by the business runners. Neither list implies an untested option
Cartesian product or arbitrary-spectrum completeness.

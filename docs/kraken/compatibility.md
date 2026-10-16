# KRAKEN/KRAKENC compatibility target

The numerical reference is Acoustics Toolbox `v2023.5`, commit
`475108519289c6fb488b58980c644ea14eccc604`, using the pinned Linux x86-64 GNU
Fortran 12.2 environment described in [`reference.md`](../development/reference.md).

**Status:** bounded KRAKEN/KRAKENC modes and FIELD workflows are implemented,
including layered fluids, material/volume loss, rough fluid interfaces, supported
elastic boundaries/caps, discrete frequencies and multi-profile propagation.
Legacy and strict self-contained JSON use the same validated cases and actual
CLI native/HDF5 output paths. This is not complete coverage of the original 2D
target below or a promise of arbitrary-spectrum parity.

Use the current capabilities and limits below when choosing inputs. Measured
engine-specific outcomes, numerical fixes, independent controls and retained
exceptions have one home in the [business coverage record](../development/kraken-business-coverage.md).
For code ownership, see the [implementation map](../development/kraken-internals.md).

## Products

The target is the two-dimensional normal-mode workflow:

1. `KRAKEN` computes real-eigenvalue normal modes.
2. `KRAKENC` computes complex modes.
3. `FIELD` synthesizes complex frequency-domain pressure from a mode set.

The Rust result model exposes modes and pressure fields, not BELLHOP-style ray
arrivals. Rust CLI output defaults to native `.mod/.shd` pairs; `--format hdf5`
selects the independent KRAKEN HDF5 schema v1 and `--format both` writes both
from one solve. Multiple ordered frequency blocks use separate native pairs,
each retaining all profiles; native compatibility does not promise byte-identical
Fortran output. KRAKEN and BELLHOP schema v3 remain separate. A wideband
time-domain response would be a separate product and acceptance contract.

## Current capabilities and limits

The supported input/physics slice is not the entire original target below.
Each checkpoint retains its representative acceptance paths, not an inferred
Cartesian product of every option:

- Contiguous constant-density fluid stacks with independent N/C/P/S meshes,
  material/volume attenuation and V/R/A boundaries. [Rough fluid interfaces](roughness.md)
  additionally include the pinned first-mesh scattering perturbation through both engines.
  Fixed analytic Munk A
  remains single-layer and lossless. KRAKEN fluid A half-spaces stay trapped;
  KRAKENC also supports leaky intervals.
- Limited single-frequency KRAKENC N/C bottom F/BRC, P/IRC and top F/TRC
  paths; restrictions and exact resource formats remain in the
  [table/boundary checkpoints](../development/kraken-checkpoints.md#tabulated-krakenc-bottoms).
- [Elastic A half-spaces](elastic-halfspaces.md) and
  [homogeneous/depth-varying finite elastic caps](finite-elastic-layers.md),
  with the pinned real-KRAKEN stiffness/loss limitations preserved.
- [Single/multi-profile FIELD](field.md), with line/point/scaled sources,
  source patterns and coherent/incoherent addition within its published limits.
  Coupled/incoherent input and unsupported coupling grids are rejected.
- Ordered discrete frequencies, repetitions, strict [self-contained JSON](json.md)
  and actual [single-file HDF5 output](../guide/kraken-hdf5.md). Failure stops the
  sequence without publishing a successful prefix.

See [legacy syntax, precision and budgets](input.md) for the input contract.
Fluid density gradients, interleaved/pure solids, rough elastic/table interfaces, unvalidated
real elastic-top/finite-solid combinations, FIELD3D, BOUNCE generation, ray
arrivals, HTTP and time-domain synthesis remain outside the accepted slice.
Unsupported combinations are rejected, not silently approximated. Successful
execution or staying under a budget is not an arbitrary-input parity certificate.

[Checkpoint history and complete evidence](../development/kraken-checkpoints.md)
retain the earlier slice boundaries, all numerical counts and historical probes.
Later checkpoints extend earlier slices without changing the original target
at `2a7a658:docs/kraken-compatibility.md`.

## Original environment target

The original target calls for KRAKEN `.env` and FIELD `.flp` adapters,
including same-stem auxiliary resources used by selected cases. The strict,
[self-contained JSON adapter](json.md) follows BELLHOP's
single-document convention, with an independent KRAKEN schema and canonical
solve-frequency materials.

The v2023.5 KRAKEN environment reader supports these SSP interpolation options:

- `N`: N²-linear
- `C`: C-linear
- `P`: PCHIP
- `S`: cubic spline
- `A`: analytic profile

KRAKEN profiles are range-independent; range variation is represented as a
sequence of profiles for FIELD propagation. The final environment matrix
includes fluid and elastic layers, attenuation units (`N`, `F`, `M`, `m`, `W`,
`Q`, `L`), volume attenuation (`T`, `F`, `B`), multiple frequencies, and the
reference's top/bottom conditions (`V`, `R`, `A`, `F`, `P`). These cover vacuum,
rigid, half-space, tabulated-reflection, and precomputed-impedance paths.
Existing reflection tables are inputs; table generation by the separate
`BOUNCE` program is excluded.

## Original FIELD target

The final 2D FIELD matrix includes:

- point, line, and scaled-cylindrical source geometry;
- omnidirectional and tabulated source patterns;
- coherent and incoherent mode addition;
- range-independent fields and multi-profile adiabatic or coupled-mode
  propagation;
- multiple frequencies, source depths, receiver depths/ranges, and receiver
  range offsets.

The reference rejects incoherent addition with coupled modes; Rust validation
preserves that rule. FIELD's separate three-dimensional `FIELD3D` program
is out of scope, as are BOUNCE table generation and ray-arrival products.

## Reference families and acceptance

These are representative families from the original target, not a per-engine
pass list. The business coverage record distinguishes accepted workflows,
unsupported inputs and invalid references:

| Feature | Reference cases |
|---|---|
| Classic modes and FIELD | `tests/Munk/MunkK.env` + `.flp`, `tests/sduct/sductK.env` + `.flp` |
| Complex/leaky modes | `tests/MunkLeaky/MunkKwb`, `MunkKbb`, `MunkKleaky` |
| Complex mesh extrapolation | `tests/calib/calibK.env` + `.flp` |
| Multi-frequency | `tests/BroadBand/MunkK` |
| Adiabatic and coupled FIELD | `tests/Gulf/gulf_ad.flp`, `gulf_cm.flp` |
| Reflection inputs (KRAKENC) | `tests/TabRefCoef/neggradC_*` + BOUNCE-generated tables |

Acceptance compares modal wavenumbers/attenuation, normalized phase-aligned
shapes and every complex pressure sample, including actual CLI products.
Small committed goldens keep ordinary tests independent of Docker; original
large products are fresh-reference artifacts. Exact case lists belong to
[`full-acceptance.sh`](../../tools/reference/full-acceptance.sh), derivations and
hashes to the [golden ledger](../../crates/kraken/tests/fixtures/golden/README.md),
and comparison criteria to the [reference contract](../development/reference.md).
A changed engine, derived geometry or generated table remains explicitly labelled.

## Repository shape

The implementation follows the existing BELLHOP boundaries: `crates/kraken`
contains validated cases, legacy adapters, mode solving, and FIELD. The
`pelagic kraken` command in `cli` uses the output adapters for native MOD/SHD,
HDF5 or both; BELLHOP v3 and KRAKEN v1 schemas and result types remain independent.
The JSON adapter reuses those validated cases and output paths; HTTP is not
provided. No shared acoustics abstraction is introduced.

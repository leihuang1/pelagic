# Multi-profile 2D FIELD

This checkpoint covers adiabatic and coupled propagation from the initial
compatibility target at `2a7a658`. It does not introduce an option Cartesian
product or change the pinned reference, numerical tolerances or existing
solver budgets.

## API and legacy input

`legacy::load_field_cases(env, flp, solver)` loads a sequence of complete ENV
profiles followed by one FIELD geometry. The snapshot counterpart,
`load_field_cases_with_resources`, never rereads inputs. It returns one
`FieldCase` per frequency, preserving order and duplicates; all profiles must
have the same frequency vector. Existing single-profile APIs remain strict
and reject sequences instead of discarding trailing profiles.

`field_table_extensions` discovers required same-stem tables across the entire
ENV sequence, in TRC/BRC/IRC order. The snapshot API accepts that same three-slot
array: each profile receives only its own required tables, and snapshots unused
by every profile are rejected. Adiabatic profiles may change boundary types;
CLI provenance and input/output protection cover all consumed tables.

`FieldCase::new(profiles, ranges_m, propagation)` validates an immutable
sequence. Ranges start at zero and strictly increase. Profiles share frequency,
solver, source/receiver geometry, FIELD mode cap, addition and source pattern.
`FieldPropagation` is `RangeIndependent`, `Adiabatic` or `Coupled`.
`solve_field` returns every profile's complete `ModeSet`, plus one pressure grid
in the existing source/depth/range layout. Modal solving reuses the existing
KRAKEN/KRAKENC backends; no separate modal approximation is introduced.

Legacy FLP selects `A` or `C` in option position two when there is more than one
profile. Profile ranges use the existing vector expansion/sorting conventions.
ENV depth-only SSP rows ending in `/` inherit all remaining material values,
as required by the original Gulf sediment. A zero lower phase-speed bound lets
the solver use its physical minimum; negative bounds remain invalid.

## Propagation semantics

- **Adiabatic:** interpolate the pinned complex32 wavenumbers and receiver
  shapes between adjacent profiles. Integrate phase using the evaluator's
  midpoint rule, splitting steps at profile ranges. The active mode count can
  shrink but does not regrow. After the last profile, hold its modes constant.
  Preserve `EvaluateADMod` mixed precision and its real nonnegative RMS pressure
  for incoherent addition (different from single-profile `EvaluateMod`'s complex
  square-root convention).
- **Coupled:** interfaces lie midway between adjacent profile ranges. Advance
  excitation to each interface, project the pressure onto all modes of the new
  profile, then continue. The FIELD cap applies only to initial excitation;
  higher-order modes can be excited later. Projection uses the reference's
  density-weighted trapezoidal quadrature and analytic fluid-half-space tails,
  without storing a dense coupling matrix.
- Both paths preserve the pinned geometry normalization and zero-range behavior.
  Receiver offsets are retained in the result, but are not used by these
  multi-profile evaluators, matching v2023.5. Source-pattern shading applies to
  the first source block. Each Rust source/frequency march has independent state.

Coupled/incoherent input is rejected, as in Fortran. Coupling currently requires
smooth fluid profiles with modal samples spanning the complete fluid interval.
At the evaluator's float32 depth precision, internal fluid interfaces must not
fall strictly inside the first/last grid interval or share an interior
three-point quadrature stencil. Unsupported grids are rejected with `KR0201`.
Sampling every interface satisfies this rule; isolated off-grid interfaces
within interior stencils remain supported, as required by the original Gulf
inputs. No grid insertion or retabulation changes pinned quadrature arithmetic.
Adiabatic and range-independent sampling requirements are unchanged. Finite
solids, elastic half-spaces and reflection/impedance tables are not accepted for
coupling. Existing individual profile limits still apply, including
in-fluid source/receiver validation. These limits do not create additional
acceptance stages beyond the initial target's representative paths.

Input copies across profiles/frequencies and retained modal-shape samples are
bounded by 5,000,000 entries. The existing 550,000,000 FIELD work ceiling also
charges interface tabulation, projection and half-space tails. Numerical errors
identify the failing profile. CLI frequencies remain sequential; profile
metadata/modes are charged to the same output quota and flushed/checked per
profile. Any failure preserves the previous output and removes reserved scratch.

## Fixed evidence

| Path | Provenance | Profile modes | Pressures |
|---|---|---:|---:|
| `ProfilesAd` | constructed Pekeris derivative, four profiles, changing depth | 3/4/3/3 | 24 |
| `ProfilesCm` | same environments, coupled FIELD, initial cap 2 | 3/4/3/3 | 24 |
| `ProfilesAdIncoherent` | unchanged ProfilesAd ENV, FLP addition C→I | 3/4/3/3 | 24 |
| `GulfAd` | byte-original `Gulf/gulf_rd.env` + `gulf_ad.flp` | 63 + 7 × 62 | 501,501 |
| `GulfCm` | byte-original `Gulf/gulf_rd.env` + `gulf_cm.flp` | 63 + 7 × 62 | 501,501 |

The original four KRAKEN workflows compare every profile's wavenumbers, attenuation,
normalized/aligned shapes and every complex pressure sample, both via API and
actual CLI HDF5. MOD/SHD are byte-identical in three pinned runs. Local maximum
pressure error is `4.1159031748919954e-10`, at unchanged tolerances. Ordinary tests
use only the two small committed golden pairs; original Gulf outputs are fresh
reference artifacts, not committed large binaries. Seven input files and six
small reference artifacts are locked in `golden/multi-profile.sha256`.

`ProfilesAdIncoherent.flp` reuses the exact `ProfilesAd.env` bytes. Three new
fixed-oracle runs have identical MOD/SHD; all 13 modes and 24 RMS pressures
compare at unchanged tolerances (local max |dp| `2.3283064365386963e-10`). A
zero-pressure corruption control fails the comparator, replacing the former
nonnegative/real-only check. It runs without Docker daily and through API/actual
CLI HDF5 in [full pre-release acceptance](../development/reference-workflows.md#full-pre-release-acceptance).
`multi-profile.sha256` now has 17 records; the earlier 13 remain unchanged.

The oracle remains AT v2023.5 `475108519289c6fb488b58980c644ea14eccc604`, Linux
amd64 / GNU Fortran 12.2.0, with no modified Fortran. Input aliasing only pairs
one unchanged ENV with each unchanged FLP under a common working stem. The
small fixtures use four profiles: the initial three-profile AD smoke aborted
in upstream FIELD, whose sentinel write uses `rProf(NProf+1)` despite allocation
of `NProf` entries. That failed smoke is not a golden or a numerical pass.

See [reproduction commands](../development/reference-workflows.md),
[HDF5 layout](../guide/kraken-hdf5.md), and
[golden provenance](../../crates/kraken/tests/fixtures/golden/README.md).
[Self-contained JSON](json.md) is implemented as a separate input Adapter;
FIELD3D, BOUNCE creation, ray arrivals and time-domain synthesis remain excluded.

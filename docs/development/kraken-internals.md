# KRAKEN implementation map

The public test surface remains `Case::from_definition`, the legacy/JSON
loaders, `solve`, `solve_complex_modes`, `solve_field` and `solve_frequencies`. Numerical acceptance
is still the [fixed compatibility target](../kraken/compatibility.md); this map
adds no physics or new acceptance requirements.

## Input to validated Case

`lib.rs` exports the Interface; `model.rs` owns unvalidated input types,
`case.rs` owns Case/FIELD invariants, `diagnostic.rs` diagnostics, and
`result.rs` numerical products. Numerical files below live in `solver/`.

- `crates/kraken/src/input/mod.rs` owns legacy acquisition and exact consumed-input
  snapshots; ENV and FLP can own different resource stems.
- `input/legacy/mod.rs` owns records, inheritance, profile/frequency ordering, resource
  selection and source-location diagnostics. Single-environment file loading discovers
  top/bottom tables together, then parses again for assembly (two ENV parses, formerly
  three). Discovery deliberately inspects only the first environment; FIELD discovery
  still scans every profile. Bottom-before-top resource errors and input-size limits
  are unchanged. Reusing parsed state across acquisition/assembly is not part of this
  optimization; FLP still has separate discovery and assembly parses.
- `input/legacy/material.rs` owns **Legacy materials**: raw absorption and power laws
  travel with each fluid, solid or half-space, never in canonical loss fields.
  Its `case_definition` selects one frequency and converts the whole material
  stack using `attenuation::db_per_wavelength`. Storage accounting happens
  before frequency copies. Biological loss uses node depths, but half-space
  conversion retains the reference's HUGE-depth exclusion.
- `input/json.rs` imports canonical definitions directly, without legacy conversion.
- `layers::validate` owns fluid SSP/loss rules, interpolation validation and
  minimum-speed calculation through the existing layer iterator. It retains
  per-layer diagnostic names and aggregate ordering, including existing repeated
  interpolation diagnostics. Valid additional profiles are constructed once
  during Case validation, not once for checking and again for their minima.
- `Case::from_definition` retains topology, spectral, geometry and remaining
  boundary/material checks. Validated Cases stay immutable.

## Validated Case to modes and FIELD

`solver::solve_modes` chooses `modes.rs` (KRAKEN) or `complex_modes.rs`
(KRAKENC). Both use `Profile`, finite-layer meshes and elastic impedances.
`ElasticLayer.material_profile` retains depth-varying cp/cs/density/P/S losses.
Legacy materials convert each node independently at each solve frequency;
`SolidMesh` samples the selected N/C/P/S profile and builds per-node compound
coefficients. Automatic solid meshes use the last input shear speed, while
spectral limits use the sampled minimum. Interpolated density, losses and bulk
modulus are validated before shooting. Real/complex coefficient grouping and
first-mesh normalization remain separate.
Their root searches, precision, work limits and deflation arithmetic remain
separate Implementations. In single lossless-fluid narrow spectra with vacuum
top/fluid A bottom and cHigh ≤ bottom cp, KRAKENC predicts the next initial
guess from the last two root spacings. It reduces actual dispersion evaluations, not their accounting:
original BroadBand/MunkK 500 Hz uses 153M of the unchanged 300M root-work
ceiling. Water-loss/broader-leaky/wide/layered/table/elastic seeds and secant tolerances remain
unchanged; raw Neville seeds still take precedence on mesh three onward.

`refinement.rs` owns **Modal refinement** behind `seed` and `accept`:

- mesh multipliers remain 1, 2, 4, 8, 16;
- raw root history is separate from the Richardson table, and includes
  cLow-excluded roots used for KRAKENC deflation;
- finite-solid mesh two still uses the original scan; Neville seeds start on
  mesh three. Real elastic tops first use Solve1's shared intervals and ZBRENTX,
  then non-deflated Solve2; `selected_count` retains MINLOC's prior-row semantics;
- both backends retain surviving first-mesh shapes/group speeds/loss and
  Richardson columns when the current mesh's search reduces the spectrum,
  including original TLslices `double` (43 to 42). Count increases still reject
  the run: a new mode has no first-mesh shape. No reference-derived count is used;
- KRAKEN extrapolates real k² and retains first-mesh loss; KRAKENC extrapolates
  complex k². Standard arithmetic bounds share bookkeeping, not a new numerical
  trait or plugin Interface.

`solve_frequencies` keeps real Solve2's run-local bound across ordered blocks,
without retaining all results; the first error ends the iterator. Like JSON,
all blocks must share one backend; a mismatched block returns `KR0201` before
numerical work, including for multi-profile blocks. Independent
`solve`/`solve_field` calls reset the bound. HDF5 legacy/JSON execution and API
differential checks share this Interface; no global solver state is used.

FIELD synthesis and multi-profile propagation still own their intentionally
different precision/operation grouping. N² complex-speed interpolation computes
endpoint reciprocals before weighting, as in pinned n2Linear; weighted division
changes cancellation-sensitive deflated search paths. The
[three-layer refinement fix](krakenc-three-layer-refinement.md) preserves existing
sqrt, seed, shooting and stopping paths and reproduces the natural 5-to-4 search.

`tests/refactoring.rs` characterizes the Case/load/solve Interfaces; existing
behavior, JSON round-trip, material, FIELD, CLI/HDF5 and fresh-reference tests
remain the acceptance surface. No helper-only test replaces those checks.

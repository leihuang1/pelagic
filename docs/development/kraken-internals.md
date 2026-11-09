# KRAKEN implementation map

The public test surface remains `Case::from_definition`, the legacy/JSON
loaders, `solve`, `solve_complex_modes`, `prepare_modes`, `PreparedModes::field`, `solve_field` and `solve_frequencies`. Numerical acceptance
is still the [fixed compatibility target](../kraken/compatibility.md); this map
adds no physics or new acceptance requirements.

## Input to validated Case

`lib.rs` exports the Interface; `model.rs` owns unvalidated input types,
`case.rs` owns Case/FIELD invariants, `diagnostic.rs` diagnostics, and
`result.rs` numerical products. Numerical files below live in `solver/`.

- `crates/kraken/src/input/mod.rs` owns the Case intake, legacy acquisition and exact
  consumed-input snapshots; ENV and FLP can own different resource stems.
- `input/legacy/mod.rs` owns records, inheritance, profile/frequency ordering, resource
  selection and source-location diagnostics. Single-environment file loading discovers
  top/bottom tables together, then parses again for assembly (two ENV parses, formerly
  three). Discovery deliberately inspects only the first environment; FIELD discovery
  still scans every profile. Bottom-before-top resource errors and input-size limits
  are unchanged. Reusing parsed state across acquisition/assembly is not part of this
  optimization; FLP still has separate discovery and assembly parses.
- `input/material.rs` owns **Input materials**: raw absorption and power laws
  travel with each fluid, solid or half-space, never in canonical loss fields.
  Its `case` selects one frequency, converts the whole material stack using
  `attenuation::db_per_wavelength` and validates the Case. Storage accounting
  happens before frequency copies. Biological loss uses node depths, but
  half-space conversion retains the reference's HUGE-depth exclusion.
- `input/material.rs` also holds rules both loaders share: which raw rows,
  half-spaces, power laws and volume-loss parameters are acceptable, how media
  classify and stack, how depths snap to fluid interfaces and how many raw values
  one frequency of an environment spends. `case::input_values` is a second counter,
  of canonical values. Each loader keeps its own wording, locations, codes and check
  order. An ENV half-space must also sit at its boundary depth, which JSON cannot
  express. `tests/rejection_parity.rs` pins both inputs' verdicts on the shared
  rules and the known differences below.
- `input/json/` builds the same input materials from a JSON case, so both inputs
  share one loss conversion; `legacy::field_document_with_resources` writes an
  ENV/FLP pair as that JSON, keeping its loss recipe.
- `layers::validate` owns fluid SSP/loss rules, interpolation validation and
  minimum-speed calculation through the existing layer iterator. It retains
  per-layer diagnostic names and aggregate ordering, including existing repeated
  interpolation diagnostics. Valid additional profiles are constructed once
  during Case validation, not once for checking and again for their minima.
- `Case::from_definition` retains topology, spectral, geometry and remaining
  boundary/material checks. Validated Cases stay immutable.

## Known differences between ENV and JSON

The inputs do not accept exactly the same cases. These differences are existing
behaviour. `tests/rejection_parity.rs` pins their load verdicts; changing one needs a
schema decision.

- **Storage budget.** `V` is `Materials::input_values`, counting raw values, and `C`
  is `case::input_values`, counting canonical values, so lossless media cost less in
  `C`. With `P` profiles and `F` frequencies, ENV requires `V*F` within 5,000,000 for
  each profile and `(P + sum(C))*F` within it overall. JSON requires `sum(V)*F` within
  it, and `FieldCase::new` also requires `P + sum(C)` within it at each frequency; only
  JSON reaches that check, and only at `F = 1`. Either input can reject what the other
  accepts. An ENV that only JSON rejects has `P >= 2`. It runs from the CLI and `export`
  prints its document, since `export` does not reload it, but that document fails to
  load with KR0201. Python loads an ENV through that document, so it rejects the same
  ENV outright.
- **Empty biological list.** JSON accepts `"layers": []`. ENV count `0` gets KR0201
  from the generic `Reader::count` bound.

## Validated Case to modes and FIELD

`solver::solve_modes` chooses `modes.rs` (KRAKEN) or `complex_modes.rs`
(KRAKENC). Both use `Profile`, finite-layer meshes and elastic impedances.
`ElasticLayer.material_profile` retains depth-varying cp/cs/density/P/S losses.
Input materials convert each node independently at each solve frequency;
`SolidMesh` samples the selected N/C/P/S profile and builds per-node compound
coefficients. Automatic solid meshes use the last input shear speed, while
spectral limits use the sampled minimum. Interpolated density, losses and bulk
modulus are validated before shooting. Real/complex coefficient grouping and
first-mesh normalization remain separate.
Their root searches, precision, work limits and deflation arithmetic remain
separate Implementations. In single-layer lossless-water narrow spectra with a
vacuum or rigid top, fluid A bottom and cHigh ≤ bottom cp (also ≤ 10 × cLow),
KRAKENC predicts the next initial guess from the last two roots only when the sampled profile has no
inter-duct speed barrier. A decrease followed by an increase in the real mesh
slowness coefficient identifies that barrier (plateaus retain the previous
trend); those separated-duct cases use the original previous-root continuation
seed. This avoids skipping interleaved mode families in Sduct C/P/S. It reduces
actual dispersion evaluations, not their accounting:
original BroadBand/MunkK 500 Hz uses 153M of the unchanged 300M root-work
ceiling. Water-loss/broader-leaky/wide/layered/table/elastic seeds and secant tolerances remain
unchanged; raw Neville seeds still take precedence on mesh three onward.
Without a refinement seed, the eligible predictor's first two roots use bounded
vertical-wavenumber guesses; this avoids skipped rigid-top lossy Pekeris modes.
Complex shape inverse iteration requires a polishing pass before accepting norm
growth (at least two passes, within the existing three-pass ceiling).

### Fluid root search

Real KRAKEN without finite elastic layers or elastic half-spaces always uses
`modes/fluid_roots.rs`. Its invariant is that the fluid positive-pivot count
`C(x)` is nonincreasing in squared wavenumber, and an interval `[a, b]` owns
exactly the root targets `C(b)..C(a)`. Shared bisections isolate one target per
interval. The bottom-up fluid shooting function uses positive power-of-two
rescaling, carrying the exponent into safeguarded secant/Illinois steps. Its
lane kernel keeps the recurrence in registers, tests every lane with one
combined comparison per row and rescales in a cold, out-of-line function. Each
lane performs exactly the scalar recurrence's operations, so results do not
depend on the lane width; a unit test checks this bit for bit. Every
eighth step bisects; refinement stops at relative bracket width `1e-13`, adjacent
floats or a computed zero. Endpoint signs must oppose and be finite. Final
Sturm counts must identify the same single target within one relative tolerance
of the answer (clipped to its isolating interval).

A failed count/sign/finiteness check, a stalled isolation or an iteration cap
leaves the affected mode to the original 64-step full-interval bisection, not a
partial bracket or a guessed root. This includes the rounded rigid/rigid plane
endpoint. Cancellation and the 2.5-billion mesh-work limit remain errors, never
fallback triggers. The search pays the original bisection's fixed charge before
it starts, 64 steps plus the shape allowance per mode, so each mesh costs what it
did before. That charge measures admission, not the search's own operations,
which its isolation, refinement and fallback step caps bound. Because the roots
differ slightly, a solve right at the extrapolation's stopping threshold can
need one mesh more or fewer than before, and so meet the limit differently.
Fixed groups of eight always go through the same ordered parallel map; neither
their membership nor their arithmetic depends on the worker count.
There are no adapter-specific paths or runtime numerical selectors.

This invariant is specific to the fluid recurrence, not a floating-point
monotonicity claim about elastic impedances. The regression witness in
`ElasticHalfBottomC` has counts 3 then 4 at adjacent increasing squared
wavenumbers (`f64::from_bits(4585822887773648427).next_down()` and that value).
Elastic KRAKEN and KRAKENC searches are unchanged. See the
[measurement record](../history/kraken-fluid-roots.md) for numerical changes,
mode-count and thread-invariance checks, and indicative timings.

### Modal refinement and FIELD

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
  complex k². Both retain an explicit complex first-mesh scattering perturbation
  and add it after extrapolation, as described in [rough fluid interfaces](../kraken/roughness.md).
  Standard arithmetic bounds share bookkeeping, not a new numerical trait or plugin Interface.

`solve_frequencies` keeps real Solve2's run-local bound across ordered blocks,
without retaining all results; the first error ends the iterator. Blocks and
profiles whose search does not carry that bound
[can solve together](architecture.md#concurrency-and-cancellation).
`solver::carries_mode_count` decides which: for real KRAKEN it asks
`modes::elastic_search`, the rule the search itself applies, and KRAKENC carries
none. Like JSON,
all blocks must share one backend; a mismatched block returns `KR0201` before
numerical work, including for multi-profile blocks. Independent
`solve`/`solve_field` calls reset the bound. HDF5 legacy/JSON execution and API
differential checks share this Interface; no global solver state is used.
The CLI reaches it through the [frequency-block stream](architecture.md#output-paths),
which writes each block before taking the next result from the iterator; Python
collects the results first. Both locate a block's numerical diagnostics under its
index with `DiagnosticReport::in_frequency`, as `frequencies[i]`, matching the
result's `/frequencies/{i}` group.

`prepared.rs` keeps a real-KRAKEN Case with its `ModeSet`. `PreparedModes::field`
validates new ranges and offsets with the Case rules, then calls the same
`tabulate_field_query` as `solve`, so it adds no separate FIELD arithmetic.

FIELD synthesis and multi-profile propagation still own their intentionally
different precision/operation grouping. N² complex-speed interpolation computes
endpoint reciprocals before weighting, as in pinned n2Linear; weighted division
changes cancellation-sensitive deflated search paths. The
[three-layer refinement fix](../history/krakenc-three-layer-refinement.md) preserves existing
sqrt, seed, shooting and stopping paths and reproduces the natural 5-to-4 search.

`tests/refactoring.rs` characterizes the Case/load/solve Interfaces; existing
behavior, JSON round-trip, material, FIELD, CLI/HDF5 and fresh-reference tests
remain the acceptance surface. No helper-only test replaces those checks.

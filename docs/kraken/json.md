# Self-contained KRAKEN JSON input v1

This is the modern adapter for the **currently supported** KRAKEN/KRAKENC and
2D FIELD workflows, not a new numerical support matrix. It is independent of
[BELLHOP JSON](../bellhop/json.md), which has different physics and fields.
[Complete Pekeris example](../../examples/kraken-pekeris.json).

```console
pelagic kraken export case.env --solver krakenc > case.json
pelagic kraken run case.json --output results/case --format both
pelagic kraken export case.json > canonical.json
```

Legacy export consumes the selected `.env`, `.flp` (same stem or `--flp PATH`),
and required TRC/BRC/IRC/SBP resources. JSON imports and re-exports consume only
the JSON document, even when same-stem files exist. All semantic validation
runs through `Case::from_definition` and `FieldCase::new`; no validated type is
constructed by deserialization. Unsupported combinations stay unsupported.

## Document and units

The root has exactly `schema_version: 1` and `frequencies`, an ordered array of
1..1000 blocks. Each block has:

- `propagation`: `range_independent`, `adiabatic` or `coupled`;
- `profile_ranges_m`: one range per profile, increasing from zero;
- `profiles`: complete, unvalidated [`CaseDefinition`](../../crates/kraken/src/model.rs)
  objects, sharing that block's frequency, engine and FIELD geometry.

A range-independent block has one profile at zero. Multiple-frequency blocks
are explicit; descending and repeated Hz values are retained. All blocks must
use the same `mode_solver` (`kraken` or `krakenc`). Geometry, profile count,
propagation and title may vary between frequency blocks. No sorting, frequency
scaling of materials, inheritance, file references or templates are implicit.
Execution uses `solve_frequencies`: real finite-solid KRAKEN retains Solve2's
search bound across ordered blocks. Independent `solve`/`solve_field` calls start
new runs; the same canonical case can consequently have a different searched
spectrum. This does not change or cumulatively rescale its material losses.

The optional `interface_roughness_m` profile field defaults to an empty (smooth)
vector. A nonempty vector has one nonnegative RMS height per fluid interface;
see [rough fluid interfaces](roughness.md). It is canonical geometry, not a
frequency-dependent attenuation unit.

All object fields are strict, including nested media, boundaries and complex
numbers. Unknown/duplicate fields, unknown enum values and missing required
fields fail as malformed input (`KR0103`). `mesh_reference_frequency_hz` is
optional/null; null uses the solve frequency. Enum strings use `snake_case`:
`n2_linear`, `c_linear`, `pchip`, `spline`, `analytic_munk`; `line`, `point`,
`scaled_cylindrical`; `coherent`, `incoherent`. Shared physical validators
report the JSON path and indexed field, e.g.
`frequencies[1].profiles[0].mesh_points`. Syntax errors also carry line/column.
Unsupported schema versions are `KR0202`; semantic failures retain existing
`KR0201`/`KR0202` diagnostics.

Distances are metres, speeds m/s, frequencies Hz, and densities **g/cm³**, as
explicitly named by `_density_g_cm3`/`density_g_cm3`. Source-pattern angles are
degrees and `amplitude` is linear pressure, not dB. Reflection-table phases are
**radians** (`phase_radians`), not the legacy table's degrees. Mode/FIELD depth
arrays are separate; JSON retains supplied f64 values without legacy f32 vector
rounding or interface alias normalization. Export preserves those already
resolved legacy values; floating-point round-trip parsing is enabled.

**Losses are canonical solve-frequency dB/wavelength**, including effective
material and volume attenuation at each SSP node. The legacy N/M/m/F/W/Q/L and
T/F/B conversion remains in the legacy loader; export stores its resolved
values, not the original loss-unit recipe. Empty node-loss arrays are lossless.
Changing a block's `frequency_hz` alone does not recompute losses; supply the
material losses for the new frequency. `mesh_reference_frequency_hz` retains
nominal mesh-frequency scaling independently, including automatic meshes.

The existing first-fluid water fields, ordered `additional_fluid_layers`, and
ordered top/bottom `*_elastic_layers` retain their Rust meanings. Each elastic
layer may include a `material_profile` of complete depth/cp/cs/density/P-loss/S-loss
samples using the `ElasticMaterialPoint` field names. Both endpoints are required,
depths strictly increase, and the first sample must match the scalar material.
Omitted or empty profiles retain homogeneous material. Node losses are already
canonical for the block frequency; JSON does not reapply legacy loss conversion.
Absolute depths and canonical shear/compressional losses are not converted into
fictitious fluid properties. The same elastic, table, analytic-profile and
coupling-grid restrictions apply as for direct Rust cases.

## Inline boundaries and patterns

Boundary objects use a `type` discriminator and `data` only for payloads:

```json
{"type":"vacuum"}
{"type":"rigid"}
{"type":"fluid_half_space"}
{"type":"elastic_half_space","data":{"shear_sound_speed_mps":900.0,"shear_attenuation_db_per_wavelength":0.1}}
{"type":"reflection","data":[{"angle_degrees":0.0,"magnitude":1.0,"phase_radians":0.0},{"angle_degrees":90.0,"magnitude":1.0,"phase_radians":0.0}]}
{"type":"impedance","data":{"frequency_hz":50.0,"points":[{"wavenumber_squared":0.02,"f":{"real":1.0,"imaginary":0.0},"g":{"real":0.0,"imaginary":1.0},"power":0},{"wavenumber_squared":0.03,"f":{"real":1.0,"imaginary":0.0},"g":{"real":0.0,"imaginary":1.0},"power":0}]}}
```

These are boundary fragments, not complete valid cases. A-boundary
compressional speed/density/loss remain in the enclosing `surface_*` or
`bottom_*` fields; non-half-space boundaries require those fields to be zero.
Impedance frequency must equal the block frequency. F/P restrictions and
interpolation/power bounds remain unchanged.

`source_pattern` is empty for omnidirectional sources or an inline ordered
angle/amplitude array. There is no `.sbp` lookup or second dB conversion.
First-source-block shading and reference coherent/incoherent conventions are
unchanged.

## Bounds, CLI and output

JSON input is at most 1 MiB, with the same per-case vector/medium/work limits and
5,000,000 cumulative case-vector/range entries. Canonical exports use compact
JSON and a terminal newline. They serialize fully before stdout: validation or
size failures emit no partial document. An expanded legacy case may exceed the
JSON byte ceiling even when compressed legacy vectors fit; export rejects it,
rather than producing an unreadable document. The library `export_case_document`
also checks compact serialized bytes using a fixed 1 MiB sizing buffer, including
UTF-8 and JSON escaping; compact output exactly at the limit remains valid. The
CLI separately checks its additional terminal newline. Pretty-printing or
modifying a returned document can increase its size and requires revalidation.
Explicit frequency blocks
intentionally duplicate geometry; templates are not part of v1.

Legacy `--solver` still defaults to KRAKEN. For JSON, the engine is in the
document; an explicit `--solver` must match and never overrides it. `--flp` is
invalid for JSON. Output defaults, overwrite, input-alias protection, quotas,
scratch ownership, sequential frequency solving and atomic publication use the
same [HDF5 v1 adapter](../guide/kraken-hdf5.md). Exit codes stay 0/2/3/4.

JSON HDF5 results have `/inputs/json` with supplied filename, exact byte count
and SHA-256 of the **parsed snapshot**, including whitespace. They do not claim
ENV/FLP/auxiliary provenance or hash a re-exported document. Frequency groups
add `title` metadata for the first profile of each block. Existing dataset
paths/types and schema version remain unchanged; BELLHOP v3 is unaffected.
`output::kraken::run_json` is the equivalent Rust output adapter.

## Validation

Ordinary tests round-trip all loadable curated legacy fixtures exactly, check
nested strictness, schema/version/size/collection bounds and frequency order,
and compare actual legacy/relocated JSON CLI HDF5 datasets. Same-stem files are
poisoned to prove they are ignored. Output preservation covers malformed input,
engine/FLP conflicts, quotas, unowned scratch, input/symlink aliases and numerical
failure after a completed first frequency.

[Local full pre-release acceptance](../development/reference-workflows.md#full-pre-release-acceptance)
reuses twenty-four fresh-reference workflows: WaterLossPower,
FieldPattern, FluidTrcC, LayeredFluidPower, complex LayeredFluidThreeWideRefined,
both engines for original TLslices
`double`, FiniteElasticStack, real FiniteElasticTopN/ShearOnly/Power, real
elastic-half-space TopN/BothS/TopBroadband, graded BottomN/TopS/Power/Bio,
original TabRefCoef BRC/IRC, both engines for original BroadBand/MunkK at
50/500 Hz, and original Gulf AD/CM. Exported
JSON definitions must equal the complete legacy definitions; JSON API results
and actual JSON CLI HDF5 then compare every mode/shape/pressure with the same
fixed `.mod/.prt/.shd` references and unchanged tolerances. JSON/HDF5 files are
not new Fortran goldens. HTTP, time-domain products and extra numerical option
combinations are not included in this capability block.

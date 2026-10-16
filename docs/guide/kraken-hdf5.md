# KRAKEN/KRAKENC CLI and HDF5 schema v1

This is a product adapter for the **currently accepted numerical subset** in
[kraken-compatibility.md](../kraken/compatibility.md), not completion of its planned
matrix. It supports legacy `.env/.flp` or [self-contained JSON](../kraken/json.md),
KRAKEN or KRAKENC, and discrete single/multiple-frequency modes plus supported
single/multi-profile FIELD. HTTP, modes-only CLI runs, reflection-table
generation, ray arrivals and time-domain synthesis are
not provided. Original BroadBand/MunkK passes complete 50/500 Hz modes and
FIELD through both engines and legacy/JSON CLI-HDF5 at unchanged work ceilings.

## CLI

```console
cargo run --release -p cli -- kraken run path/to/case.env --format hdf5 --output results/case
cargo run --release -p cli -- kraken run path/to/case.env --solver krakenc --flp geometry.flp --format both --output results/complex
```

The installed computation binary is `pelagic`; `cargo install --path crates/cli`
builds it using the existing static HDF5 dependency. See the [CLI guide](cli.md).

- Legacy `--solver kraken|krakenc` defaults to `kraken`; the engine is not guessed.
  JSON declares its engine; an explicit `--solver` must match it. `--flp` is
  invalid for JSON, which never resolves auxiliary files. `pelagic kraken export` emits
  a complete canonical document without running the numerical solver.
- Both backends accept 1..500 contiguous constant-density fluid layers with
  independent meshes, N/C/P/S interpolation and material/volume attenuation.
  Density/speed/loss may jump at interfaces; density gradients and finite-layer
  shear inside fluid media remain unsupported. Finite solid caps are
  separate materials; analytic Munk and F/P/TRC combinations remain single-layer.
  Mesh/profile/shape/work budgets are totals for the stack, not per layer.
- Both backends accept smooth V/R/A boundaries; real KRAKEN stays trapped
  at each A half-space. KRAKENC supports complex N/C/P/S or lossless fixed-Munk-A,
  with N/M/m/F/W/Q/L material units, optional T/F/B volume loss, and the existing
  bounded refinement/frequency-order contract. Analytic A remains lossless and
  rejects volume addition. Loss conversion is per solve frequency; the HDF5
  schema and exact-input provenance contract are unchanged. Original BroadBand/MunkK
  now passes both frequencies through KRAKENC's unchanged 300M root-work ceiling.
  A derived 7500 Hz third block verifies that late failure still preserves old
  output and removes scratch after 50/500 Hz succeed; no prefix is published.
- N/C/P/S fluid stacks also support [elastic A half-spaces](../kraken/elastic-halfspaces.md):
  top/bottom through both engines. Elastic/table/analytic combinations remain
  unsupported. KRAKEN retains the reference's cs cutoff, top-A compressional
  perturbation and omitted elastic shear/bottom absorption; use KRAKENC for full
  elastic loss. Requested P/S loss and the real/complex model are recorded
  separately in additive HDF5 metadata.
- [Finite solid caps](../kraken/finite-elastic-layers.md) are supported
  above/below contiguous N/C/P/S fluid stacks through both engines. Original
  elsed/ice and depth-varying material profiles pass both engines.
  Sequential real finite-solid frequencies retain Solve2's search bound; they
  are not independent solver runs.
  KRAKEN finite stiffness uses Re(c²) but omits elastic absorption perturbation.
  Modal/FIELD samples stay inside the absolute fluid interval, not in solids.
- KRAKENC `F`/`P` bottoms additionally consume same-stem `.brc`/`.irc`; this
  slice is single-frequency, RMax=0, lossless N/C water and vacuum top. Missing,
  malformed or unaccepted table combinations fail explicitly. Rust does not
  generate tables or infer an IRC solve frequency from its header.
- KRAKENC top F consumes `.trc`: single frequency, RMax=0, lossless N/C,
  blank restart/no B, smooth bottom and cLow >= last SSP-node speed.
  Evanescent top-table roots, top P and simultaneous top/bottom tables are rejected.
- `.flp` defaults to the environment's same-stem file. It remains required:
  `.env` controls modal samples; `.flp` controls FIELD geometry, mode addition,
  source-pattern selection and mode limit. `*` consumes the `.flp` stem's `.sbp`.
- All CLI formats use result directories, default `<case-stem>` in the current
  directory. Select `--format hdf5|both` for `CASE.h5`; default is native output.
- Verified owned products are replaced by default, stale owned products removed,
  and unknown conflicts/inputs/aliases protected. See the
  [native output and file-group contract](native-output.md); no `--overwrite` flag.
- Exit codes: `0` success, `2` input/argument failure, `3` numerical failure,
  `4` output/quota/I/O failure. Numerical errors identify the zero-based
  frequency index and Hz value. Unsupported combinations are rejected; the
  pinned KRAKEN elastic-loss limitation above is explicitly retained.

Exit 0 and successful HDF5 publication certify completion, not fixed-oracle
parity for arbitrary inputs. The former
[wide three-layer refinement exception](../development/krakenc-three-layer-refinement.md) is fixed:
its natural four-mode KRAKENC spectrum and complete legacy/JSON HDF5 products now
pass the pinned reference. This does not add a runtime parity-certification flag.

Rust adapters may call `output::kraken::run_legacy` with input/output
paths, engine, overwrite policy and byte quota, or `run_json` for one JSON
snapshot with an optional matching engine assertion. These existing Rust APIs
retain single-file publication; CLI uses `run_legacy_directory` or
`run_json_directory`, sharing the same HDF5 writer and ordered solve iterator.
The numerical `kraken` crate
has no production HDF5 dependency.

## Resource limits and publication

Input limits are unchanged: 1 MiB per UTF-8 file, 1,000 frequencies, and
5,000,000 total copied case-vector entries. Inputs are read once into bounded
snapshots through `input::load_legacy`, parsed through the existing
`legacy::load_field_cases_with_resources`, and hashed from those exact bytes.
The returned read-only `FieldInput` keeps frequency cases with every consumed
resource role/path/source. `legacy::load_field_cases` delegates to this same
acquisition path, so export and HDF5 do not reassemble resource inventories.
Explicit snapshot parsing and existing single-profile entry points stay intact. JSON instead parses one bounded snapshot through
`json::load_case_document_named` and records only `/inputs/json`; its exact
bytes are hashed, including whitespace. Metadata is never obtained by rereading
an input after solving.

The default cumulative output limit is **268,435,456 bytes (256 MiB)**;
`--max-output-bytes N` accepts a positive override. All FIELD float32 payloads
are admitted against it before numerical work. Numeric dataset payloads are
charged cumulatively before writing; actual HDF5 file size, including metadata,
is checked after the header, each frequency flush, final summary attributes,
and HDF5 close before publication. CLI additionally checks cumulative physical
bytes across all selected native/HDF5 products, report and manifest; see
[the group quota/failure contract](native-output.md).
A rejected pre-installation run publishes nothing and attempts to remove its owned scratch;
a cleanup I/O failure is reported as a warning.

Only one frequency's `SimulationResult` and one large component-write buffer
are retained at a time. Frequency cases themselves retain the loader's bounded
input copies. Existing mesh/mode/shape/FIELD work limits still apply **per
frequency**; this output quota is not a cumulative CPU budget, timeout or
cancellation mechanism. A scratch file can temporarily exceed the physical
quota during the header or one bounded frequency's writes, before its flush check; there is
no hard byte-limited HDF5 filesystem driver.

For the retained **Rust single-file APIs** (not CLI group publication), the
private publication module is shared with BELLHOP; its v3
schema and absence of a KRAKEN byte quota remain separate. KRAKEN payload
accounting and header/profile/frequency flush checks stay in the v1 writer;
the shared publisher checks its existing physical quota again after close.

`<output>.tmp` is exclusively created by HDF5 and is never a truncated
pre-existing file. After all frequencies succeed, HDF5 flush/close and file
sync must succeed. Without overwrite, an atomic hard link refuses a destination
that appeared during execution; with overwrite, an atomic rename replaces the
old output. Scratch/output paths share a directory/filesystem. Filesystems
without the required hard-link/rename support return an explicit output error.
An already existing scratch file is left untouched. Atomic publication is not
an all-platform power-loss durability guarantee for directory metadata.

## Root attributes and axes

This is independent of [BELLHOP schema v3](bellhop-hdf5.md); the common
`output` crate reuses only I/O helpers, not solver result models.

| Attribute | Type / meaning |
|---|---|
| `schema_name` | UTF-8 `kraken` |
| `schema_version` | uint32 `1` |
| `implementation` | Rust implementation/package version |
| `compatibility_reference` | pinned Acoustics Toolbox v2023.5 commit |
| `title` | first profile's title at the first frequency |
| `solver` | UTF-8 `kraken` or `krakenc` |
| `coordinate_convention` | range origin at source; depth positive downward |
| `frequency_count` | uint64, number of input frequencies |
| `mode_count`, `pressure_count` | uint64 totals across all frequencies; modes also total all profiles |
| `max_output_bytes` | uint64 configured quota |

`/frequency_hz` is float64 `[F]`, unit `Hz`, in input order. Descending and
repeated frequencies are retained. `/inputs/env` and `/inputs/flp` have UTF-8
`filename` (the supplied path), uint64 `size_bytes`, and UTF-8 `sha256`
attributes. Bottom F/P adds `/inputs/brc` or `/inputs/irc`; top F adds `/inputs/trc`;
a tabulated source pattern adds `/inputs/sbp`. Each is hashed from the **actual
parsed snapshot**. Unused
same-stem resources are neither read nor recorded. Input contents are not embedded.
These groups and the `surface_boundary`/`bottom_boundary` attributes below are additive schema-v1
metadata; existing dataset layouts, types, units and schema identity are unchanged.
Older v1 files may omit this additive metadata. JSON input instead records only
`/inputs/json` with the same filename/size/SHA attributes, never reconstructed
ENV/FLP/resource provenance.

Results live under `/frequencies/0`, `/frequencies/1`, ... using **indices,
not Hz names**. Every frequency has its own mode count and arrays; no padded
common-mode matrix or cross-frequency mode correspondence is implied.

## Per-frequency metadata

Each `/frequencies/i` group has attributes:

- `title`: UTF-8 first-profile title for this frequency block (additive v1;
  older files may omit it);

- `frequency_hz`: float64 actual solve frequency;
- `mesh_reference_frequency_hz`: float64 nominal frequency; for a non-broadband
  input this equals the actual frequency;
- `requested_mesh_points`: uint64 nominal NG for the first fluid layer, including `0` for automatic;
- `finite_fluid_layer_count`: uint64 number of finite fluid layers (additive v1 metadata);
- `max_range_m`: float64 extrapolation control;
- `field_mode_limit`: uint64 requested FIELD cap (not the total stored mode count);
- `profile_count`: uint64 number of modal environments;
- `field_propagation`: UTF-8 `range_independent`, `adiabatic` or `coupled`;
- `source_geometry`: UTF-8 `line`, `point` or `scaled_cylindrical`;
- `mode_addition`: UTF-8 `coherent` or `incoherent` (additive v1 metadata);
- `source_pattern`: UTF-8 `omnidirectional` or `tabulated`, plus uint64
  `source_pattern_point_count` (additive v1 metadata);
- `surface_boundary`: UTF-8 `V`, `R`, `A` or `F` (additive v1 metadata);
- `bottom_boundary`: UTF-8 `V`, `A`, `R`, `F` or `P` (additive v1 metadata).

An elastic A boundary additionally records optional per-frequency attributes,
with prefix `surface_` or `bottom_`:

- `half_space_material`: UTF-8 `elastic` (the boundary code stays `A`);
- `sound_speed_mps`, `shear_sound_speed_mps`, `density_g_cm3`: float64 material;
- `attenuation_db_per_wavelength`, `shear_attenuation_db_per_wavelength`: float64
  requested losses converted for this solve frequency, not inferred mode losses;
- `elastic_attenuation_model`: UTF-8 `reference_real` (KRAKEN's real impedance,
  top-A compressional perturbation and omitted elastic shear/bottom absorption)
  or `complex` (KRAKENC).

These additive v1 attributes do not change datasets, schema identity or BELLHOP
v3. Older v1 files and nonelastic boundaries may omit them. They are material
provenance, not certification of arbitrary-input numerical parity.

Finite solids additionally record `finite_elastic_layer_count`
(uint64) and `/frequencies/i/elastic_media/{top|bottom}/{index}`. Indices are
ordered from shallow to deep within each side. Each group has `material=elastic`,
`attenuation_model=reference_real_stiffness|complex`, float64 `top_depth_m`,
`bottom_depth_m`, `compressional_sound_speed_mps`, `shear_sound_speed_mps`,
`density_g_cm3`, `compressional_attenuation_db_per_wavelength`,
`shear_attenuation_db_per_wavelength`, and uint64 `requested_mesh_points`.
Requested solve-frequency losses are not inferred modal attenuation. The real
model retains Re(c²) stiffness without elastic absorption. These additive v1
attributes/groups may be absent in older files. `media` remains **fluid only**;
its first top depth can now be nonzero below a solid cap. Existing datasets,
schema identity and BELLHOP v3 are unchanged.

Each elastic layer also records uint64 `material_profile_point_count`. Zero means
homogeneous material and no profile group. Otherwise `material_profile/` contains
six float64 one-dimensional datasets of that length: `depth_m` (m),
`compressional_sound_speed_mps` and `shear_sound_speed_mps` (m/s), `density_g_cm3`
(g/cm^3), and `compressional_attenuation_db_per_wavelength` and
`shear_attenuation_db_per_wavelength` (dB/wavelength). Scalar material attributes
remain the first sample. Datasets retain the input samples, not a solver mesh;
they carry `unit` attributes and consume the existing output-payload budget.
These are additive KRAKEN schema-v1 metadata; older files may omit them.

Each frequency also has `/frequencies/i/media/0`, `/1`, ... in depth order.
Each layer group carries float64 `top_depth_m`, `bottom_depth_m`, `density_g_cm3`,
`top_roughness_m`, `bottom_roughness_m`, and uint64 `requested_mesh_points`.
Roughness is RMS height in metres (zero for smooth interfaces); adjacent layers
repeat their shared interface height. These are finite layers, not the A half-spaces;
nominal NG is not the refined/solve-frequency mesh. All depths are absolute.
Older v1 files may omit these additive groups/attributes; datasets and schema
identity are unchanged. Modal samples and FIELD depths may span the whole stack.

## Modes

Under `/frequencies/i/modes`, let `M` be the first profile's mode count at that
frequency and `D` its modal sample-depth count. Each later profile uses the
same mode layout with its own `M` and `D`. All datasets are float64 and have a `unit`
attribute:

| Dataset | Shape | Unit |
|---|---|---|
| `sample_depth_m` | `[D]` | `m` |
| `horizontal_wavenumber_real`, `horizontal_wavenumber_imaginary` | `[M]` | `rad/m` |
| `phase_speed_mps`, `group_speed_mps` | `[M]` | `m/s` |
| `attenuation_nepers_per_m` | `[M]` | `neper/m` |
| `eigenfunction_real`, `eigenfunction_imaginary` | `[M,D]` | `reference_normalized` |

`eigenfunction_axis_order` is `mode,sample_depth`. `normalization` identifies
the pinned AT density-weighted normalization, including the fluid/elastic
boundary-admittance derivative where present; eigenvectors retain their arbitrary
unit phase. Samples are fluid pressure, not solid displacement or strain. The
`reference_normalized` unit is not an absolute pressure unit. Stored shape
values preserve the Rust result, including its reference complex32 sampling
rounding; float64 storage does not restore precision already rounded away.
Shapes and group speeds remain first-mesh values, even when wavenumbers are
Richardson-extrapolated. Attenuation is `-Im(k)`; neither imaginary component
nor phase convention is changed during serialization.

## FIELD

For multi-profile output, `/frequencies/i/profile_range_m` is float64 `[P]`
(unit `m`), and `/frequencies/i/profiles/j` records UTF-8 `title`, float64 `range_m`,
the same profile/material metadata listed above, and a complete `modes` group.
`profiles/0/modes` is a hard link to `/frequencies/i/modes`, preserving the old
first-profile path without duplicating its payload. All later profile modes are
stored separately. One pressure field, not one per profile, is synthesized.
These additions retain schema v1; single-profile dataset paths/types are unchanged.
Files are quota-checked after each profile flush as well as each frequency.

Under `/frequencies/i/field`, geometry is float64:

| Dataset | Shape | Unit |
|---|---|---|
| `source_depth_m` | `[S]` | `m` |
| `receiver_depth_m` | `[Z]` | `m` |
| `receiver_range_m` | `[R]` | `m` |
| `receiver_offset_m` | `[Z]` | `m` |
| `pressure_real`, `pressure_imaginary` | `[S,Z,R]`, float32 | `1` |

`pressure_axis_order` is `source_depth,receiver_depth,receiver_range`, with
range varying fastest in row-major storage. It is **not** the flattened,
range-major BELLHOP field layout. Offsets are separate per-depth values; the
single-profile solver's effective range is range plus offset. Pressure is relative complex
pressure, not calibrated pascals. Incoherent FIELD retains the pinned
`EvaluateMod` complex square-root convention for single-profile runs; adiabatic
FIELD uses `EvaluateADMod`'s real nonnegative RMS convention. Coupled/incoherent
input is rejected. Multi-profile evaluators retain but do not apply receiver
offsets, matching the pinned reference. Pressure storage remains single precision;
arithmetic preserves each pinned evaluator's mixed-precision stages. Writing adds no new numerical rounding.

## Acceptance

Ordinary tests run the CLI, verify schema/types/units/metadata, compare all
serialized values exactly with the Rust result, retain duplicates and varying
mode counts, and exercise overwrite/input-alias/scratch protection, input bounds,
physical/payload quotas and failure after a successful first frequency.
HDF5 schemas and numerical assertions are unchanged; CLI tests now use result
directories and verify complete native readback alongside the same HDF5 checks.

[Local full pre-release acceptance](../development/reference-workflows.md#full-pre-release-acceptance)
additionally passes CLI-produced `.h5` files back through the
same strict `.mod/.prt/.shd` comparator (`KRAKEN_HDF5_RESULT`): both derived
Pekeris broadband pairs, original single-frequency MunkK, all five original
KRAKENC pairs, and original BroadBand/MunkK (both engines/frequencies,
1,003,002 pressures per engine), plus four small table derivatives and all three original TabRefCoef
geo/brc/irc workflows with BOUNCE-generated resources, four cubic/analytic
KRAKENC derivatives, broadband PCHIP Munk and original MunkS/MunkAnalytic
environments with **derived** FIELD geometry, 27 water-material workflows and
24 derived smooth-boundary/TRC workflows, 43 derived layered-fluid workflows
(52 frequency blocks, 539 modes, 5,028 pressures), two original TLslices `double`
workflows (84 modes, 1,002 pressures), and the three fixed
single-profile FIELD-extension workflows. The multi-profile checkpoint adds
two small derivatives and both byte-original Gulf AD/CM paths (24 profile blocks,
1,020 modes, 1,003,050 pressures), comparing every profile's modes and every
pressure via actual CLI HDF5. Layer metadata, fractional
interfaces, cross-layer sources/receivers, repeated frequencies and cumulative
budgets are checked; count-increase rejection and a later-frequency root-work
failure preserve old output and remove scratch. Every mode, shape and
pressure is checked at the existing tolerances. The table derivatives have new
Fortran goldens; no numerical tolerance is changed. Rust HDF5 artifacts
are not committed as Fortran goldens.

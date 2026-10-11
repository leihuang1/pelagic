# Numerical reference

**Execution policy:** ordinary Linux/Windows CI runs real Rust solvers against
committed goldens. The historical fresh-Fortran CI matrix documented below now
runs locally **before every release**, via
[`tools/reference/full-acceptance.sh`](../../tools/reference/full-acceptance.sh).
Its case lists and tolerances are retained; see [workflow/report requirements](reference-workflows.md#full-pre-release-acceptance)
and [daily testing](testing.md). Fresh comparisons are no longer a PR job.

The compatibility oracle is the official Acoustics Toolbox release `v2023.5`:

- Repository: <https://github.com/oalib-acoustics/Acoustics-Toolbox>
- Tag: `v2023.5`
- Commit: `475108519289c6fb488b58980c644ea14eccc604`
- License: GPL-3.0

Small BELLHOP numerical goldens are committed under
`crates/bellhop/tests/fixtures/golden`. Their per-file provenance, hashes,
compiler, target, and flags are recorded alongside them. KRAKEN has seventeen
constructed single-frequency fluid cases (twelve lossless, five with bottom loss) and raw
`.mod/.shd/.prt` goldens with
[provenance and hashes](../../crates/kraken/tests/fixtures/golden/README.md).
`tools/reference/compare-kraken.sh CASE.env` regenerates reference outputs and
compares mode counts, coordinates, wavenumbers, attenuation, phase/group speeds,
phase-aligned shapes, and every complex pressure sample. The same comparator
runs without Docker on committed goldens; local full acceptance also compares
fresh outputs for all seventeen cases and the unmodified upstream `MunkK.env`/
`.flp` (102 modes, 501,501 complex pressures). Local full acceptance also
compares the unmodified upstream `MunkAnalytic.env` with a separately derived
coherent FIELD `.flp` (102 modes, 25 pressures); its original three-line `.flp`
fails in pinned v2023.5 FIELD.
The unmodified official sduct has leaky modes and is **not** covered by the
derived KRAKEN trapped-mode comparison; its separate KRAKENC original-pair
comparison is described below. Measured errors and provenance accompany the goldens.
Separate KRAKENC regressions compare the **derived**
`PekerisComplex.env`, its blank-restart and slower-fluid-bottom variants
(four modes each), a `cLow=1550 m/s` blank-restart variant (three in-band
modes), a three-mesh Richardson derivative (four modes and nine pressures),
two-point graded `N`-profiles in both directions (four modes each),
and a **derived**, truncated seven-knot MunkLeaky water column with and without
0.8 dB/wavelength bottom loss (30 modes, nine leaky each) against pinned
`.mod/.prt` goldens and local full-acceptance output. A third seven-knot
derivative changes only `N` to `C` interpolation (30 modes). Separately derived
coherent `.flp` inputs compare nine complex line-source pressures for
`PekerisComplexBlank` and 36 point-source pressures for each of the two lossy
seven-knot cases with committed `.shd` goldens and local full-acceptance output.
Local full acceptance compares unmodified `MunkKleaky` (329 modes, 150,801
pressures), `MunkKwb` (63, 150,801),
`MunkKbb` (102, 150,801), `sductK` (1,077, 201,201) and Richardson-refined
`calibK` (33, 101,101) `.env/.flp` pairs end to end against `.mod/.prt/.shd`.
The `.env` determines modal samples; the
`.flp` determines FIELD geometry. A metamorphic
frequency/depth-scaling test checks that small distinct squared-wavenumbers
remain separate; it does not claim a fresh Fortran acceptance for scaled
inputs.

KRAKENC cubic/analytic profile acceptance reuses the lossless `Profile`, adding
**derived** `MunkLeakyPartialP/S` (30 modes including 9 leaky, 36 pressures each),
`PekerisComplexSpline3` (4 modes, 9 pressures, meshes 1/2/4), and
`MunkAnalyticComplex` (102 modes, 25 pressures, NG=2003, eleven modal depths and
meshes 1/2/4). A **derived** broadband PCHIP Munk has NG=803, 75/50/62.5/50 Hz,
45/30/37/30 modes and 36 pressures per block. Three pinned runs of all five
produce identical `.mod/.shd`; raw goldens and input/output hashes are recorded.
Analytic f32 grid-step rounding is shared by both Rust mode backends.
Unmodified `tests/Munk/MunkS.env` (a SCOOTER environment) and `MunkAnalytic.env`
also pass through KRAKENC, 102 modes/25 pressures each, with a **separately derived**
coherent `.flp`. Original three-line `.flp` fails in pinned FIELD for both;
this is not original-pair or SCOOTER acceptance. The material checkpoint below
adds complex N/C/P/S water. The smooth-boundary checkpoint adds V/R/A at either
end; P/S/A table-boundary combinations remain excluded.

Water-material acceptance uses fourteen **derived** pairs: N/C/P/S complex
water, N/M/m/F/W/Q/L material units, T/F/B volume attenuation, overlapping
biological layers, and repeated/unsorted refined frequencies. Thirteen pairs
run through both KRAKEN and KRAKENC; the leaky PCHIP partial-Munk derivative
runs through KRAKENC only: 27 workflows, 36 frequency blocks, 404 modes and
432 pressures. Three pinned runs yield identical `.mod/.shd`; all samples pass
local API and actual CLI-HDF5 comparisons at unchanged tolerances. Raw goldens,
hashes and derivation details are committed; local full acceptance is configured
for all 27.
Biological attenuation is sampled at SSP knots, not mesh nodes, and excluded
from half-spaces as in `UpdateHSLoss` (HUGE depth). This is not original upstream
VolAtt acceptance and does not add physical density gradients or layers.

Discrete multi-frequency regressions use two **derived** Pekeris `.env/.flp`
pairs (KRAKEN and KRAKENC): unsorted 75/50/62.5 Hz, fractional mesh scaling,
three refinement levels, `W` bottom loss, and nine pressures per frequency.
Committed `.mod/.prt/.shd` and local full acceptance compare every frequency
block. Local full acceptance also compares the **unmodified**
`tests/BroadBand/MunkK.env/.flp` through both engines: 102 modes at 50 Hz,
1,023 at 500 Hz, and 501,501 pressures at each frequency (1,003,002 per engine).
Three pinned runs yield identical `.mod/.shd`.
API and actual legacy/JSON CLI-HDF5 comparisons check the complete products.
The reference
reader checks frequency order, per-frequency mode counts and offsets, shapes,
printed/binary wavenumbers, group speeds and every pressure; a later-frequency
corruption regression ensures no frequency is skipped. These are frequency-
domain results, not a wideband time-domain acceptance. KRAKENC's narrow,
single lossless-fluid spacing predictor (cHigh ≤ bottom cp) brings 500 Hz to
152,546,495 work, below the unchanged 300M ceiling; max |dp|=1.5360e-8.
WRITE-only reference tracing leaves MOD/SHD bytes unchanged. Other seed paths,
secant tolerance, deflation, inputs and goldens are unchanged. A local
full-acceptance CLI regression separately derives a 7500 Hz third block to
verify exit 3, no partial
publication, preservation of old output and scratch cleanup after 50/500 Hz
succeed. That rejection is protection evidence, not numerical acceptance.

Smooth-boundary acceptance adds eleven **derived** pairs: all nine V/R/A top/
bottom combinations with spline water loss, a lossy rigid-rigid plane mode with
NG=68, and a radiating air top through KRAKENC only (21 workflows). Three more
**constructed** top-TRC derivatives with N/C water, cLow=1550 and A/R bottoms
add three workflows; top tables require a propagating spectral window, no
refinement/B and no simultaneous bottom table. All 24 `.mod/.shd` workflows are
byte-identical in three pinned runs and have committed goldens, hashes and
API/actual-CLI-HDF5 differential. These are not original TRC input-pair acceptance.
Automatic NG=17 rigid-plane refinement changes its mode count and remains an
explicit numerical failure, not a truncated result. An evanescent top-TRC
experiment produced a growing reference root and is explicitly outside support.

TabRefCoef acceptance adds four **derived** 50 Hz N/C single-water cases with
**constructed** BRC/IRC tables and committed `.mod/.prt/.shd`. Original
`neggradC_geo`, `neggradC_brc`, `neggradC_irc` `.env/.flp` remain unchanged:
56/54/42 modes and 50,601 pressures each, including FIELD at 0 m outside the
1..100 m modal sample interval. The upstream directory contains no tables.
`tools/reference/prepare-tabref.sh` uses source-rebuilt pinned BOUNCE (the bundled
executable/object are removed) to generate `.brc/.irc` from original `neggradB.env`,
then runs KRAKENC/FIELD three times. Tables and `.mod/.shd` must be byte-identical;
input/resource/output hashes are retained. BRC/IRC/geo are compared to their own
reference, not assumed equivalent. Pinned GNU real-component grouping prevents
IRC secant from selecting a different subset of roots; this is not a proof of
mathematical root completeness. Table multi-frequency/refinement and Rust BOUNCE
remain excluded.

The [KRAKEN CLI/HDF5 adapter](../guide/kraken-hdf5.md) is independently checked
through its actual serialized output: `KRAKEN_HDF5_RESULT` makes the same test-only
comparator read `.h5` modes and FIELD instead of recomputing the in-memory
result. Local full acceptance exercises both Pekeris broadband derivatives,
original MunkK, all five original fluid KRAKENC pairs, original BroadBand/MunkK,
four table
derivatives, the three original TabRefCoef workflows, four cubic/analytic
derivatives, the broadband PCHIP derivative and the two unmodified MunkS/analytic
environments with derived FIELD geometry through this path,
with every frequency/sample and unchanged tolerances. Schema/metadata/units,
exact Rust-value round-trips and file/quota failure protections also run without
Docker. Existing BELLHOP schema-v3 tests remain unchanged; Rust `.h5` output is
not a Fortran golden.

BELLHOP ray trajectories
cover all `N/C/P/S/Q/A` sound-speed models. Eigenray and arrival goldens cover
Cartesian and ray-centered geometric-hat beams, Cartesian geometric-Gaussian
beams, caustics, arrival combination, and multi-depth/multi-range receiver
grids. Pressure-field goldens cover coherent, semi-coherent, and incoherent
scaling, simple-Gaussian beams, and Cartesian/ray-centered Cerveny beams.
Dedicated reflection goldens cover acousto-elastic, grain-size, and `.irc`
impedance-table amplitude and phase. Additional small fixed-Linux cases cover
coherent hat and incoherent Gaussian **line sources**, Thorp and overlapping
biological losses (33 pressures each), beam-shift trajectory coordinates and
all simple-Gaussian eigenray coordinates. Option-removal controls ensure the
line/loss/shift options actually affect the result. These are representative
paths, not acceptance of every beam/option combination.

Layered-fluid acceptance adds twenty-two **derived** pairs, twenty-one through both
engines and a leaky broadband pair through KRAKENC: 43 workflows, 52 frequency
blocks, 539 modes and 5,028 pressures. Each `.mod/.shd` is byte-identical in
three pinned runs. The comparator traverses every fluid medium record and every
frequency/mode/FIELD sample, through both API and actual CLI-HDF5; local maximum
pressure error is 2.64e-9 at unchanged tolerances. Offline goldens and layer
metadata/budget/failure regressions run without Docker.

`OriginalLayeredDouble.env` is byte-identical to upstream `tests/TLslices/double.env`;
its `.flp` is the official shared `fieldbat.flp` selected by upstream `runtests.m`.
Both pinned engines and Rust find 43 then 42 roots on meshes 1/2. The two
original workflows now pass every declared mode and all 501 pressures per engine,
through API and actual legacy/JSON CLI-HDF5, with max |dp|=1.1824e-11.
Refinement keeps surviving first-mesh data and rejects count increases; it does
not use oracle counts. MOD/SHD are byte-identical in three unmodified runs;
`golden/original-double.sha256` records two inputs and six new artifacts.
`LayeredDoubleRefined` retains doubled NG=200/400/400 as a separate derivative. `LayeredNormalization` removes shear only
from the original `normal.env` bottom and is also explicitly derived. Original
`normal/flused` now pass the half-space checkpoint and `elsed/ice` the finite-cap
checkpoint below; Gulf's sequence now uses multi-profile
FIELD. None is claimed as accepted by the fluid-stack stage.
`LayeredFluidThreeWideRefined` changes only the wide fixture's RMax to 1000 km:
KRAKENC now naturally finds 5 then 4 roots via pinned N² reciprocal-before-weighting.
All four modes and 63 pressures pass API and actual legacy/JSON CLI-HDF5; KRAKEN
independently passes all five modes and 63 pressures. Three unmodified runs per
engine agree; `golden/layered-refinement.sha256` locks the two inputs and six
artifacts. See the [fix and historical diagnosis](krakenc-three-layer-refinement.md).

Elastic-half-space acceptance adds 16 derived and three original input pairs:
37 workflows, 49 frequency blocks, 473 modes and 5,715 pressures, through API and
actual CLI/HDF5. The original scholte/normal/flused environments and official
shared fieldbat.flp are byte copies of the pinned TLslices sources; no shear is
removed and NG is unchanged. All .mod/.shd agree in three runs; local maximum
pressure error is 6.67e-8 at unchanged tolerances. The 149-record
`golden/elastic-halfspace.sha256` manifest and local full-acceptance source-byte comparisons
lock provenance. Real elastic tops retain shared isolation/Brent, non-deflated
Solve2 and MINLOC selection; failed slow-interface and 75 Hz broadband probes
remain unaccepted. Original elsed/ice pass the distinct finite-cap checkpoint.
See the [scope and evidence](../kraken/elastic-halfspaces.md); no new parity waiver.

Homogeneous finite elasticity adds 25 pairs (23 constructed/derived, two
unmodified originals): 50 workflows, 56 blocks, 933 modes, 10,536 pressures,
triplicate identical MOD/SHD, full API and actual CLI/HDF5 comparisons at unchanged
tolerances (local max |dp| 5.96e-8). Original elsed/ice and official fieldbat.flp
retain every byte and NG; the 200-record `golden/finite-elastic.sha256` locks
provenance. Both engines support contiguous layered fluids. Real finite-solid
SSP-node rounding and Solve2's ordered search bound now close the former TopN,
ShearOnly and Power failures; the existing parity exception is not extended. See [finite-cap scope](../kraken/finite-elastic-layers.md).

Depth-varying finite elasticity adds 12 derived pairs / 23 workflows / 29 frequency
blocks / 158 modes / 1,827 pressures, with all declared results compared through
API and actual legacy/JSON CLI-HDF5. The 93-record `golden/graded-elastic.sha256`
locks all new input/artifact bytes; triplicate MOD/SHD output is identical.
Maximum pressure error is 2.08250058582033e-9 with unchanged tolerances. Complete
cp/cs/density/P/S-loss profiles and per-frequency power-law/biological conversion
are retained. See [material sampling and evidence](../kraken/finite-elastic-layers.md#depth-varying-material-evidence).

The single-profile FIELD checkpoint adds three explicitly derived representative
pairs, without expanding them into an option Cartesian product: `FieldScaled`
through KRAKEN (3 modes/9 pressures), and `FieldPattern` / `FieldIncoherent`
through KRAKENC (4/9 each). The pattern case consumes a committed seven-point
`.sbp`. Three pinned runs produced byte-identical MOD/SHD for every pair; all
modes, shapes and pressures pass direct and actual CLI-HDF5 comparison at the
unchanged tolerances, with local maximum pressure error 3.34e-8. Inputs and
reference outputs are locked by `golden/single-profile-field.sha256`; Rust HDF5
is not a golden. Multi-profile FIELD is covered by the separate checkpoint below.

The multi-profile FIELD checkpoint covers two small four-profile derivatives
and both byte-original Gulf AD/CM workflows: 24 profile blocks, 1,020 modes and
1,003,050 pressures. Full API and actual CLI/HDF5 comparisons pass unchanged
tolerances, with local maximum |dp| `4.1159031748919954e-10`. All four MOD/SHD
pairs are byte-identical across three pinned runs. Seven inputs (including the
three original Gulf files) and six small reference files are SHA-locked in
`golden/multi-profile.sha256`; original Gulf binaries are fresh-run artifacts,
not committed large goldens. Local full acceptance checks original input bytes against the pinned
source tree. See [profile propagation](../kraken/field.md).

The [self-contained KRAKEN JSON checkpoint](../kraken/json.md#validation) reuses
its fixed representative references: water power-law loss, source patterns, TRC,
layered-fluid power laws, finite elastic stacks and real TopN/ShearOnly/Power,
real half-space TopN/BothS/TopBroadband, original TabRefCoef
BRC/IRC, BroadBand/MunkK and both Gulf paths. Complete exported definitions are
checked against legacy definitions before JSON API and actual JSON CLI/HDF5
mode/shape/pressure comparisons. The test entry is
`json_fields_match_fresh_reference`; no reference artifacts or tolerance rules
are replaced. Input SHA provenance hashes the parsed JSON bytes, including
whitespace, rather than canonical re-serialization.

## Pinned Linux x86-64 oracle

`tools/reference` provides the reproducible differential environment:

- Debian `bookworm-slim` image index digest:
  `sha256:abd67ffcfa541b485a3dff59865ab629aa048a6c613e639d36e7456b0b229241`
- GNU Fortran `12.2.0`
- flags: `-O1 -ffast-math -funroll-all-loops -fomit-frame-pointer -std=gnu`
- Acoustics Toolbox source archive SHA-256:
  `f8a7a2c1e80a73431cd230a10bef5fcfc996c88889a0e1540771c3922ee2a21f`
- Rust `1.88.0` bookworm image index digest:
  `sha256:af306cfa71d987911a781c37b59d7d67d934f49684058f96cf72079c3626bfe0`

The semantic trajectory comparator aligns isolated minimum-step vertices.
When a value lies within a few ulps of an SSP or boundary interface, one
compiler can take the reference's `1e-4 × base step` while another changes
segment or reflects immediately. Aligned coordinates retain the `1e-5 m`
tolerance; this discrete branch allowance is measured and reported separately.

Pinned Linux comparisons currently show:

- ParaBot: 201 rays, maximum coordinate error `1.4e-11 m`, no branch allowance
- Ellipse: 72 rays, maximum coordinate error `2.2e-11 m`, no branch allowance
- block: 50 rays, exact coordinate agreement
- DickinsBray: 501 rays, aligned error below `9e-10 m`; 16 minimum-step alignment
  operations with maximum branch displacement `0.12 m`

The committed `DickinsCritical` and `ParaBotCritical` cases preserve focused
coverage of the affected SSP-interface and curved-boundary decisions.

## Arrival and pressure differential

`compare-arrival.sh` and `compare-field.sh` extend the same pinned-Linux
procedure to arrival and pressure outputs, parsing the reference `.arr` text
and the direct-access `.shd` records directly.

- `tests/SBCX/sbcx_Arr_asc.env` (official, unmodified): 200 beams over 100
  depths × 500 ranges = 50,000 receivers ≈ 502,000 arrivals. Double-precision
  receiver-grid interpolation changes four receiver counts by one at
  influence-window edges. Matching arrivals differ by at most `3.5e-7` in
  amplitude, `2.0e-5` degrees, and `2.0e-6` seconds. Phase agrees within one
  storage quantum: the reference stores phase as single-precision degrees
  (≈ `2.2e-6` rad grid at large phases) while Rust stores single-precision
  radians, so boundary flips up to `4.2e-6` rad are admitted and reported.
- `tests/Munk/MunkB_Coh.env` (official, unmodified): 501 depths × 501 ranges =
  501,501 coherent geometric-hat pressure samples, maximum absolute error
  `2.0e-10`.
- `MunkB_Semi.env`: `9.3e-10`; `MunkB_Inc.env`: `7.3e-12`;
  `MunkB_Coh_gb.env` (geometric-Gaussian): `7.0e-10`;
  `MunkB_Coh_CervenyC.env`: 100,701 samples, `1.5e-11`;
  `MunkB_Coh_CervenyR.env`: 100,701 samples, `1.8e-12`.
- The committed `ShadedField`, `FreeFGB_grid`, `Field_CervenyF`, and
  `Field_CervenyW` goldens were generated by the same container and match
exactly (source beam pattern, Francois-Garrison attenuation, and space-filling
and WKB Cerveny widths receive numerical coverage through `cargo test`).

All seven official two-dimensional `A/a` environments parsed by the Rust
loader execute with the implemented `G/B` influence models. Three committed
arrival comparisons match receiver-by-receiver arrival counts exactly and
agree at the expected single-precision storage points.

Small pressure grids match the reference single-precision values within
`5e-8` absolute pressure. Full-grid Munk coherent, semi-coherent, incoherent,
geometric-Gaussian, and Cerveny results match the pinned container within
`1e-9` absolute pressure.

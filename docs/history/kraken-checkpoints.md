# KRAKEN/KRAKENC checkpoint history and fixed evidence

See [current capabilities and the unchanged target](../kraken/compatibility.md)
and [legacy input contract and budgets](../kraken/input.md). These records retain
accepted evidence and historical stage boundaries; they are not an alternate
current support matrix. References below to fresh CI describe the historical
execution schedule; the same fresh-reference matrix now runs in
[local full pre-release acceptance](../development/reference-workflows.md#full-pre-release-acceptance),
while ordinary CI retains committed-golden regression.

## Lossy Pekeris and reference-zero comparisons (2026-10-16)

Rigid-top seed selection and complex inverse-iteration polishing now pass the
independent lossy Pekeris controls. Exact reference-zero imaginary k uses a narrow
comparison floor, without clamping production roots. Current matrix outcomes,
independent controls and retained reference differences are recorded only in the
[business coverage record](../development/kraken-business-coverage.md).

## Rough-fluid-interface extension (2026-10-15)

The [roughness checkpoint](../kraken/roughness.md) supersedes the earlier
fluid-roughness exclusions below: both engines now include first-mesh
Kuperman–Ingenito scattering for fluid V/R/A interfaces. Eight original/derived
reference workflows add 14 blocks, 322 modes and 3,510 pressures. Elastic/table
roughness restrictions and the historical scope of earlier evidence remain.

## Separated ducts and broader business diagnostics (2026-10-15)

The [numerical-fix report](../development/kraken-business-coverage.md) records the KRAKENC
separated-duct seed correction (Sduct C/P/S: 194/194/184 modes), independent
Sturm-count/refinement controls and unchanged 500-Hz broadband work ceiling.
At this checkpoint the configured 27-stage diagnostic passed; the broader
345-path inventory retained 7 differential failures and separately classified
unsupported/reference cases. The later Pekeris/reference-zero checkpoint
supersedes those counts, not the retained historical evidence.

## Implementation checkpoints

The capability/checkpoint sections below retain their implementation-stage
boundaries and evidence. Later sections extend earlier slices; an earlier
exclusion is not a claim that a later accepted capability is unavailable.
The final target remains the one recorded at `2a7a658:docs/kraken-compatibility.md`.

The KRAKEN `load_case`/`solve` path currently accepts a narrow legacy
`.env`/`.flp` subset:

- one frequency and 1..500 contiguous fluid layers using shared
  `N` (N²-linear), `C` (sound-speed-linear), `P` (monotone PCHIP), or `S`
  (not-a-knot cubic spline) interpolation; each layer has constant density,
  its own SSP/loss nodes and mesh, with material jumps at interfaces.
  `A` (the fixed 5000 m analytic Munk profile from upstream `misc/munk.f90`)
  remains single-layer and lossless. Density gradients within fluids are rejected; finite solid caps use the separate checkpoint below;
- smooth vacuum (`V`), rigid (`R`) or acoustic fluid half-space (`A`) at either
  end. A boundaries carry material/volume attenuation; V/R have no half-space
  record or material properties. Real KRAKEN requires cHigh no larger than
  either fluid A-half-space speed; KRAKENC also accepts radiating/leaky half-spaces.
  N/C/P/S water additionally accepts elastic A top/bottom through both engines,
  as detailed below. KRAKEN caps elastic cHigh at cs and retains the reference's
  limited loss model (top compressional perturbation, omitted elastic shear/bottom absorption). N/C/P/S water supports
  nonnegative absorption; analytic A remains lossless. Finite solids with homogeneous or depth-varying material outside the contiguous
  fluid interval are supported separately; interleaved solids and rough
  boundaries remain unsupported;
- a line (`X`), point (`R`) or scaled-cylindrical (`S`) source; omnidirectional
  (`O`) or same-stem tabulated (`*`, `.sbp`) pattern; coherent (`C`) or
  incoherent (`I`) mode addition; one range-independent FIELD profile at 0 km;
- finite, in-water source/receiver depths. FIELD normally interpolates the
  `.env` modal samples; it may extend either sampled endpoint by at most
  `1500 / frequency_hz` metres using the nearest two complex32 samples.
  It does not insert a zero-depth sample or clamp to an endpoint. Single-sample
  mode grids cannot extend, and depths outside the water remain rejected.

The solver uses a real acoustic finite-difference mesh with Sturm mode counts,
inverse iteration for mode shapes and group speeds on the first mesh, and up to
five Richardson-extrapolated eigenvalue meshes controlled by `RMax`. A rigid
surface includes the half-weight surface node in the Sturm and inverse-iteration
matrices; a vacuum surface fixes its pressure to zero. A rigid bottom uses a
half-weight terminal node with no half-space contribution. For a constant
profile between two rigid boundaries, the zero-order plane mode lies at the
upper spectral endpoint; the search includes the rounded finite-difference
endpoint. A fluid bottom includes the decaying half-space contribution to
normalization and the pinned
reference's first-mesh perturbation for water and both A-boundary attenuations. Analytic Munk
sampling preserves upstream's f32 intermediate `eps` and grid step even though
its sound speed is stored as f64. Point-source FIELD
uses the reference's `sqrt(k)` modal factor and cylindrical spreading. The
former closed-form Pekeris solver remains **only in tests** as an independent
analytical cross-check. Analytic `A` has no SSP point records, uses density 1,
and requires the upstream formula's 5000 m water column; it is not a general
configurable analytic profile. The parser rejects unaccepted boundary types,
source types, and FIELD options instead of silently changing meaning.

Seventeen constructed fixture pairs cover Pekeris (including a three-point
spline with forced extrapolation, an alternate lossy bottom, derived rigid
surfaces with/without bottom loss, two derived `S` waveguides with a rigid
bottom, and a constant `S` rigid-rigid plane mode at the spectral endpoint), lossless
`N` Munk, lossy-bottom `N` Munk with point-source FIELD, a derived analytic
`A` Munk with fixed-depth formula and `W` bottom loss, and derived trapped
`C/P/S` sduct profiles. The PCHIP sduct derivative also has bottom `W` loss.
**Additionally, the unmodified upstream `tests/Munk/MunkK.env` and `.flp` are
compared in CI:** 102 modes and 501,501 complex pressures. The committed
`MunkBottomLoss` golden is a *reduced-grid derivative*, not the unmodified
input. The analytic `A` golden is also derived. CI additionally compares the **unmodified
upstream `MunkAnalytic.env`** with its automatic mesh and extrapolation (102 modes)
and the separately derived coherent `.flp` (25 pressures). The upstream
three-line `MunkAnalytic.flp` cannot be parsed by v2023.5 FIELD itself, so the
original `.env/.flp` *pair* is not claimed as accepted. The KRAKEN sduct
derivatives still remove the original leaky phase-speed interval; **unmodified**
`sductK` is instead compared through KRAKENC below.
The separate KRAKENC `load_complex_case`/`solve_complex_modes` path currently
supports complex `N/C/P/S` or lossless fixed-Munk-A water, smooth V/R/A boundaries
with material/volume attenuation (including half-spaces slower than the water), trapped
or leaky phase-speed intervals, and non-negative `RMax`. The KRAKENC legacy
reader accepts both blank and dotted restart options. The `.env` supplies modal sample depths; `.flp`
supplies FIELD geometry, used by `solve` for coherent line-/point-source
pressure or ignored by `solve_complex_modes`. The derived
`PekerisComplex.env` extends Pekeris to `cHigh=2000 m/s` and enables Fortran
restarts; `PekerisComplexBlank` uses the default restart setting. A derived
`PekerisComplexRefined.env/.flp` lowers NG to 100 and sets `RMax=1000 km`;
the pinned solver uses mesh multipliers 1/2/4, and all four modes and nine
pressures match committed and fresh reference. As in Fortran, shapes and group
speeds come from the first mesh while complex squared wavenumbers are Richardson
extrapolated. `PekerisComplexSlow` lowers the bottom speed to 1400 m/s.
`PekerisComplexCLow` uses a 1550 m/s lower phase-speed bound and blank
restarts, returning only the three in-band modes (the excluded first root is
still used for deflation). A separate **derived** `PekerisComplexBlank.flp`
verifies nine coherent line-source pressures against committed and fresh
v2023.5 `.shd`; all modes in these cases are compared against committed and
fresh `.mod/.prt`. Scaled depth/frequency regression
checks that duplicate-root detection retains distinct modes. Fluid-bottom
sound speed must be positive for both KRAKEN and KRAKENC. The derived
`PekerisComplexGradient` and `PekerisComplexReverseGradient` change the
water SSP endpoints to 1500→1520 and 1520→1500 m/s respectively, exercising
nonconstant N²-linear water with four modes each, including one leaky mode,
against the same committed and fresh modal comparator. The **derived**
`MunkLeakyPartial` retains the first seven water SSP knots from the original
`MunkKleaky`, truncated at 1000 m: with a lossless bottom, narrower leaky
interval, 800-point mesh and six modal sampling depths, all 30 modes (nine
leaky) match committed and fresh `.mod/.prt`. `MunkLeakyPartialLoss` adds the
original 0.8 dB/wavelength `W` bottom loss; its 30 modes also pass committed
and fresh differential. Complex secant carries the shooting/deflation scaling
exponent into adjacent function comparisons. Counted root work and complex
inverse iteration then cover the **unmodified** `MunkKleaky.env/.flp`:
all 329 modes (including leaky modes) and all 150,801 complex FIELD pressure
samples pass fresh pinned `.mod/.prt/.shd` comparisons. A separate coherent
point-source `.flp` for the **derived** `MunkLeakyPartialLoss` exercises source/
receiver interpolation, positive range offsets and zero range (36 pressures)
against a committed `.shd` golden and fresh CI output. Another **derived**
`MunkLeakyPartialC.env/.flp` changes only that input's water interpolation
from `N` to `C`: all 30 modes and 36 complex pressures match committed and
fresh reference. For wide spectral intervals, complex secant seeds subsequent
roots from the previous root, as in pinned Fortran; the 300-million counted
root-work limit is unchanged. Four other **unmodified upstream** pairs also
pass fresh `.mod/.prt/.shd` comparison in CI:

| Original input | KRAKENC modes | Complex FIELD pressures |
|---|---:|---:|
| `MunkKwb.env/.flp` (cHigh=1551.91 < bottom speed 1600 m/s) | 63 | 150,801 |
| `MunkKbb.env/.flp` (cHigh=bottom speed 1600 m/s) | 102 | 150,801 |
| `sductK.env/.flp` (`C`, cHigh=100000 m/s) | 1,077 | 201,201 |
| `calibK.env/.flp` (`C`, RMax=1000 km; meshes 1/2/4) | 33 | 101,101 |

Repeated pinned runs produce identical `.mod/.shd` binaries for each original
pair; the original artifacts stay out of Git. The sduct FIELD has 216,693,477
modal contributions, below the 550-million-per-frequency work ceiling. Water loss
and smooth complex boundaries are covered by the derived workflows below. KRAKENC `P/S/A` profile evidence is detailed below. Upstream
`tests/PekerisRD` is a BELLHOP, not a KRAKEN, case. The same comparator in
[`tests/differential_reference.rs`](../../crates/kraken/tests/differential_reference.rs)
checks committed `.mod/.shd/.prt` goldens and newly generated pinned Fortran
output in CI; provenance is recorded
[with the goldens](../../crates/kraken/tests/fixtures/golden/README.md).

### KRAKENC cubic and analytic water

The earlier interpolation checkpoint reused the real/lossless `Profile` for
PCHIP (`P`), not-a-knot cubic spline (`S`) and the fixed 5000 m Munk formula (`A`).
Complex N/C/P/S water and additional loss units are added by the material
checkpoint below; analytic A remains lossless. The half-space slice retains
vacuum top, constant density and A bottom, blank/dotted restarts, bounded Richardson refinement and
ordered discrete frequencies. F/P table boundaries remain **N/C only**;
density gradients remain excluded; fluid stacks are added by the layered
checkpoint below, while analytic A and table paths remain single-layer. Smooth complex rigid
boundaries are covered by the boundary checkpoint below.
Spline positivity and coefficient-overflow checks still run at `CaseDefinition`
validation. Both mode backends share analytic mesh sampling: the f32 step from
`5000.0 / N` is promoted to f64 separately from the physical finite-difference
step; endpoint clamping must not erase this reference rounding.

All modes, shapes, attenuation, speeds and FIELD pressures are compared at
unchanged tolerances, including actual CLI-HDF5 readback:

| Derived input | Modes | Pressures | Reference mesh multipliers |
|---|---:|---:|---|
| `MunkLeakyPartialP` / `MunkLeakyPartialS` | 30 each (9 leaky) | 36 each | 1 |
| `PekerisComplexSpline3` | 4 (1 leaky) | 9 | 1/2/4 |
| `MunkAnalyticComplex` | 102 | 25 | 1/2/4 |
| `MunkLeakyPchipBroadband` at 75/50/62.5/50 Hz | 45/30/37/30 | 36 per block | 1/2/4 |

The broadband derivative has NG=803; scaling is performed before truncation,
with the repeated 50 Hz result stored in its own frequency-index group.
The analytic derivative has NG=2003 (noninteger analytic grid step), RMax=1000 km
and eleven modal sample depths. These are explicitly derived, reduced-geometry
inputs; small raw goldens are committed and all seven single/multi-frequency
reference workflows produce identical `.mod/.shd` in three pinned runs.

The **unmodified upstream environments** `tests/Munk/MunkS.env` (S, originally
a SCOOTER fixture) and `MunkAnalytic.env` (A) are additionally run with KRAKENC:
102 modes and 25 pressures each, using a separately derived coherent `.flp`.
Neither original three-line `.flp` parses in pinned FIELD, so this is original
**environment** acceptance, not acceptance of the original input pairs or a
SCOOTER implementation. Hashes and provenance are
[with the goldens](../../crates/kraken/tests/fixtures/golden/README.md).

The **unmodified** BroadBand/MunkK pair now passes full 50/500 Hz KRAKENC
API and actual legacy/JSON CLI-HDF5 comparisons: 102/1,023 modes and 501,501
pressures per frequency, max |dp|=1.5360e-8 at unchanged tolerances. All modal
wavenumbers, attenuation, shapes and phase/group speeds are checked. Three
unmodified pinned runs have identical MOD/SHD; their large binaries stay out
of Git. KRAKEN acceptance is unchanged.

The old narrow-spectrum constant-water initial guess required 300M root work
before completing the 45th 500 Hz root. A two-root spacing predictor now takes
152,546,495 work for the complete search, including the first out-of-interval
root, without changing the 300M ceiling, secant tolerance, deflation or mesh.
It applies only to single lossless-fluid narrow spectra with vacuum top,
fluid A bottom and cHigh ≤ bottom cp. Water-loss, broader leaky, wide, layered,
table and elastic search paths retain their previous seeds. This does not certify arbitrary secant spectra.
A reduced-depth modal regression reproduces the old budget failure offline;
fresh CI also checks a separately derived 7500 Hz late failure after both
original frequencies succeed, preserving the old output and cleaning scratch.

### Single-fluid material attenuation

N/C/P/S SSP nodes accept nonnegative compressional absorption. The legacy
loader converts N (neper/m), M (dB/m), F (dB/m-kHz), W (dB/wavelength), Q
(quality factor; Q=0 means no loss), L (loss parameter), and m (dB/m at freq0
with a power law and transition frequency). For m, the water header adds beta/fT
and the bottom-options record adds its own beta/fT; require beta >= 0 and fT > 0.
The fourth option character adds Thorp T, Francois–Garrison F (T, salinity, pH,
mean depth), or biological B (1..200 finite layers with positive resonance/Q).
Legacy trailing-value inheritance applies to node absorption as to sound speed.

`CaseDefinition::water_attenuation_db_per_wavelength` is empty for lossless
water or holds one effective value per SSP node **at the case frequency**.
The existing bottom attenuation field uses the same canonical unit. Both
backends share complex SSP interpolation: convert losses at the supplied nodes
before interpolation, use complex N², and interpolate P/S real and imaginary
parts independently. Reject nonfinite coefficients and cubic negative/gain
undershoot, or Im(c) > Re(c). KRAKEN uses Re(omega²/c²) for real roots and the
first-mesh volume/boundary loss perturbation; KRAKENC uses complex coefficients
through shooting, inverse iteration and normalization. Refinement, frequency
order/repetition, mode limits, work ceilings and output schema are unchanged.

Volume loss is recomputed per frequency and sampled **at SSP nodes**, not at
each finite-difference node. Thorp/FG also apply to a fluid half-space; biological
layers do not (`UpdateHSLoss` passes HUGE(depth)). Analytic Munk rejects volume
addition and has no lossy point records. F/P bottoms remain lossless N/C only;
the later boundary checkpoint adds smooth rigid/half-space combinations and
limited top TRC, not lossy/P/S/A table combinations.
Density gradients remain rejected: pinned acoustic shooting/vector/normalization
homogenize density using the top of each medium, not a physical density gradient.

Fourteen explicitly **derived** input pairs produce 27 accepted reference
workflows (13 through both backends, leaky broadband through KRAKENC only),
36 frequency blocks, 404 modes and 432 pressures. All `.mod/.shd` outputs are
byte-identical in three pinned runs. Offline goldens, API and actual CLI-HDF5
comparisons use existing tolerances; local maximum pressure error is 3.34e-8.
`WaterLossN/C/P/S` cover depth-varying complex SSPs; `WaterLossUnitN/M/F/Q/L`
cover additional material units; `WaterLossPower` covers distinct water/bottom
power laws, a transition inside 75/50/62.5/50 Hz and fractional NG scaling;
`WaterLossThorp/Fg/Bio` cover volume terms at 500 Hz, including overlapping
biological layers; `WaterLossLeaky` combines PCHIP water loss, bottom loss,
leaky modes, refinement and repeated frequencies. These are **not** acceptance
of original upstream VolAtt input pairs. Fresh CI is configured to check the
same 27 workflows through the API and actual HDF5 output.

### Finite fluid stacks

`CaseDefinition` keeps its existing water fields as the first layer and adds
`additional_fluid_layers: Vec<FluidLayer>` in depth order. Each additional layer
has absolute SSP depths from the previous interface to `bottom_depth_m`,
constant `density_g_cm3`, nominal `mesh_points`, and empty or per-node
`attenuation_db_per_wavelength` at the solve frequency. `total_depth_m()` is
the whole finite stack depth; source/receiver/modal depths may cross interfaces.
Legacy multi-medium input maps into the same model, preserves trailing-value
inheritance across records/media, converts loss separately at every layer/node/
frequency, and locates diagnostics in the offending medium. Layer-specific m
power laws and shared T/F/B volume loss use the existing conversion path.

Profiles interpolate independently; P/S never bridge a material discontinuity.
Each layer contributes its own h and density to the acoustic matrix, shooting,
normalization, group speed and attenuation integrals. Interface pressure and
P'/density are continuous; this is not a concatenated uniform mesh or an
averaged density. Two material coefficients share one pressure unknown.
KRAKEN uses tridiagonal inertia counts for the stack; KRAKENC transfers the
complex boundary state through all layers and deflates every searched root.
The second complex mesh retains the high/previous-root scan; Neville seeds
start on the third mesh, using raw old roots rather than Richardson output.
Shapes/group speeds stay first-mesh, with reference f32 mesh/sample rounding.

Smooth V/R/A boundaries, trapped KRAKEN, trapped/leaky KRAKENC, repeated
frequencies, up to five refinement meshes and coherent line-/point-source FIELD
extend to fluid stacks. Analytic Munk A, F/P bottoms and top TRC **remain
single-layer**; no new table/analytic combination is enabled. Density variation
inside a fluid layer, roughness and multiple FIELD profiles remain unsupported.
Elastic half-spaces and finite solid caps are added by separate
checkpoints below, not by treating solids as additional fluid layers. At most 500 finite layers, 100,000 total SSP nodes and 100,000
total loss values are retained. Mesh intervals, shape values, copied frequency
inputs and numerical work use the previous ceilings **for the entire stack**.
HDF5 v1 adds ordered finite-layer metadata without changing datasets or BELLHOP v3.

Twenty-two explicitly derived pairs pass 43 full API/actual-CLI-HDF5 workflows:
52 frequency blocks, 539 modes and 5,028 pressures, with byte-identical `.mod/.shd`
in three pinned runs. N/C/P/S, unequal meshes, density/speed/loss jumps, all nine
V/R/A combinations, fractional interfaces and interface-side samples are covered.
Power-law and leaky PCHIP derivatives keep 75/50/62.5/50 Hz and meshes 1/2/4;
a 500 Hz biological case samples overlapping volume loss on both sides of an
interface. Local maximum pressure error is 2.64e-9; tolerances are unchanged.
All media records/frequencies are checked, including a last-medium corruption
regression; ordinary tests also check metadata, shared budgets and output failure.

**Original `double` now passes both engines.** The byte-identical TLslices
environment and official shared `fieldbat.flp` (selected by upstream `runtests.m`)
are retained as `OriginalLayeredDouble`, with NG=100/200/200 and RMax=1000 km.
Both pinned searches and Rust find 43 then 42 roots on meshes 1/2; refinement
retains the surviving first-mesh shapes/group speeds/loss and Richardson columns.
Count increases still return `KR0303`. All 42 modes and 501 pressures per engine
pass API and actual legacy/JSON CLI-HDF5 comparisons; max |dp| is 1.1824e-11.
Three unmodified runs produce identical MOD/SHD; `golden/original-double.sha256`
locks two input files and six new artifacts. No earlier input, reference,
tolerance or budget changes. `LayeredDoubleRefined` remains a separate derivative
with doubled NG=200/400/400. Likewise
`LayeredNormalization` removes the original `normal.env` bottom's shear speed
and is labelled derived. Original normal/flused now pass the elastic half-space
checkpoint below; original elsed/ice now pass the homogeneous finite-cap checkpoint.
Gulf's multi-profile environment is now covered by the FIELD checkpoint below.

The three-layer narrowed refined fixture uses cHigh=1700 (four modes); its
separate RMax=0 wide fixture uses cHigh=1800 (five modes). The added
`LayeredFluidThreeWideRefined` changes only the wide input's RMax to 1000 km and
now passes **both complete products**: five KRAKEN modes, four KRAKENC modes,
63 pressures per engine. KRAKENC naturally searches 5 then 4 roots after
restoring N² reciprocal-before-weighting; no roots or references are trimmed.
[Diagnosis and acceptance](krakenc-three-layer-refinement.md) include one-variable
probes, WRITE-only reference byte controls, an ordinary regression and full
API/actual legacy/JSON CLI-HDF5 comparisons. The former PR #25 release exception
is no longer needed for this case. Reference-only seed perturbations remain
historical diagnostics, not accepted oracles. Old inputs/goldens, budgets and
tolerances are unchanged; no arbitrary-spectrum root-completeness or universal
CLI parity certificate is claimed.

### Elastic half-spaces

The [half-space checkpoint](../kraken/elastic-halfspaces.md) ports real/complex P/S
impedance and its normalization derivative without substituting acoustic material.
`Boundary::ElasticHalfSpace` stores positive shear speed and canonical
solve-frequency dB/wavelength shear loss; the existing boundary speed/density/loss
fields carry compressional material. Validation requires positive bulk modulus
(cp² > 4/3 cs²), finite nonnegative losses and Im(c) <= Re(c). N/C/P/S finite
layers remain fluid, constant-density and independently meshed. Analytic Munk,
F/P/TRC, rough interfaces and finite solid layers are not enabled.

Both engines accept elastic top/bottom. KRAKEN retains the pinned cs cutoff and
0.85*cMin adjustment. Real impedance ignores P/S loss, but Normalize retains the
top-A compressional perturbation; shear and bottom elastic absorption are omitted.
KRAKENC includes complex attenuation/radiating shear roots. Both retain the
compressional half-space group-speed expression, not independently verified
elastic energy/group dispersion. Real tops use shared Solve1 intervals/ZBRENTX
on meshes one/two, then non-deflated Solve2; MINLOC uses the previous Richardson
row to select the surviving first-mesh data, without oracle counts or clipping rules.

16 derived pairs plus original TLslices scholte/normal/flused with official shared
fieldbat.flp pass 37 complete workflows: 49 frequency blocks, 473 modes, 5,715
pressures, triplicate byte-identical .mod/.shd, API and actual CLI/HDF5 at unchanged
tolerances. The original environments are byte-identical (no shear removal or NG
change); normal's bottom keeps cs=2000. Maximum local pressure error is 6.67e-8.
The new metadata distinguishes requested loss from the reference real/complex
attenuation model. Original elsed/ice now pass the separate finite-cap checkpoint;
failed slow-interface and 75 Hz top/both broadband probes remain unaccepted.
Real elastic-top/finite-solid combinations remain explicitly unvalidated;
the layered release exception is not extended to these paths.

### Homogeneous finite elastic caps

[Finite-layer scope/evidence](../kraken/finite-elastic-layers.md): five-component
compound-matrix transfer through independently meshed constant cp/cs/rho/loss
solids above/below contiguous fluids through both backends. Real SSP-node
rounding, scalar operation grouping and Solve2's mesh/frequency search bound
are retained; no oracle-count cropping or parity waiver is used.
Original elsed and ice plus 23 constructed/derived pairs pass 50 full API and
actual CLI/HDF5 workflows: 56 frequency blocks, 933 modes, 10,536 pressures,
triplicate identical pinned MOD/SHD, unchanged tolerances. Pressure sampling stays
inside the absolute fluid interval; no solid displacement output or fictitious
fluid substitution. Total media/mesh/storage/root-work budgets are shared.
KRAKEN finite coefficients use Re(c²) but omit elastic absorption perturbation;
use KRAKENC for complex elastic attenuation. Graded/interleaved solids, analytic
Munk, tables, roughness, pure solids and acoustic outer A behind a solid remain
explicitly unsupported. Source bytes and 200 SHA records lock provenance.

### Smooth boundaries and top reflection tables

Both backends accept all nine smooth top/bottom V/R/A combinations. Public
`Boundary` is shared by `SurfaceBoundary`/`BottomBoundary` aliases; top A uses
`surface_sound_speed_mps`, `surface_density_g_cm3` and
`surface_attenuation_db_per_wavelength` (canonical at the case frequency).
Non-A boundaries require all three corresponding material fields to be zero.
Normalization, group speed and KRAKEN first-mesh loss perturbation include each
A half-space; KRAKENC uses complex impedance at either end. Legacy top A inherits
its material into the first water record, as in pinned ReadEnvironment; top A
with m units is rejected because the reference does not define its power-law
parameters. Source/receiver depths remain inside water, not in either half-space.

Eleven derived pairs cover the nine V/R/A combinations with spline water loss,
refinement and boundary loss, a constant rigid-rigid lossy plane mode (NG=68),
and a radiating 343 m/s air top (KRAKENC only): 21 API/CLI-HDF5 workflows.
The automatic NG=17 rigid-plane variant changes its searched mode count between
meshes and is explicitly rejected, not trimmed to the first mesh. Complex
refinement seeds each root from the third mesh onward using raw previous meshes
(Neville interpolation in h²), independently of Richardson output rows; this prevents third-mesh root
skipping with the top A/vacuum-bottom case. Shapes/group speeds remain first-mesh.

Top F consumes a bounded same-stem `.trc` in the same count/angle/magnitude/
unwrapped-phase format as BRC. It is restricted to single-frequency KRAKENC,
lossless N/C water, RMax=0, blank restart, no B option and a smooth V/R/A bottom.
**cLow must be at least the last SSP-node speed**, the inside-water speed used
by pinned BCImpedancec even at the top. Evanescent top-table roots, simultaneous
top/bottom tables and top P/IRC are explicitly rejected. A forward-gradient
TRC experiment outside this bound produced a growing reference root and is not
accepted; no root is dropped to make the comparison pass. Three constructed
TRC derivatives with N/C water, cLow=1550 and A/R bottoms add three workflows.
All 24 boundary `.mod/.shd` workflows are byte-identical in three pinned runs.
Their complete modes/FIELD and actual CLI-HDF5 pass existing tolerances.

`load_frequency_cases_with_boundary_tables` parses supplied top/bottom snapshots
without rereading them; older source APIs explicitly reject a missing TRC.
CLI provenance adds `/inputs/trc` and protects that consumed resource/aliases
as output destinations, including input, quota and numerical failures.
`surface_boundary` and bottom V are additive HDF5-v1 metadata; datasets are unchanged.
These are constructed/derived fixtures, not acceptance of an original TRC pair.

### Tabulated KRAKENC bottoms

Bottom `F` consumes the same-stem `.brc`; `P` consumes `.irc`. This slice
requires **one frequency, RMax=0, lossless constant-density N/C single water
layer, vacuum top, smooth bottom and blank restart option**. The `B` frequency
option (even with one value), table mesh refinement, random restarts, real
KRAKEN F/P, simultaneous top TRC and elastic/multilayer propagation are rejected or remain
outside this slice. There is no half-space record/material for F/P; direct
`CaseDefinition` half-space speed, density and attenuation must be zero.

Both formats require 2–100,000 ordered entries, bounded by the 1 MiB UTF-8
input limit. BRC has a count followed by one `angle magnitude phase_degrees`
record per point: finite increasing angles in 0..90 degrees, finite
nonnegative magnitudes, and finite **already unwrapped** phases. Interpolation
is linear in magnitude/phase, not complex reflection. The grazing angle uses
the real parts of complex kx/kz and reference f32 bracket selection; reflection
is zero outside the angle interval. BRC contains no frequency field; the caller
must supply a table appropriate to the single solve frequency.

IRC has a quoted title/frequency header, count and ASCII fixed-width
`(5G15.7,I5)` rows: real k², complex f, complex g, decimal scaling power.
E/D and letterless three-digit exponents are supported, not arbitrary
whitespace-delimited rewrites. The header frequency must exactly equal the
solve frequency. Real k² is finite, nonnegative and strictly increasing;
f/g are finite, powers lie in -1000..1000, and every adjacent two/three-point
window spans at most 100 powers. Brackets use real k²; up to three rescaled
points form a polynomial evaluated at complex k². Outside the real domain,
the corresponding endpoint f/g/power is retained.

Shooting, inverse iteration and normalization share the actual `(f,g,power)`
boundary; no fictitious half-space is substituted. F/P retains the f/g
normalization derivative but has no A-half-space group-speed contribution.
Critical coefficient/division and real-component grouping retain the pinned
GNU Fortran `-ffast-math` order: single-ulp differences can select a different
IRC secant root. Root work still uses the existing 20,000 roots / 300M operations
ceilings; the water-only root estimate is not a bound for table boundaries.
Singular/nonfinite boundary evaluations or normalization return diagnostics.
Reaching a lower spectral endpoint is not proof of every mathematical root.

`tools/reference/prepare-tabref.sh` copies unmodified upstream inputs and runs
**source-rebuilt pinned BOUNCE** three times to generate tables, then KRAKENC
and FIELD. The upstream directory contains no table files: these are generated
reference resources, not upstream original `.brc/.irc`. Tables and `.mod/.shd`
are identical across runs; Rust and actual CLI-HDF5 readback compare all modes,
shapes, speeds, attenuation and pressures at unchanged tolerances:

| Original TabRefCoef pair | Modes | Complex FIELD pressures |
|---|---:|---:|
| `neggradC_geo.env/.flp` | 56 | 50,601 |
| `neggradC_brc.env/.flp` + generated BRC | 54 | 50,601 |
| `neggradC_irc.env/.flp` + generated IRC | 42 | 50,601 |

Their `.env` samples 1..100 m while original `.flp` requests 0..100 m; neither
input is modified. BRC/IRC/geo results differ and are checked against their
own reference, not forced to agree. Four small **derived** 50 Hz N/C cases
with **constructed** three-point tables additionally cover line-/point-source
FIELD and nonzero IRC powers in committed goldens and fresh CI. Hashes and
source/compiler records are [with the goldens](../../crates/kraken/tests/fixtures/golden/README.md).
Rust BOUNCE generation remains excluded. CLI schema-v1 additive metadata records
the exact consumed table snapshot alongside `.env/.flp`.

### Single-profile FIELD extensions

The range-independent FIELD path now implements the remaining single-profile
options from the initial target: scaled-cylindrical geometry, tabulated source
patterns and incoherent mode addition. `S` uses point-source modal normalization
but omits the final cylindrical `1/sqrt(range + offset)` spreading. `.sbp`
levels are converted from dB to linear pressure amplitude and interpolated at
the pinned FIELD take-off angle (`c0 = 1500 m/s`). As in v2023.5 FIELD, one
pattern table shades the first source-depth block. `I` suppresses modal phase
and returns the complex square root of the sum of squared complex modal
contributions used by `EvaluateMod.f90`; it is not replaced by an absolute-value
intensity model.

Three small derived pairs are the fixed representative acceptance paths, not a
new option Cartesian product: `FieldScaled` through KRAKEN (3 modes/9 pressures),
`FieldPattern` through KRAKENC (4/9), and `FieldIncoherent` through KRAKENC
(4/9). Their MOD/SHD outputs are byte-identical in three pinned runs. Complete
modes, shapes and pressures pass the existing tolerances through the API and
actual CLI/HDF5; local maximum pressure error is 3.34e-8. The 16-record
`golden/single-profile-field.sha256` manifest locks inputs and reference files.
HDF5 records addition/pattern metadata and the exact consumed SBP snapshot.
This checkpoint excludes multi-profile propagation, covered separately below;
no newly observed option combination is added to its acceptance scope.

### Multi-profile FIELD

`legacy::load_field_cases` and `solve_field` now accept ordered ENV sequences
with adiabatic or coupled FLP propagation. Every profile's full modal product is
retained. Coupled/incoherent input is rejected; coupled projection currently
requires smooth fluids and full-interval modal samples. Frequency order,
independent source marches, cumulative input/shape/work budgets and CLI atomic
publication are preserved. Zero lower phase-speed bounds and depth-only SSP
inheritance support byte-original Gulf inputs without altering them.

Two small four-profile derivatives and both original `Gulf/gulf_rd.env` +
`gulf_ad.flp` / `gulf_cm.flp` paths pass full API and actual CLI/HDF5 differential
at unchanged tolerances: 24 profile blocks, 1,020 modes, 1,003,050 pressures,
local maximum |dp| `4.1159031748919954e-10`. MOD/SHD are byte-identical in three
pinned runs. Schema-v1 additive profile groups retain all modes; the first mode
set remains available at the existing path via a hard link, not a duplicated
payload. [Propagation semantics, fixed evidence and limits](../kraken/field.md)
describe this capability block; no option Cartesian product is added.

### Self-contained JSON

Strict JSON v1 represents ordered frequency/profile blocks with inline boundary
tables and source patterns. Export stores already converted solve-frequency
losses; it does not add raw-unit recipes or retabulate geometry. Both backends
and existing case/profile validators, budgets and atomic HDF5 publication are
reused. `/inputs/json` hashes the exact parsed snapshot, not reconstructed ENV
resources. Unknown/duplicate fields and invalid combinations are rejected.

Ordinary tests cover exact legacy-definition round trips, relocated CLI runs,
nested validation, quotas and output rollback. The representative
workflows documented in [JSON validation](../kraken/json.md#from-env-and-flp-files) pass JSON API
and actual CLI/HDF5 fixed-oracle comparison, including
real finite TopN, ShearOnly and ordered Power, real elastic-half-space
TopN, BothS and ordered TopBroadband,
original BroadBand/MunkK, TabRefCoef BRC/IRC and Gulf AD/CM. Numerical tolerances,
reference files, BELLHOP schemas and physics support limits are unchanged; this
checkpoint adds no option Cartesian product. See [schema, units and
limits](../kraken/json.md).

### Multiple frequencies

`legacy::load_frequency_cases(env, flp, ModeSolver)` returns a `Vec<Case>` in
input frequency order. For complete FIELD runs, `legacy::load_field_cases`
and `solve_frequencies` process ordered frequency blocks lazily, preserving
real finite-elastic Solve2 search bounds and stopping at the first failure.
Independent `solve`/`solve_field` calls start separate reference runs, which can
have different finite-solid spectra from the same case inside a sequence.
`load_case` and `load_complex_case` reject multiple frequencies rather than
silently taking the first. The CLI/HDF5 adapter uses the ordered iterator with
cumulative output quotas; it adds no parallel/batched numerical solver,
FFT or time-domain response. JSON uses the separate canonical adapter described above;
HTTP is not provided.
The sixth top-option character `B` enables a frequency count/vector after
`.env` source/receiver depths. Frequencies are finite and positive; duplicates
and descending order are preserved, unlike sorted depth/range vectors. Slash-
terminated endpoint subtabulation uses double precision. The nominal `freq0`
remains `CaseDefinition::mesh_reference_frequency_hz = Some(freq0)`; `None`
uses the single case frequency. Automatic NG is resolved at freq0, then each
mesh uses `INT(NG * multiplier * frequency / freq0)`, not an already-rounded
base mesh multiplied afterward. For example NG=101 at freq0=50 Hz gives
151/303/606 intervals at 75 Hz, and 126/252/505 at 62.5 Hz. Existing `N/W`
loss semantics remain unchanged; modes and material/volume loss are recomputed
at every frequency, not obtained by scaling a previous mode set.

Two **derived** Pekeris broadband pairs have 75/50/62.5 Hz in that order,
NG=101 and RMax=1000 km: KRAKEN has 5/3/4 modes, KRAKENC has 7/4/6, with
nine line-source pressures per frequency. They pass committed and fresh
`.mod/.prt/.shd` comparisons. The **unmodified upstream**
`tests/BroadBand/MunkK.env/.flp` uses automatic `S` water sampling and `W`
bottom loss at 50/500 Hz through both engines: 102/1,023 modes and 501,501
pressures per frequency, 1,003,002 pressures per engine. Three pinned runs per
engine produced identical `.mod/.shd`; fresh CI compares all frequency blocks. Original artifacts
are not committed and do not replace the different single-frequency
`tests/Munk/MunkK` pair.

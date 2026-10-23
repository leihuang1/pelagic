# Finite elastic layers

Finite solids are now propagated with the pinned five-component compound matrix,
not replaced by acoustic layers or half-spaces. This bounded block covers smooth,
homogeneous or depth-varying solid caps above/below a contiguous N/C/P/S fluid stack:

- **KRAKENC:** multiple fluid layers and multiple finite solid caps, with complex
  P/S attenuation. Outer solid boundary must be V, R or elastic A.
- **KRAKEN:** multiple contiguous fluid layers and finite solid caps,
  retaining pinned real stiffness and elastic-loss omission. Elastic top
  half-spaces combined with finite solids remain unvalidated; fluid-only tops
  pass the separate half-space block.
- Finite cp/cs/density/P/S loss may vary with depth within each solid; jumps between
  solids are allowed, with full compound state carried across each interface. Roughness,
  solids interleaved with fluids, pure-solid cases, analytic Munk,
  F/P/TRC combinations remain excluded.
- Modal/source/receiver samples remain **fluid pressure**, at absolute depths
  inside the fluid interval. No displacement/strain sampling in solids is exposed.
- An acoustic outer A behind finite solids is rejected: the pinned BC routine
  does not initialize its five-component compound state in that branch.

`ElasticLayer` carries absolute bottom depth, cp, cs, density, canonical
solve-frequency P/S dB/wavelength losses and nominal NG. Its optional
`material_profile` stores complete `ElasticMaterialPoint` samples: absolute depth,
cp, cs, density and both losses. Empty profiles retain the homogeneous representation;
nonempty profiles include both endpoints and agree with scalar material at the first
sample. `top_elastic_layers`
starts at 0; `bottom_elastic_layers` starts at `fluid_bottom_depth_m()`. The
existing water fields still define the first fluid, now possibly below a cap.
`fluid_top_depth_m()` and `fluid_bottom_depth_m()` bound acoustic samples;
`total_depth_m()` is the physical last finite interface, excluding the half-space.
Legacy SSP inheritance is retained; a fluid after a solid must explicitly reset
cs to zero where inherited records would otherwise retain shear material.

Each solid uses its own shear-based automatic mesh or explicit NG. Frequency
scaling precedes truncation, including every refinement level. The reference
Euler first step, modified midpoint, terminal filter and decimal scaling exponent
are retained. The first physical medium's h supplies Neville seeds. KRAKEN
finite-solid cases use deflated real Solve2 secants instead of acoustic inertia;
KRAKENC reuses its complex secant, first-mesh shapes/group speeds and Richardson
roots. The first two raw mesh scans and later Neville seeds stay distinct from
extrapolated eigenvalues. Real solids retain SSP-node rounding even for uniform
materials, the pinned scalar operation grouping and full outer-boundary state.
N/C interpolate solid density linearly; P/S use the pinned PCHIP/spline density
coefficients. The constant-material rounding of existing inputs is retained.

`Solve2` carries its searched-mode bound `M` across meshes and frequencies. A
cHigh exit reduces the next search bound; surviving first-mesh shapes/loss and
Richardson rows follow that bound. This is not post-hoc cropping to an oracle
count. Ordinary fluid and KRAKENC mesh-count guards remain unchanged.
`solve_frequencies(&[FieldCase])` preserves ordered run-local state, computes
blocks lazily and stops after the first error. Independent `solve`/`solve_field`
calls start new runs; no global state or implicit material-loss scaling is added.

## Loss semantics and budgets

KRAKEN finite-solid stiffness uses **Re(c²)**, unlike its elastic-half-space
formula's **Re(c)²**. Changing finite-solid loss can therefore change the real
spectrum and pressure, but no elastic absorption perturbation is added. Original
ice has nonzero P/S losses yet zero real-KRAKEN modal attenuation; zeroing those
losses changes its spectrum. KRAKENC includes complex stiffness and absorption.
The original half-space limitation and cs cutoff are unchanged. Group speeds
follow the reference's fluid/compressional-half-space slowness expression, not an
independent full-elastic energy/group-dispersion validation.

Limits are unchanged and shared with fluid media: at most 500 total finite media,
1M total mesh intervals per frequency/level, 100K total conceptual profile/loss
entries, 20K roots, 5M sampled shapes, 1M FIELD samples, 5M copied-input entries,
1000 frequencies. Parsing counts all raw SSP records before compressing uniform
solids; copied solid material costs seven scalar entries plus six values per
retained profile sample per frequency. Both P/S node-loss arrays count toward the
existing aggregate loss-storage bound. Root work counts
five compound-component steps per solid node against the existing 2.5B KRAKEN /
300M KRAKENC limits. These are logical work/storage bounds, not wall-time or native
HDF5 descriptor guarantees. Atomic publication/quota/input-protection contracts
remain unchanged.

## Complete reference evidence

Acoustics Toolbox v2023.5 commit `475108519289c6fb488b58980c644ea14eccc604`, Linux
x86-64 / GNU Fortran 12.2.0, the same flags and unmodified numerical tolerances.
25 input pairs (23 derived/constructed plus two original pairs) give **50
workflows, 56 frequency blocks, 933 modes and 10,536 pressures**. Every mode, shape,
FIELD sample and material attribute is compared through API and actual CLI/HDF5.
All `.mod/.shd` are byte-identical in three independent pinned runs. Local max
pressure error: 5.96e-8. The 200-record `golden/finite-elastic.sha256` locks inputs
and generated artifacts; Rust HDF5 is never a golden.

- OriginalElasticSediment is byte-identical `tests/TLslices/elsed.env` (5000 m
  fluid, finite 100 m cp1400/cs700/rho1.5 sediment, elastic A bottom). Both engines:
  46 modes / 501 pressures, with every interface mode retained.
- OriginalElasticIce is byte-identical `tests/TLslices/ice.env` (30 m cp3000/cs1400
  lossy solid cap, fluid 30..5000 m, acoustic A bottom). Both: 44 / 501.
- Both official FLPs are byte-identical `fieldbat.flp`, selected by upstream
  `runtests.m`. The six FiniteSingleIce/Sediment C/P/S derivatives change only title
  and interpolation. They do not replace the independently accepted originals.
- FiniteElasticBottom/Top/Both N/C/P/S, shear-only, power-law and V/R bottom cases
  exercise independently meshed/dense layered fluids and homogeneous solids in
  both backends. The four-frequency power case preserves 75/50/62.5/50 Hz order/repeats.
- FiniteElasticStack adds two different solid media on each side of two fluids;
  no density averaging or interpolation crosses solid/material interfaces.

## Depth-varying material evidence

Twelve new derived input pairs add **23 workflows, 29 frequency blocks, 158 modes
and 1,827 pressures**, with maximum pressure error **2.08250058582033e-9**. The
93-record `golden/graded-elastic.sha256` contains 24 input files and 69 Fortran
artifacts. Three independent runs have byte-identical MOD/SHD output; PRT records
retain CPU timings and only trim trailing whitespace. Existing inputs, goldens,
tolerances and work budgets are unchanged.

Each solid has four depth samples, varying cp, cs, density and both losses.
`GradedElasticTop{N,C,P,S}` and `GradedElasticBottom{N,C,P,S}` exercise both engines;
`GradedElasticStack` covers separately sampled adjacent solids.
`GradedElasticPower` preserves 75/50/62.5/50 Hz and converts raw power-law losses
at every node for every frequency. `GradedElasticBio` includes depth-local
biological loss in finite material. API and actual legacy CLI/HDF5 compare every
declared mode, shape, group speed, loss and pressure. Four representative inputs
also pass self-contained JSON API and actual JSON CLI/HDF5 against the same oracle.

Top N/C and Bio use RMax=0 because their constructed 1000-km KRAKENC probes fail
the reference's mesh-convergence check; these probes are not accepted evidence.
The separately named real `GradedElasticTopNRefined` keeps RMax=1000 km and checks
the complete three-mode refined spectrum. The other top/bottom/stack/power paths
retain the inherited refinement range. No count clipping or relaxed convergence
guard is used. N² sampling retains the pinned principal complex square root;
an FMA residual corrects platform `hypot` rounding at identical complex inputs.

## Real multi-fluid regression and remaining limits

The formerly rejected real paths now compare complete declared spectra and FIELD:
TopN has four modes, ShearOnly three, and Power 8/6/6/6 at 75/50/62.5/50 Hz.
The same canonical 62.5 Hz Power case has seven modes in a separate pinned run;
its six-mode result in the sequence comes from the preceding search bound.
Spline BothS has three modes; replacing constant spline density with rounded
linear weights would incorrectly find six. All 17 constructed real workflows
add 20 blocks, 106 modes and 1,260 pressures to the earlier evidence.
Write-only diagnostic Fortran rebuilds retained byte-identical MOD/SHD controls;
no altered seed, tolerance, numerical reference or solver budget is accepted.
The TopN, ShearOnly and ordered Power cases also pass self-contained JSON API
and actual JSON CLI/HDF5 comparison against the same references.

Interleaved solids, real elastic-top/finite-solid combinations and the other excluded
combinations above remain outside this checkpoint. The former KRAKENC
[three-layer refinement gap is fixed](../history/krakenc-three-layer-refinement.md);
no new parity waiver is added.
Separate [multi-profile FIELD](field.md) and
[JSON](json.md) capabilities are already implemented within their
published limits. This block does not certify every branch-sensitive spectrum,
all mathematical roots or an elastic multi-profile combination matrix; HTTP,
BOUNCE generation, elastic displacement and time-domain products are not added.

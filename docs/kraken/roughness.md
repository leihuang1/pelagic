# Rough fluid interfaces

KRAKEN and KRAKENC now include the official v2023.5 Kuperman–Ingenito
first-order interface-scattering perturbation. This closes the former rejection
of original TLslices `kupert`, `kuperb` and `kuperm`; it is not a new rough-surface
ray model or an arbitrary-roughness accuracy guarantee.

## Input and scope

`CaseDefinition::interface_roughness_m` is an optional vector of finite,
nonnegative RMS heights in metres. Empty means smooth. Otherwise it contains
**fluid layer count + 1** values: the top, each internal interface, and the bottom.
Missing JSON fields default to empty, preserving existing JSON v1 documents.
Legacy ENV medium-header sigma values and bottom-option sigma populate the same
vector; all-zero legacy inputs retain the empty representation. Frequency scaling
changes materials/meshes, not the physical roughness heights. Input-copy budgets
include the vector. Rust `CaseDefinition` struct literals must initialize the new
field (`Vec::new()` for a smooth case).

The implemented slice is contiguous fluid media with V/R/fluid-A boundaries.
Nonzero roughness with finite solids, elastic half-spaces or reflection/impedance
tables is rejected, rather than silently using a fluid approximation. N/C/P/S
sampling is shared with the existing modal solver. The fixed evidence below
covers range-independent FIELD, including layered, lossy, refined, multi-source
and ordered/repeated-frequency inputs; it does not certify all interpolation,
roughness and multi-profile combinations.

This remains the reference's perturbative physical model. Large RMS roughness is
not a justification to treat its output as an independently accurate scattering
solution. The original units, loss models and work ceilings are unchanged.

## Numerical ownership

`solver/scattering.rs` follows official `Kraken/Scattering.f90::KupIng` and both
engines' `ScatterLoss`. It uses normalized first-mesh pressure and the reference's
one-sided interface derivative, separate material samples on either side, and
the scattering square-root branch (negative imaginary root for negative real
arguments). It does not modify the smooth-interface root search or mode shape.

The real engine supplies its real mesh coefficients; the complex engine supplies
complex coefficients. Top-half-space velocity retains the engines' different
root branches. Vacuum/rigid exterior density limits follow the official formulas.
Identical materials give zero interface scattering.

`Refinement` now retains an explicit complex **first-mesh k² perturbation** next
to each mode. Real KRAKEN adds material/boundary loss and scattering; KRAKENC's
complex roots already include material loss and add only scattering. Both add the
complete stored perturbation to the final Richardson eigenvalue before taking the
square root. This preserves scattering's real as well as imaginary contribution;
it must not disappear during refinement or be extrapolated as a new root.
Surviving perturbations follow the existing spectrum-truncation bookkeeping.
Shapes and group speeds remain first-mesh data, as in official Fortran.

HDF5 v1 fluid `media/{index}` metadata adds float64 `top_roughness_m` and
`bottom_roughness_m`. Older files may omit these additive attributes. Native MOD
has no RMS-roughness field; the original ENV/JSON and HDF5 provenance retain it.

## Fixed evidence

The same unmodified official v2023.5 compiler/runtime supplies all new goldens;
no previous golden or numerical threshold is changed. `golden/roughness.sha256`
locks **32 records: eight inputs and 24 MOD/SHD/PRT artifacts**.

| Input | Provenance | Modes per engine | Pressures per engine |
|---|---|---:|---:|
| RoughSurface | byte-original TLslices/kupert + shared fieldbat.flp | 46 | 501 |
| RoughBottom | byte-original TLslices/kuperb + shared fieldbat.flp | 44 | 501 |
| RoughInterface | byte-original TLslices/kuperm + shared fieldbat.flp | 46 | 501 |
| RoughBroadband | labelled LayeredFluidPower derivative, sigma 0.1/0.2/0.3 m | 8/5/7/5 | 252 |

Eight workflows comprise **14 frequency blocks, 322 modes and 3,510 pressures**.
The broadband derivative keeps 75/50/62.5/50 Hz, three sources, seven receivers,
material power laws and RMax=1000 km. Both engines pass the existing mode/shape/
pressure comparator. Scattering is classified as physical loss under the
existing imaginary-k loss tolerance (1e-10); printed-loss precision checks and
all other thresholds are unchanged, not replaced by a looser roughness tolerance.

The same eight workflows also pass fresh same-container Linux Fortran/Rust
comparisons through legacy and JSON APIs, actual CLI `--format both`, HDF5
readback and independent native readback. Initial evidence is retained under
`target/kraken-numerics/linux-smoke/`; the [broader business report](../development/kraken-business-coverage.md)
retains its final outcomes and unresolved cases.

Independent controls include a homogeneous vacuum/rigid waveguide's first-mode
small-roughness formula `alpha = sigma² kz³ / (D kx)`, quadratic height scaling,
identical-material cancellation, root-branch checks and invalid input/unsupported
combination rejection. The first-mode control deliberately isolates one mode;
it is not a proof of general secant root completeness. An earlier wide-interval
control exposed branch-sensitive KRAKENC mesh counts and is retained under
`target/kraken-numerics/probes/rigid-wide*`, not reclassified as a numerical pass.

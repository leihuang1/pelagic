# Wide three-layer KRAKENC refinement: parity fix

**Fixed for the pinned workflow; no release exception is needed for this case.**
`LayeredFluidThreeWideRefined.env/.flp` keeps the earlier wide fixture's bytes
except RMax changes from 0 to 1000 km. NG=101/83/127, cLow/cHigh=1400/1800 m/s,
materials, loss, title and FIELD geometry are unchanged. The narrowed
`LayeredFluidThree` and base-mesh `LayeredFluidThreeWide` remain distinct cases;
none of their inputs, reference outputs or tolerances was changed.

Pinned Acoustics Toolbox v2023.5 (`475108519289c6fb488b58980c644ea14eccc604`),
GNU Fortran 12.2.0 and the documented fast-math flags search five roots on mesh 1
and four on mesh 2. Rust now does the same. Shared refinement retains the four
surviving first-mesh shapes/group speeds and extrapolates complex k². This is
an actual search contraction, not deletion of a fifth Rust root or a
reference-derived root-count rule. KRAKEN independently returns five modes for
the same derived input; both complete products are accepted.

## Root cause and minimal fix

Fortran `sspMod.f90::n2Linear` computes the two complex reciprocals `1/c²`, then
multiplies them by the interpolation weights. Rust's lossy-fluid branch used
`(1-w)/c_top² + w/c_bottom²` instead. These algebraically equivalent expressions
round differently. Small coefficient differences propagate through the first
four roots and are amplified by cancellation in the fifth deflated secant.

`profile.rs::complex_speed` now reuses the existing reciprocal-before-weighting
order for N² interpolation. The existing elastic/real principal-root branch is
retained; ordinary complex-fluid sqrt, CRCI conversion, half-space arithmetic,
shooting, deflation, seed selection and stopping tests are unchanged. No
fixture-name checks, forced seeds, root truncation, new numerical abstraction,
raised work ceiling or relaxed tolerance is used.

## Diagnosis and causal checks

The old failing count regression and fresh complete API comparator reproduced
reference=4/Rust=5 before the fix. A WRITE-only reference rebuild recorded both
meshes' coefficients and natural roots, plus mesh-2 boundary/interface,
raw-dispersion and deflation evaluations. Its MOD/SHD bytes matched the installed
unmodified oracle, including after input-node tracing was added.

One-variable probes and subsequent removal checks gave:

| Rust probe | Final KRAKENC modes |
|---|---:|
| Baseline | 5 |
| Pinned two-step CRCI conversion only | 5 |
| CRCI plus algebraic principal sqrt, original weighted division | 5 |
| CRCI, algebraic sqrt and reciprocal-before-weighting | 4 |
| Remove CRCI change | 4 |
| Also restore original complex-fluid sqrt: final minimal fix | 4 |

The combined coefficient diagnostic matched all 314/625 complex speeds and
coefficients bit-for-bit. That was an isolation probe, **not** the shipped
patch or a claim that the final solver is universally bit-identical. Removing
unnecessary CRCI/sqrt changes preserved complete numerical acceptance. Small
remaining root differences stay within the existing product tolerances.

The mesh-2 first seed remains `0.047895828732633719 + 0i`. The old fourth root's
real error was about `1.6653e-15`, its imaginary error `8.4504e-16`. The minimal
patch reduces the real error to about `2.84e-16`; the natural fifth search exits
at the reference's out-of-window root near
`0.0273422640556015 - 0.000981650507728i`, rather than the old in-window root near
`0.032081981205861 - 0.0000628866847424i`. Scaling powers remain zero in this
probe. These observations localize this fix; they do not certify all
mathematical roots or every branch-sensitive spectrum.

Historical `tools/reference/probe-layered-refinement.py` perturbations of the
oracle's fifth seed by ±256 ULP still illustrate sensitivity. They are
**diagnostic only**, never acceptance references or goldens. Their five-root
outputs matched the old Rust implementation, not the current four-root product.
The installed reference/image is never altered.

## Acceptance and regression

The ordinary, no-longer-ignored regression checks five base-mesh modes, four
refined modes, all 63 pressures, and preservation of surviving first-mesh
shapes/group speeds:

```sh
cargo test --release -p kraken --test layered \
  wide_three_layer_refinement_keeps_the_pinned_mode_count -- --exact --nocapture
```

All declared modal wavenumbers, attenuation, phase/group speeds, aligned shapes
and 63 pressures pass the unchanged complete comparator. Both engines have
fresh API and actual legacy CLI-HDF5 coverage; KRAKENC additionally passes
self-contained JSON API and actual JSON CLI-HDF5. Maximum pressure error for the
fixed KRAKENC case is `1.4725502861e-9`; KRAKEN's is `1.3969838619e-9`.
Three unmodified reference runs per engine have identical MOD/SHD. The new
`golden/layered-refinement.sha256` records two derived inputs and six artifacts;
old inputs/goldens remain unchanged. The former fresh CI, now
[local full acceptance](reference-workflows.md#full-pre-release-acceptance), includes
this pair in the layered group and KRAKENC in the JSON group. Offline goldens compare complete modes and FIELD,
not only the four-root count.

FIELD receiver ranges remain 0.5/1/2 km: RMax=1000 km controls refinement, not
receiver distance. The fix does not claim the fifth mathematical root has
negligible physical effect, or that successful CLI/HDF5 publication certifies
arbitrary-input reference parity. The former PR #25 release-gate exception is
historical and is not extended to any other failure or capability.

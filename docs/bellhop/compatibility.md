# Input compatibility

The CLI accepts both the modern [self-contained JSON format](json.md) and
the legacy two-dimensional environment syntax accepted by Acoustics Toolbox
`v2023.5`. The HTTP service accepts JSON only. `pelagic bellhop export case.env` resolves
a legacy case and all same-stem resources into one canonical JSON document.

## Legacy inputs

Implemented:

- Fortran list-directed records used by legacy inputs
- quoted and unquoted fields, commas, `!` comments, `/` termination, repetition, null values, and `D` exponents
- legacy vector subtabulation and sorting
- `N/C/P/S/Q/A` SSP selection
- supported boundary, attenuation, run, receiver-grid, source, and beam options
- development-mode single-beam selection
- range-dependent `.ssp` matrices
- short and long, piecewise-linear and curvilinear `.ati`/`.bty` boundary data
- `.brc`/`.trc` magnitude and phase tables
- BOUNCE/KRAKEN `.irc` impedance-function tables for a precalculated bottom reflection condition
- `.sbp` source beam patterns
- same-directory, same-stem auxiliary-file resolution through `load_case`
- semantic validation without invoking the numerical solver

Numerically implemented:

- deterministic single-threaded `R` ray traces
- `N/C/P/S/Q/A` sound-speed evaluation and derivatives
- reference modified polygon/midpoint integration with interface and boundary step reduction
- flat, piecewise-linear, curvilinear, and range-dependent boundaries
- vacuum, rigid, acousto-elastic, grain-size, angularly tabulated, and precalculated internal-impedance reflection conditions
- source beam-pattern weighting and reflection amplitude/phase accumulation
- `E` eigenray and `A/a` arrival calculations for Cartesian/ray-centered geometric-hat (`G/g`) and Cartesian geometric-Gaussian (`B`) beams
- simple-Gaussian (`S`) eigenray detection
- coherent, semi-coherent, and incoherent (`C/S/I`) pressure fields
- geometric-hat (`G/g`), Cartesian geometric-Gaussian (`B`), simple-Gaussian (`S`), and Cartesian/ray-centered Cerveny (`C/R`) field influence
- Cerveny space-filling, minimum-width, and WKB epsilon selection; image beams, windows, branch cuts, and boundary-curvature conditions
- point and line source scaling, rectilinear and irregular receiver grids, caustic phases, arrival combination, and strongest-arrival retention

Unavailable because the corresponding reference paths are incomplete:

- `W` table generation: BELLHOP v2023.5 only prints that it is writing an IRC file; it has no generator, does not read a layered medium for this option, and reaches its unknown-boundary error if a ray reflects
- simple-Gaussian arrival influence and Cerveny eigenray/arrival combinations; reference BELLHOP's corresponding influence paths do not emit these products
- ray-centered geometric-Gaussian (`b`) influence; this option is also explicitly unimplemented by reference BELLHOP

Three-dimensional options are rejected explicitly.

The complete case loader has also been exercised against 69 two-dimensional environments invoked by the official `v2023.5` MATLAB test scripts, including all auxiliary files they request. `tests/MunkRot/MunkRot.env` is intentionally rejected because its run type explicitly selects three-dimensional behavior.

### Irregular receiver-grid limitation

Irregular grids retain the pinned v2023.5 influence behavior, which differs
by beam family:

| Beam family | Depth used at receiver range `i` |
| --- | --- |
| Cartesian geometric-hat (`G`) and geometric-Gaussian (`B`) | Paired `depths[i]` |
| Ray-centered geometric-hat (`g`), simple-Gaussian (`S`), Cerveny (`C/R`) | First depth `depths[0]` |

For the latter four families, output coordinates still label the requested
paired depths, **not the depths used to calculate the pressure**. Do not use
these combinations to model varying-depth paired receivers. Use `G/B`, or a
rectilinear grid and select the desired range/depth cells afterwards.
This is a reference limitation, not a Rust porting defect; changing the
influence calculation would break numerical compatibility.

The pinned source sets `NRz_per_range=1` in `Bellhop/bellhop.f90:202`.
`Bellhop/influence.f90:64,252,322,693` then use `Rz(iz)`, while lines 458/578
explicitly select `Rz(ir)`. Six ordinary Rust characterization tests and an
opt-in fresh-reference test cover coherent fields with mixed depths `[20,90]`
and uniform controls `[20,20]`/`[90,90]`. The latter is included in
[full pre-release acceptance](../development/reference-workflows.md#full-pre-release-acceptance),
not ordinary PR CI, and runs all 18 derived cases,
checks reference pairing byte-for-byte, and compares receiver 1's Rust pressure
with Fortran at the unchanged `5e-8` absolute component tolerance:

```sh
# Requires the pinned image; see ../development/reference-workflows.md.
tools/reference/test-one.sh --release -p bellhop --test irregular_pairing \
  irregular_pairing_matches_pinned_reference -- --ignored --exact --nocapture
```

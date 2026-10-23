# Official BELLHOP 2026 source review

## Decision

**The official 2026-08-29 package does not fix the confirmed 2D boundary-contact
instability.** It contains meaningful changes elsewhere, but is not a drop-in
numerical replacement for our pinned `v2023.5` oracle. Do not silently upgrade the
oracle, overwrite goldens, or import the whole release as a boundary fix.

This investigation uses only official sources. No third-party solver or source
patch was introduced. The production Rust solver and pinned reference remain
unchanged. See the [preceding analytic investigation](bellhop-boundary-analysis.md)
for the defect and the [business matrix](../development/bellhop-business-coverage.md) for existing
compatibility evidence; its results have not been reclassified.

A subsequent minimal official-source candidate passed 90 analytic controls but
changed numerous complex fields without sufficient independent acceptance evidence.
On 2026-10-15 the operator requested withdrawal of that experiment and deferral
of the boundary repair. Its patch, checker and report are archived locally under
`target/bellhop-contact/withdrawn/`; they are not active project tools or an
accepted oracle. Production Rust and the pinned reference were never replaced.
See the [acceptance decision](../development/bellhop-business-coverage.md#operator-acceptance-and-deferred-boundary-work).

## Provenance and build

The [official download page](http://oalib.hlsresearch.com/AcousticsToolbox/), checked
on 2026-10-15, links to:

- Archive: <http://oalib.hlsresearch.com/AcousticsToolbox/at_2026_8_29.zip>
- Size: **40,439,628 bytes**
- SHA256: `307773ce2e0bc05a8131d8ec1d02a056bdd4d7545a1ae47fdedca2f8d28520d7`
- Comparison baseline: official `v2023.5`, commit
  `475108519289c6fb488b58980c644ea14eccc604`.

The old hostname's HTTPS certificate does not match that hostname. The download
therefore used the official page's HTTP URL, without disabling TLS verification.
The recorded hash identifies the inspected bytes; it is not an upstream signature
or proof of transport authenticity. The separate HTTPS OALIB resource index did
not provide this dated archive link.

The downloaded source remains untouched under
`target/bellhop-official-latest/source/at/`. A separate build copy compiled
successfully without source patches, using the **same Linux amd64 container,
GNU Fortran 12.2.0, runtime and flags as the old oracle**:

```text
-O1 -ffast-math -funroll-all-loops -fomit-frame-pointer -std=gnu
```

New BELLHOP binary SHA256:
`c064d2d5e570d70728564a2282802b538862bc95d5e74c4fe108dcec826e1fad`.
Old oracle SHA256:
`23d990a73983bedc1d37572ea2aa1319f65f446691cf3a821c7eaf9ef41213ce`.

The container runs through Docker on Apple ARM64.
All runs use a short case stem in the case directory and a 600-second subprocess
limit. Original ENV/resource bytes are copied unchanged. No numerical tolerance,
solver quota, or existing reference artifact was altered.

## 1. Boundary defect: still present, with a larger minimum step

The active 2D executable still uses `Bellhop/Step.f90`; the newly added
`Step2DMod.f90` belongs to the BELLHOP3D/Nx2D executable, not this path.

The 2D reflection condition in `bellhop.f90` remains:

```fortran
DistBegTop > 0.0d0 .AND. DistEndTop <= 0.0d0
```

The corresponding bottom condition is also unchanged. There is no new explicit
contact-event result. The boundary segment selection is not direction-aware.
The minimum step in `ReduceStep2D` changed from:

```text
v2023.5:    1e-4 * Beam%deltas
2026-08-29: 1e-3 * Beam%deltas
```

The same constant-speed analytic control was rerun: 100 m water depth, 25 m
source depth, -37 degree launch angle, 300 m requested step, 1010 m range limit.
The large requested step is valid for this test because boundary stepping must
shorten it; constant sound speed removes ordinary curved-ray truncation error.

| Official version | Maximum depth error against analytic reflected trajectory |
|---|---:|
| v2023.5 | 0.14443560555641 m |
| 2026-08-29 | **1.444356055564782 m** |

The entire existing 72-input analytic sweep was repeated with the new official
binary (six angles, zero/±1 ULP perturbation, four requested steps). The table's
case is its worst result. This is a demonstrated regression in that control,
**not a claim that all new-version trajectories are ten times less accurate**.

A separately labelled diagnostic build restores only the old minimum-step factor
in the latest source. Its error returns to exactly `0.14443560555641 m`. Reverting
only the Gaussian field routine does not change the 1.444 m error. This isolates
the cause; neither diagnostic reversion constitutes a boundary fix.

## 2. Three original complete fields change substantially

The unmodified original DickinsB, DickinsFlatB and ParaBotTLGB cases were run with
the latest official executable. All produced finite fields with the same grids
as the old oracle. The selected failing beam from each case was also rerun.

| Original field | Cells | New/old maximum component difference | Cells above existing 5e-8 threshold | New/old relative L2 difference |
|---|---:|---:|---:|---:|
| DickinsB | 601,601 | 4.1196e-3 | 553,156 | 18.5237% |
| DickinsFlatB | 201,201 | 2.2281e-3 | 198,113 | 16.3942% |
| ParaBotTLGB | 501,501 | 6.4280e-4 | 250,684 | 5.6614% |

Relative L2 is `||P_new-P_old||_2 / ||P_old||_2`. These are **version differences,
not physical error estimates**. They must not be interpreted as proof that the
new fields are less accurate, nor as confirmation that the old fields are right.

### Controlled attribution

Three separate diagnostic builds were made from the new official source:

1. Restore only the old minimum-step factor.
2. Restore only the old `InfluenceGeoGaussianCart` routine.
3. Restore both.

| Diagnostic reversion | DickinsB relative L2 vs old | DickinsFlatB | ParaBotTLGB |
|---|---:|---:|---:|
| Minimum step only | 17.2979% | 16.0080% | 2.5859% |
| Gaussian routine only | 6.7351% | 3.5492% | 3.2735% |
| **Both** | **0** | **0** | **0** |

With both reversions, **every complex-f32 pressure value in all three complete
fields is exactly equal to the old oracle**. Thus the observed changes in these
particular fields are accounted for by two identified changes, not an unexplained
collection of upstream modifications. This attribution does not extend to all
other beam types, SSP options or environments.

## 3. What changed besides boundary stepping?

The table distinguishes source-confirmed changes from independently established
accuracy improvements. File references are relative to the official archive.

| Area | Source-confirmed change | Assessment |
|---|---|---|
| SSP derivative jumps | `Bellhop/Step.f90` applies p/q interface jumps only for N/C/Q/H; skips them for smooth PCHIP, cubic spline and analytic profiles. | A useful correction to evaluate independently: a smooth interpolant should not acquire an artificial first-derivative jump. No general accuracy certification performed here. |
| Cartesian Cerveny window | `Bellhop/influence.f90` changes the beam-window test from `omega * imag(gamma) * dz²` to its negative. | Corrects the sign of the decay-window quantity for decaying beams. Separately, `bellhop.f90` changes Cerveny taper radius from 50 to 30 wavelengths; that is a numerical-policy change, not automatically an accuracy improvement. |
| Geometric Gaussian field | Near-field width floor now divides `0.2 * frequency * travel_time` by wavelength; normalization is rewritten using `sigma1`. | A substantial output-changing algorithm update, confirmed by the isolation experiment. Do not adopt it under the label “boundary fix.” Semi/incoherent weighting also needs its own evaluation. |
| Grain-size bottom | `ReadEnvironmentBell.f90`, `ReflectMod.f90` and `misc/AttenMod.f90` share the grain-size conversion, validate Mz in [-1,9], use the local water sound speed at reflection instead of a hard-coded 1500 m/s, and assign calculated grain-size density. | Removes identifiable limitations in the old conversion; changes input acceptance and reflection amplitudes. Worth a dedicated small regression rather than a wholesale upgrade. |
| Range-dependent material attenuation | `bellhop.f90` uses the input `AttenUnit` for long-format boundary materials instead of hard-coded `'W '`. | A targeted units-handling correction worth checking against our material paths. |
| Arrival merging | `ArrMod.f90` guards amplitude-weighted merging with `abs(AmpTot) > epsilon(AmpTot)`. | Adds near-zero division protection; the threshold's effect on weak arrivals remains a separate consideration. |
| Runaway dynamic-ray state | `bellhop.f90` terminates rays when `abs(q(1)) > 1e100`, with an SSP-cusp comment. | A robustness guard, not a solution to boundary-contact instability or a demonstrated performance improvement. |
| 3D/Nx2D | `sspMod.f90` adds PCHIP dispatch for 3D; new/refactored stepping and reflection modules and a parabolic boundary module are present. | Source review only. BELLHOP3D/Nx2D was not built or numerically validated in this investigation. |

The bundled top-level HTML change history ends at March 2023; it is not a complete
release note for this archive. The inventory above comes from actual source diffs.
`GetSegz` is now a shared depth lookup, but still uses position/history, not ray
direction, to resolve segment ownership; the refactoring is not the missing
boundary-event fix.

## 4. Compatibility spot-check and unresolved SGB issue

To measure the cost of simply substituting the new oracle, the retained Rust
comparison executable was run against fresh latest-official products for the
37 existing golden environments and eight constructed N/P/S/Q SSP fields.
All 45 had passed against the old oracle. Existing comparison tolerances,
including the old separately reported ray minimum-step allowance, were retained.

| Input group | Match current Rust | Difference | Non-finite official output |
|---|---:|---:|---:|
| 37 golden environments | 12 | 25 | 0 |
| 8 constructed SSP fields | 0 | 8 | 0 |
| 4 original SGB S/I point/line cases | — | — | 4 |

This is a **49-input compatibility spot-check**, not a new full business matrix.
A difference is not automatically a bug in either version: it can be an intended
algorithm change, a changed step policy, or a defect needing separate diagnosis.
Successful comparisons are not end-to-end CLI acceptance runs.

The four SGB semi/incoherent cases still produce non-finite pressures: 10, 14, 10
and 14 non-finite real/imaginary components respectively. `InfluenceSGB` still
accumulates coherent complex amplitude before semi/incoherent pressure scaling;
the separate mathematical support problem has not been repaired by this release.

## Recommendation

1. **Keep v2023.5 as the frozen compatibility baseline.** Existing evidence remains
   valid for that version; do not describe it as latest-official numerical parity.
2. **Do not upgrade the whole oracle to solve the boundary problem.** The official
   release still has it and introduces unrelated field/SSP/material changes.
3. **Evaluate useful official corrections individually**, with a focused test for
   each, so changed outputs remain attributable.
4. If a boundary repair is authorized later, use a separately pinned, minimal patch
   on an official source baseline plus the analytic counterexample and current
   comparison harness. No third-party solver is necessary for that route.

## Retained evidence and limits

`target/bellhop-official-latest/` contains the downloaded archive/page/headers,
pristine source, original build, source hash comparison and per-file diffs,
probe scripts, all inputs/products/logs, analytic and field metrics, spot-check
results, and explicitly separated diagnostic reversion builds/results.

Important result files:

- `source-comparison.json`, `diffs/`, `build.log`
- `analytic-results.json`, `field-results.json`, `beam-results.json`
- `spot-check-results.json`, `spot-check/*/rust-comparison.log`
- `ablation-results.json`, `ablation-build/`, `ablation-runs/`
- `provenance.json`

No production solver/config/golden was changed. No third-party implementation,
full 219-path rerun, independent complex-field physical validation, latest KRAKEN
acceptance, or 3D acceptance is claimed. The numerical output of a newer official
release must not itself be treated as proof of greater physical accuracy.

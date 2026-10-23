# BELLHOP boundary-reflection investigation

**Status (2026-10-15): recorded and deferred by operator decision.** The current
BELLHOP implementation is accepted with known exceptions; the experimental
Fortran patch/checker has been withdrawn, not adopted. See the
[acceptance decision](../development/bellhop-business-coverage.md#operator-acceptance-and-deferred-boundary-work).
The findings below remain historical numerical evidence, not an active repair plan.

## Conclusion (2026-10-15 UTC)

**The boundary instability is a confirmed numerical defect shared with the pinned
Fortran algorithm, not merely a cross-platform discrepancy or an unproven Rust
translation error.** A constant-speed, flat-boundary analytic control reproduces
it in both implementations. One ULP of launch-angle perturbation changes a
14.4 cm trajectory error into roundoff-level error without changing the physical
reflection topology.

The appropriate direction is **explicit boundary events and consistent segment
ownership**, not matching one compiler's overshoot, reducing precision, increasing
tolerances, or simply reducing the minimum-step constant. A restricted contact
prototype fixes the analytic control, but is **not a validated replacement** for
the production integrator. No production solver changes were made in this
investigation.

The preceding numerical/tooling work was merged through
[PR #46](https://github.com/leihuang1/pelagic/pull/46), after CI
passed, at commit `10d5afd44ec45e9ccb7c0e7cd7243041f7d585dc`. This investigation
starts from that commit. The [business matrix](../development/bellhop-business-coverage.md)
remains **205 PASS / 9 FAIL / 4 ORACLE_NONFINITE / 1 EXCLUDED_3D**; none of the
controls below upgrades an original failure to PASS.

The subsequent [official 2026-08-29 source review](bellhop-official-2026-review.md)
confirms that the latest official 2D implementation still has this defect. Its
larger minimum-step floor increases the analytic counterexample's error to
1.44 m. Other official changes are documented separately; no third-party solver
is needed or adopted for that follow-up.

## 1. Scope and controls

Both implementations ran in the existing Linux amd64 business container:
Rust 1.99.0 / LLVM 23.1.1, release; GNU Fortran 12.2.0 with the unchanged pinned
oracle and flags. Docker runs on an Apple ARM64 host.

Three independent Rust diagnostic executables were built from source snapshots:

- **base:** unchanged merged solver, plus post-trace recording and raw complex-f32
  pressure output.
- **floor-1e6:** only the two `1e-4 * base_step` expressions in `reduce_step` change
  to `1e-6 * base_step`.
- **contact:** retains the original floor. After integration, detects an incoming
  near-contact within a local `64 * EPSILON * coordinate-product-scale` bound,
  projects position to that segment's plane, and carries a reflection flag.
  Does not project across a segment endpoint. Flat depth is assigned exactly.

The contact variant is deliberately incomplete: it corrects position **after**
the SSP evaluation and p/q jump calculation; it does not recompute the accepted
integration step, implement directional SSP/boundary segment ownership, or
resolve simultaneous events. The factor 64 is an experimental bound, not an
approved physical or error tolerance. This is a mechanism probe, not the proposed
production patch.

The base executable exactly reproduced all three previously recorded full-field
maximum component errors and exceedance counts. Original full-case ENV/resource
bytes were verified unchanged, as were all production Rust source hashes.
Distinct package names and executable hashes prevent Cargo from reusing a sibling
snapshot's binary. An initial same-name Cargo cache collision was detected before
experiments and corrected; those executables were not used for the results.

All variants retain existing solver budgets. Every subprocess has a 600-second
limit. All reported runs completed; no quota was increased. ENV/JSON/CLI product
acceptance was not rerun for this diagnostic-only work.

## 2. An analytic counterexample, independent of oracle agreement

Use a 100 m deep, constant-1500 m/s waveguide, a pressure-release surface, rigid
bottom, source depth 25 m, and a forward ray at -37 degrees. Trace to 1010 m
range with a 300 m requested step. This is intentionally a large requested step:
boundary-limited stepping must shorten it, and constant sound speed removes
ordinary curved-ray integration truncation error.

For positive horizontal propagation, the exact depth is the triangular-wave
fold of `25 + r * tan(theta)` into `[0, 100]`. The exact travel time is
`r / (1500 * cos(theta))`. Thus every saved point can be checked at its own range;
there is no interpolation or reference-implementation ambiguity.

An executable legacy input is:

```text
'Flat constant-c analytic control'
1000.0
1
'CVW'
0 0.0 100.0
0.0 1500.0 /
100.0 1500.0 /
'R' 0.0
1
25.0 /
1
50.0 /
1
0.5 /
'RB'
1
-37.0 /
300.0 110.0 1.01
```

At the first surface contact, **both original implementations** produce:

```text
range = 33.17612054051025 m, depth = +3.552713678800501e-15 m
```

The positive residual means “not reflected yet.” The following nominal zero step
is raised to `300 * 1e-4 = 0.03 m`, reaching:

```text
range = 33.200079605811666 m, depth = -0.018054450694557897 m
```

Reflection therefore occurs outside the water. The misplaced reflection plane
shifts the outgoing path; repeated events accumulate the error.

| Variant, -37 degrees, 300 m requested step | Maximum analytic depth error | Stored points |
|---|---:|---:|
| Pinned Fortran | 0.14443560555641 m | 23 |
| Rust base | 0.14443560555641 m | 23 |
| Rust floor-1e6 | 0.001444356055579 m | 23 |
| Rust contact prototype | 2.8422e-14 m | 19 |

Changing -37 degrees to either adjacent f64 value reduces the original Rust
error to at most `2.49e-13 m`, with 19 points. This is not grazing incidence, a
corner hit, or a physically different reflection sequence. Original Fortran
reproduces the 14.4 cm counterexample on the same platform. The diagnosis does
not depend on comparing two compilers.

The broader analytic sweep covers six angles (`±11`, `±37`, `±73` degrees), each
with zero/±1 ULP perturbation, and four requested steps (7, 10, 30, 300 m):
**72 inputs, 216 Rust traces, and 72 unchanged-Fortran traces**. In these selected
controls, the 7/10/30 m steps already have roundoff-level depth error; the 300 m
step exposes the defect. The maximum contact-prototype error over all 72 is
`2.05e-11 m`. This does not assert every smaller step is always safe.

Travel-time and tangent-norm checks can still pass while depth is wrong: in this
flat, constant-speed control, the horizontal velocity remains correct even when
the reflection is displaced. Matching bounce counts or travel time is not a
sufficient trajectory correctness test.

## 3. How large are the original full-field differences?

The following compares unchanged Rust and pinned Fortran at the **original**
steps, ray fans, and receiver grids. Each original complex pressure was read;
there was no subsampling. Differences are between implementations, **not errors
against physical truth**.

`|Delta TL| = |20 log10(|P_Rust| / |P_Fortran|)|`. Phase uses the absolute principal
angle of `P_Rust * conjugate(P_Fortran)`, in `[0, 180]` degrees. TL/phase statistics
exclude common zero cells. There are no one-sided zero cells in these three base
comparisons. The magnitude-filtered subset requires **both** magnitudes to be at
least `1e-5`; it is an explicitly reported diagnostic subset, not a new tolerance.

| Original field | Total cells | Nonzero pairs | TL p99 | Maximum TL difference | Maximum phase difference | Max TL, both magnitudes >= 1e-5 |
|---|---:|---:|---:|---:|---:|---:|
| DickinsB | 601,601 | 579,917 | 0.01052 dB | 0.85564 dB | 5.09169 deg | 0.11743 dB |
| DickinsFlatB | 201,201 | 198,312 | 0.00513 dB | 0.39144 dB | 2.86619 deg | 0.28399 dB |
| ParaBotTLGB | 501,501 | 312,348 | 0.00676 dB | 0.66168 dB | 3.95807 deg | 0.66168 dB |

No nonzero pair in these three original comparisons exceeds 1 dB. Most differences
are small, but they are not all confined to vanishing signals. The ParaBot maximum
occurs at `(1620 m, 1120 m)`, with magnitudes `1.78806e-5` and `1.92959e-5`.
DickinsB's largest TL discrepancy is much weaker: approximately `1.2e-8` versus
`1.1e-8` at `(78700 m, 2475 m)`.

Consequently, **“these three fields are catastrophically wrong everywhere” is not
supported**. Nor is a universal business-use acceptance claim: coherent processing,
interference nulls, and decision thresholds can require different error budgets.
The analytic instability and existing strict parity failures remain real.

## 4. Why the tempting small patches are not ready to ship

### Reducing the minimum-step floor

It reduces the analytic counterexample by approximately 100 times, but preserves
the unwanted extra step and the discontinuous contact decision. In the full fields
it also changes many boundary/SSP/segment transitions, not just the isolated beam.

### Contact-only correction

It fixes the flat analytic reflection and makes the selected ParaBot beam agree
at the original step. That is useful positive evidence for event-based handling.
However, correcting only contact detection cannot establish consistency of
integration state, interface ownership, and downstream beam influence.

Both probes produce larger deviations from the old oracle:

| Full field | Base: max component error / failures | floor-1e6: max / failures | contact: max / failures |
|---|---:|---:|---:|
| DickinsB | 1.5493e-7 / 545 | 4.8009e-4 / 524,121 | 2.1216e-6 / 10,734 |
| DickinsFlatB | 9.6305e-7 / 386 | 2.7116e-5 / 184,350 | 8.7119e-6 / 3,179 |
| ParaBotTLGB | 2.9472e-6 / 15,849 | 4.7061e-4 / 132,863 | 4.7084e-4 / 133,223 |

Failures still use the original `5e-8` component threshold. The floor probe's
maximum TL deviation from the old oracle reaches 45.13 dB in DickinsB; the contact
probe reaches 22.27 dB in ParaBot, including the magnitude-filtered subset.
**These are changes from a known-imperfect oracle, not proof that the new answers
are physically worse.** They establish that neither probe is a harmless parity
patch, and neither has enough validation for production acceptance.

## 5. Step refinement: parity is not convergence

First, the three selected failing beams were rerun at requested steps
`h, h/2, h/4, ..., h/64`, retaining original beam counts, selected beam indices,
and angular spacing. At all six smaller steps, original Rust and Fortran agree
exactly at the selected receiver. This does **not** generalize to the full fan.

Then all three complete fields were computed with unchanged fans/grids at
`h, h/2, h/4, h/8`, for original Fortran, original Rust, and the contact prototype.
Here `h=300 m` for both Dickins cases and `h=1020 m` for ParaBot. This adds
**27 complete refined-field computations** beyond the initial snapshots.

The following is the original Rust's successive complex-field relative L2 change:
`||P(h_new)-P(h_old)||_2 / ||P(h_old)||_2`. These are convergence diagnostics,
**not estimates of error against an exact wave solution**.

| Field | h -> h/2 | h/2 -> h/4 | h/4 -> h/8 | Rust/Fortran failures at h/2, h/4, h/8 |
|---|---:|---:|---:|---:|
| DickinsB | 8.2063% | 4.1633% | 2.5878% | 2, 549, 0 |
| DickinsFlatB | 7.9319% | 4.8153% | 2.8927% | 65, 25, 0 |
| ParaBotTLGB | 2.3619% | 0.3333% | 0.04508% | 1,462, 314, 334 |

Original Fortran has almost the same refinement changes. Thus the original
Rust/Fortran agreement is much tighter than their shared step sensitivity; making
the differential test pass does not establish physical convergence. At h/8 the
two Dickins fields pass the component comparison but still change by several
percent from h/4 in this norm.

The contact prototype is promising for ParaBot: its successive L2 changes are
`0.003267%`, `0.003425%`, and `0.00009277%`, versus the much larger original
changes above. However, this is still a fixed-ray-fan test, not beam-count
convergence or independent physical validation. In DickinsB its distance from the
original solver is not monotonic under refinement, reinforcing the need to test
all event/receiver interactions rather than accept one repaired critical ray.

## 6. Recommended repair boundary and acceptance criteria

The defect is split across code which chooses a step and code which later
rediscovers an event from rounded coordinates:

- `solver/integrator.rs::reduce_step` loses which constraint selected the step;
  the positive floor can then move it beyond that constraint.
- `step_2d` blends derivative estimates, constructs a position, evaluates SSP,
  and applies p/q jumps without retaining an explicit boundary-hit identity.
- `solver/mod.rs::trace_ray` chooses/refines segments and infers reflection from
  `begin_distance > 0 && end_distance <= 0` after the fact.
- `solver/ssp.rs` uses depth-interval history and different depth/range endpoint
  comparisons, not propagation direction, to decide interface ownership.
- `solver/boundary.rs` also chooses endpoint segments without ray direction.

A production repair should establish one consistent internal event contract:

1. Carry the accepted hit type and relevant segment/interface identity through
   integration. Define deterministic treatment of tied events and vertices.
2. Make the accepted endpoint and state updates correspond to that event. Assign
   representable axis-aligned boundary coordinates exactly; use explicit ownership
   for sloped boundaries rather than repeatedly guessing from a tiny signed dot
   product. A geometrically exact endpoint alone is not enough.
3. Select the outgoing SSP/boundary segment using propagation direction. Apply
   gradient jumps, reflection curvature, p/q updates, phase and bounce counts once,
   on the appropriate side of the interface.
4. Keep incoming/outgoing reflection states at the contact. Do not insert physical
   travel outside the domain merely to escape a zero step. Retain bounded
   non-progress detection for genuine degenerate cases.
5. Check all consumers of trace points. In particular, `simple_gaussian` currently
   estimates distance using `state_index * base_step + fraction * base_step`;
   changing the number of near-duplicate states can alter its field even without
   meaningful physical path change. This is a code-inspection finding here, not a
   freshly quantified SGB result. The separate SGB S/I intensity problem is also
   not repaired by boundary work.

The smallest convincing regression set needs flat and sloped analytic reflections,
±ULP inputs, forward/backward propagation, grazing/turning rays, coincident
SSP/boundary/vertex events, curved boundaries, and p/q/phase/bounce invariants.
It must then cover rays, coherent fields, arrivals, eigenrays, multiple sources,
and the existing business matrix. Preserve the old oracle results as compatibility
evidence; add independently justified new-behavior tests rather than overwriting
old goldens or weakening thresholds to manufacture agreement.

The policy choice is explicit: **a corrected canonical event behavior can
intentionally differ from the original Fortran's unstable edge cases**. Do not
silently promise both bit-level legacy behavior and corrected geometry. An ADR
and documented deviations should precede production rollout; a new user-facing
compatibility switch is not automatically required.

## 7. External corroboration, not a replacement oracle

Exa led to A-New-BellHope, which independently describes the same mechanism:
rounding selects opposite sides of a boundary, the minimum step amplifies the
residual, and receiver interpolation/phase/segment decisions can amplify it
further. Its repair combines boundary landing, explicit reflection decisions,
and direction-aware segment ownership.

Primary sources:

- [Boundary stepping and detailed edge-case analysis](https://github.com/A-New-BellHope/bellhop/blob/ae1a477287ec0200deab447a3c52c8a8ad511a65/README.md#boundary-stepping-changes)
- [StepToBdry2D implementation](https://github.com/A-New-BellHope/bellhop/blob/ae1a477287ec0200deab447a3c52c8a8ad511a65/Bellhop/Step.f90)
- [bellhopcxx/bellhopcuda accuracy discussion](https://github.com/A-New-BellHope/bellhopcuda/blob/main/doc/accuracy.md)

The Fortran fork was identified at commit
`ae1a477287ec0200deab447a3c52c8a8ad511a65`. It was inspected, not built or used as
a substitute oracle. Its implementation still contains thresholds and minimum-step
fallbacks; it should not be treated as a proof of universal stability. No external
source was copied into the production solver.

## Evidence and limits

All diagnostic scripts, snapshots, executables, inputs, outputs and logs are under
`target/bellhop-boundary/`:

- `prepare.py`, `build.sh`, `build.log`, `bin/`, `variants/`
- `experiments.py`, `fortran-analytic.py`, `convergence.py`
- `analytic-results.json`, `analytic-default-results.json`,
  `fortran-analytic-results.json`, `analytic/`
- `full-results.json`, `full-relative-l2.json`, `full/`
- `minimal-results.json`, `minimal/`
- `convergence-*.json`, `convergence/`
- `source-hashes.json`, `provenance.json`, `platform.log`

These are retained local diagnostic artifacts, not tracked release fixtures.
The analytic ENV and formulas above permit an independent minimal reproduction
without those artifacts. Metric self-checks cover identity, amplitude ratio and
phase; the raw-field reader validates SHD layout/counts/finiteness and reproduces
the existing full-field discrepancy counts.

No production boundary fix, exhaustive parameter-space proof, beam-count
convergence, independent wave-equation validation, or fresh 219-path acceptance
run is claimed. In particular, neither the sub-1 dB original differences nor the
analytic success of the contact prototype establishes blanket business suitability.

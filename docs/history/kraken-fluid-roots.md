# Fluid KRAKEN root search

Measured on 8 November 2026 against `48f43a4`, on an Apple M4 Pro (14 CPUs),
macOS arm64, Rust 1.99.0. These are indicative shared-desktop measurements,
not a portable speed guarantee or an end-to-end comparison with Fortran.

## Change and scope

Fluid KRAKEN now shares isolating Sturm bisections, then refines signed fluid
shooting in fixed eight-lane groups with safeguarded Illinois steps. It uses a
relative squared-wavenumber bracket width of `1e-13`, rather than counting down
to adjacent floats for every mode. Failed brackets and final count checks use
the original exact bisection for the affected mode. Elastic KRAKEN and KRAKENC
are unchanged. The [implementation map](../development/kraken-internals.md#fluid-root-search)
states the invariant, work accounting and fallback. No numerical selector or
adapter-specific implementation was added.

## Solver-only timings

`tools/perf/run.py bench`, the unchanged `tools/perf/cases.json`, one thread,
seven seeded interleaved rounds, one warmup and three samples per process.
Each row reports medians of 21 solver calls, not process/output time. No other
builds or experiments from this work ran alongside the timing. The machine was
shared, so these ratios remain indicative. The base binary came from a
`git archive 48f43a4` copy with a separate Cargo target directory.

| Case | Previous ms | New ms | Speedup |
|---|---:|---:|---:|
| Pekeris | 0.642 | 0.358 | 1.80× |
| Munk lossless | 10.148 | 5.941 | 1.71× |
| Sduct trapped | 49.511 | 32.836 | 1.51× |
| Surface duct | 65.520 | 50.313 | 1.30× |
| Large Munk FIELD | 332.081 | 265.620 | 1.25× |
| Gulf adiabatic | 86.752 | 63.949 | 1.36× |
| Gulf coupled | 74.886 | 51.831 | 1.44× |
| Broadband | 36.859 | 19.337 | 1.91× |
| Layered seabed | 0.542 | 0.522 | 1.04× |
| Elastic sediment, unchanged path | 10.573 | 10.616 | 1.00× |
| KRAKENC rigid-loss Pekeris, unchanged path | 2.839 | 2.853 | 1.00× |
| BELLHOP Munk TL, unchanged path | 201.486 | 201.775 | 1.00× |

## Transmission-loss changes

All eight changed example inputs were compared, including both Gulf FIELD
options. Fortran is the **pinned Linux amd64 v2023.5 Docker reference**, not the
native macOS compiler used in the research prototype. Full pressure arrays
were exported through `solve_frequencies`, without modifying the solver.
Geometry and per-profile mode counts were checked before comparison.

TL difference is `abs(20 log10(|p_b| / |p_a|))`. No nonzero-amplitude floor or
near-null mask was applied. Common zeros are counted separately and omitted
from the TL quantiles; a one-sided zero would make the maximum infinite. There
were no one-sided zeros. Gulf adiabatic has 501 common zeros; all other examples
have none. Values below are **maximum / p99, in dB**, per example (not a pooled
percentile across examples).

| Example (field cells) | Previous → new | Fortran → previous | Fortran → new |
|---|---:|---:|---:|
| Pekeris (9) | 2.57e-10 / 2.36e-10 | 9.35e-7 / 9.29e-7 | 9.35e-7 / 9.29e-7 |
| Surface duct (200,901) | 39.560 / 8.22e-6 | 37.558 / 1.62e-5 | 24.292 / 1.76e-5 |
| Gulf adiabatic (501,501) | 58.855 / 1.02e-6 | 63.983 / 1.72e-6 | 32.319 / 1.93e-6 |
| Gulf coupled (501,501) | 74.260 / 2.94e-4 | 69.440 / 2.86e-4 | 37.816 / 3.13e-4 |
| Munk modes (100,701) | 50.075 / 1.74e-6 | 50.772 / 2.03e-5 | 32.818 / 2.04e-5 |
| Broadband (101) | 0 / 0 | 1.28e-5 / 6.10e-6 | 1.28e-5 / 6.10e-6 |
| Layered seabed (70,641) | 13.913 / 0 | 19.974 / 9.98e-7 | 8.674 / 9.98e-7 |
| Rough surface (100,701) | 41.159 / 0 | 41.769 / 3.75e-6 | 25.410 / 3.73e-6 |

The large maxima in all three comparisons are surface near-null residuals, not
large changes in useful-energy pressure. Here, “near-null” means both cell
magnitudes are less than `1e-10` of their respective field peaks; this labels
cells but does not remove them. The previous→new maxima are at depth 0 m and
ranges 9,920 m (surface duct), 49,200 m (Gulf adiabatic), 46,200 m (Gulf coupled),
77,200 m (Munk), 7,780 m (layered seabed) and 53,200 m (rough surface).
At the worst, Gulf coupled changes `3.916e-21` to `2.022e-17` in pressure
magnitude. Pekeris's tiny maximum is not a near-null; broadband pressure is
unchanged even though its mode values changed.

Across these examples the largest absolute complex-pressure change is
`9.32e-10` previous→new and `3.01e-8` Fortran→new, below the unchanged `2e-6`
pressure gate. Smaller TL maxima against Fortran do not establish greater
physical accuracy. These data do not promise a uniform all-cell TL bound.

## Verification

- The complete unchanged performance inventory has 817 workflows. At threads
  1, 2 and 4: 2,451 configurations, zero mismatches and zero unexpected outcomes
  (2,121 successes, 318 expected load errors, 12 expected solve errors).
- Against the previous binary, 186 snapshots change: 110 FIELD/frequency API
  and 76 single-profile API workflows, all real fluid KRAKEN. BELLHOP, elastic
  KRAKEN and KRAKENC snapshots and all error outcomes are unchanged.
- All 645 successful KRAKEN/KRAKENC workflows retain their ordered mode counts:
  1,141 profile spectra, zero count changes. Failed loads/solves are not counted
  as successful spectra. Unit tests also compare 65 directly loadable fluid
  fixture meshes with the exact root path.
- Tests exercise isolating brackets, refinement accuracy, failed sign/final
  count/nonfinite checks, exact fallback, the rigid plane endpoint, cancellation,
  the work-limit boundary and the adjacent-float elastic count reversal.
- All 27 full-acceptance stages passed with unchanged tolerances and references,
  using `--allow-dirty`: diagnostic numerical evidence, **not clean-commit
  release acceptance**. No committed golden was regenerated.

## Independent review and work accounting

An independent review reproduced one regression. The search first charged its
attempted isolation, refinement and fallback work against the 2.5-billion
mesh-work limit. A 35-mode fluid case on a million-interval mesh, which the
previous release solved at 97% of that limit, then failed with `KR0302`.

The previous fluid bisection never charged its actual work. Before searching,
it charged a fixed 64 steps plus the shape allowance per mode and mesh point.
The new search now pays exactly that charge, in the same place, before either
search runs. So the work limit admits the same problems as before, at every
refinement mesh, and its own step caps bound the faster search's work. The
reviewed case solves again at 1, 2 and 4 threads. A unit test checks the charge
and the exact admission boundary on a case of several eight-mode groups. The
fix changes no arithmetic: the full inventory at 1, 2, 4 and 14 threads
(3,268 configurations) matches the reviewed build's snapshots exactly.

Raw snapshots, timings, all-cell metrics and acceptance logs remain local in the
root-search scratch directory and this worktree's `target/acceptance/`. Native
x86 performance and separately installed Python/MATLAB applications were not
measured in this campaign; the shared solver and existing adapter regression
checks were used. A clean-commit full acceptance is still required before release.

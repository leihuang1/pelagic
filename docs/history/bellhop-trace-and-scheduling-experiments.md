# BELLHOP tracing and scheduling experiments

## Result

Two changes were retained; five candidates were measured and rejected. Every
retained change leaves results byte-identical: all 62 BELLHOP inputs of the
perf kit's original inventory match `main` (`4bf17ec`) at 1, 2, 4 and 14 threads,
on aarch64-apple-darwin only.

1. **Inline the ray step's hot calls, and compute the receiver angle lazily.**
   `#[inline(always)]` on `SoundSpeedModel::evaluate` (three calls per step) and
   on `reduce_step` (two calls per step). The Cartesian geometric kernels compute
   a segment's `atan2` on its first receiver instead of for every segment.
   Arrivals and eigenrays run 1.4 to 1.5 times faster on one thread, and a
   shallow, isovelocity arrival run nearly twice as fast.
2. **Keep ray workers busy while results are applied.** Workers stay up for a
   whole source depth instead of being started and joined for every batch of 4
   rays per thread, and keep evaluating while the calling thread applies results
   in launch order. Multi-threaded runs are 1.13 to 2.0 times faster; one thread
   and peak memory are unchanged.

The measurements below come from one 14-CPU Apple M4 Pro (10 performance and 4
efficiency cores), Rust 1.99.0, release builds with default target CPU and
floating-point semantics. They are not a claim about other machines. x86-64 was
not measured, by the maintainer's decision for this round.

## Why tracing

The [previous experiments](bellhop-performance-experiments.md) timed
influence-heavy fields, where tracing was 6% of the time. A fresh one-thread
profile of `main` showed that this does not hold for other run kinds:

| Case | Tracing | Influence code | sincos and exp | atan2 and hypot |
|---|---:|---:|---:|---:|
| `munk-arrivals` | 87% | 5% | – | 7% |
| `bh-arr-big` | 72% | 19% | – | 7% |
| `bottom-transition` (hat-beam field) | 34% | 42% | 21% | 4% |
| `seamount` (Gaussian field) | 30% | 30% | 36% | 4% |
| `bh-tl-big` (Gaussian field) | 9% | 40% | 50% | 1% |

Under the byte-identity rule, libm calls cannot change, so a Gaussian field
cannot become more than about twice as fast on one thread; arrivals and
eigenrays had the most room.

## Method

- Byte identity: `tools/perf/run.py check` over the 62 BELLHOP inventory entries,
  against snapshots of `main`, after every candidate. Each candidate was built into
  its own target directory.
- Timing: `run.py bench`, one thread unless stated, 5 to 7 interleaved rounds of
  3 repetitions, paired speedups against the previous retained state. The keep
  rule of the previous experiments applied: at least 5% on the workloads the
  change targets, and no regression above 5% elsewhere.
- Source-line profiles: Instruments' Time Profiler (`xctrace record`), exported
  samples resolved with `atos -i` against a build with full debug information
  (`CARGO_PROFILE_RELEASE_DEBUG=2`) and a `dsymutil` bundle. Line-table-only
  builds lose the inlined frames, which hides where a fully inlined step spends
  its time. Debug information does not change the generated code.
- CPU counters (`xctrace`, CPU Counters template) on `munk-arrivals` after the
  first change: about 53% of issue slots did useful work, 30% waited on the back
  end and 15% were discarded by mispredicted branches.
- A shallow trace workload, `bt-arrivals`, was derived from `bottom-transition`:
  the same environment, with `bellhop.run` set to `arrivals` and a single receiver
  at 50 m depth and 4 km range. It exposed two regressions that the frozen case
  list did not.

## Retained 1: inline the step, defer the angle

Built up in three spikes, each byte-identical:

| Case | Lazy angle | + inline `evaluate` | + inline `reduce_step` |
|---|---:|---:|---:|
| `munk-arrivals` | 1.07× | 1.35× | 1.49× |
| `munk-eigenrays` | 1.07× | 1.36× | 1.52× |
| `bh-arr-big` | 1.05× | 1.31× | 1.41× |
| `bottom-transition` | 1.00× | 1.07× | 1.18× |
| `seamount` | 1.01× | 1.09× | 1.09× |
| `bh-tl-big` | 1.01× | 1.02× | 1.03× |

Against `main`, `bt-arrivals` is 1.95 times faster. Inlining moves no
floating-point operation: Rust does not contract or reassociate them.

## Retained 2: a pipelined scheduler

With one thread per case nothing changes, but single large cases scaled poorly:
`bh-tl-big` was 6.4 times faster on 10 threads than on one, `munk-arrivals` 4.2
times. Each batch started and joined its workers, then the calling thread
applied the batch's effects alone while the workers were idle.

`parallelism::for_each_ordered` replaces `map_prefix`. It keeps the batch's
bounds as a sliding window: no ray starts 4 rays per thread or more ahead of the
next to apply, or while waiting records hold 256 MiB or more. A ray's
`points_before`, which only lets a worker skip evaluating a ray that must fail
the ray-point limit, becomes a lower bound read when the ray starts; the calling
thread's limit check, which decides, is unchanged. Speedups against the first
change:

| Case | 1 thread | 4 threads | 10 threads | 14 threads |
|---|---:|---:|---:|---:|
| `bh-tl-big` | 1.01× | 1.13× | 1.35× | 1.46× |
| `munk-tl` | 1.00× | 1.14× | 1.39× | 1.47× |
| `bottom-transition` | 0.98× | 1.34× | 1.64× | 1.37× |
| `bh-arr-big` | 0.99× | 1.42× | 2.00× | 1.55× |
| `munk-arrivals` | 1.00× | 1.53× | 2.00× | 1.75× |

Peak RSS at 14 threads stayed within 2 MiB of the first change: `bh-tl-big`
53.3 → 51.6 MiB, `munk-tl` 29.1 → 30.2 MiB, `bh-arr-big` 8.5 → 8.4 MiB.

A first version hung when the calling thread panicked: nothing woke the workers
waiting for room in the window, and the thread scope waited for them. A guard now
stops the pipeline however the calling thread leaves, and a worker that panics
stops it too. Two unit tests force each case, a panic in a worker's item and a
panic while the calling thread applies one, and fail after a minute instead of
hanging; with either guard removed, its test fails.

## Rejected

Each was byte-identical and was reverted for its timing.

| Candidate | Effect | Why rejected |
|---|---|---|
| Reuse a step's end sample as the next step's start sample, skipping one of three sound-speed evaluations | 1.03× on Munk arrivals; 0.88× on `bt-arrivals`, 0.97× on `bottom-transition` | With one isovelocity layer an evaluation costs almost nothing, and carrying the sample across steps costs more |
| The same, also passing the sample into reflections and keeping it when a reflection does not move the ray | 1.06–1.07× on Munk arrivals and eigenrays; 0.97× on `bottom-transition` | Same cause |
| Neighbour-first, then bisected, sound-speed segment search on sorted grids | 1.03–1.04× on Munk arrivals; 0.98× on `bt-arrivals` | Below 5%, mixed |
| Gaussian receivers evaluated eight lanes at a time up to the libm calls | 0.98–1.03× on fields | The compiler emitted `fdiv.2d` and `fsqrt.2d`, but the out-of-order core already overlapped the scalar divisions |
| Skip the receiver angle in runs that do not read it | 0.99–1.01× | atan2 per hit segment is not a measurable cost in dense fields |
| Trace 2 or 4 rays side by side on one thread, one step each in turn | 1.06–1.08× on Munk arrivals; 0.92× on `bt-arrivals`, 0.96× on `bottom-transition` | Mixed; the core already overlaps much of one ray's step |

The segment-search candidate kept the reference's linear scan for unsorted grids,
and a unit test matched both searches over NaN, infinite and repeated nodes from
every starting segment; it would be a reasonable base if a case with many
sound-speed layers ever makes the search dominate.

## Final confirmation

Both changes together against `main`, with a second copy of `main` as a noise
control (its paired speedups stayed within 0.98–1.02×). Solver time, 7
interleaved rounds:

| Case | 1 thread | 4 threads | 10 threads | 14 threads |
|---|---:|---:|---:|---:|
| `munk-arrivals` | 1.51× | 2.03× | 2.40× | 1.93× |
| `munk-eigenrays` | 1.53× | 2.03× | 2.40× | 1.68× |
| `bh-arr-big` | 1.41× | 1.85× | 2.33× | 1.74× |
| `bt-arrivals` | 1.95× | 2.85× | 2.62× | 1.79× |
| `bottom-transition` | 1.17× | 1.49× | 1.76× | 1.49× |
| `munk-tl` | 1.04× | 1.17× | 1.39× | 1.44× |
| `seamount` | 1.11× | 1.28× | 1.50× | 1.50× |
| `bh-tl-big` | 1.04× | 1.16× | 1.38× | 1.42× |

End to end, `tools/perf/bellhop_cli.py` on the frozen case list, 7 rounds,
legacy output; all 168 invocations wrote artifacts identical to `main`'s:

| Case | 1 thread | 14 threads | Peak RSS at 14 threads, MiB |
|---|---:|---:|---:|
| `bh-arr-small` | 1.34× | 1.12× | 5.34 → 5.44 |
| `bh-arr-big` | 1.24× | 1.10× | 9.25 → 9.02 |
| `bh-tl-medium` | 1.03× | 1.23× | 30.52 → 31.16 |
| `bh-tl-big` | 1.04× | 1.35× | 54.55 → 54.89 |

End-to-end gains are smaller than solver gains: loading, publication and
process start take about 6–9 ms per run here, most of a small case's time.

## Verification

- BELLHOP crate: all tests pass, including thread invariance with 1-byte and
  4,096-byte ray records, limits, cancellation and the new pipeline tests (order,
  window and budget, first error, worker panic, apply panic, cancellation); the
  pipeline tests also passed 300 repeated runs.
- `run.py check`: 248 of 248 configurations match `main` for each retained commit.
- Workspace: `cargo test --locked --workspace --all-features` passed 379 tests,
  with 12 ignored; formatting, workspace Clippy with `-D warnings` and
  `tools/check-docs.py` pass.

## Limits

Byte identity was checked on aarch64-apple-darwin only; Linux, Windows and Intel
macOS need their own check before a release. Full pre-release acceptance was not
run. Raw measurements and profiles were kept locally, as the
[evidence rule](../development/reference.md#committed-evidence-and-run-artifacts)
requires.

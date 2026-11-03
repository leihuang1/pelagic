# BELLHOP performance experiments

## Result

Retained one optimization: `#[inline(always)]` on the per-receiver `record`
function in `crates/bellhop/src/solver/influence.rs`. No numerical expressions,
workload parameters, accumulation order, memory limits, or public APIs changed.

On this M4 Pro, the demanding transmission-loss case went from **221.986 ms to
175.567 ms end to end at 14 threads: 20.91% less time**. The paired speedup was
1.262×, with a process-block bootstrap 95% interval of 1.259–1.275×. The same
case's median peak RSS was effectively unchanged: **55.58 → 55.23 MiB**.
This is a machine/workload/compiler-specific measurement, not a universal claim.

Raw measurements, build/input hashes, source patches, profiling samples and
verification records are kept locally in `target/bellhop-experiments/evidence/`,
following the [evidence rule](../development/reference.md#committed-evidence-and-run-artifacts):
a fresh checkout does not have them. File names below refer to that archive.

## Protocol (fixed before solver changes)

Baseline: `a0bdb0ca05bec2c747bcc4c9f21f378e8252ffb6`, initially clean.
Implementation: native Rust `crates/bellhop`, not an external BELLHOP executable.
CLI path: `crates/cli/src/main.rs` → `cli::bellhop::solve` →
`bellhop::solver::run` → trace/influence → ordered effect application →
`output::bellhop::write_directory`. Library default is one thread; CLI default
is all available CPUs. No new backend or paid compute was used.

Host: Apple M4 Pro, 10 performance + 4 efficiency cores, 48 GiB RAM,
Darwin 27.0.0 arm64. Rust 1.99.0 (`b940084d7`), LLVM 23.1.1.
Release builds with line-table debug information, default target CPU and
floating-point semantics; no fast-math. The native-CPU experiment alone used
`RUSTFLAGS='-C target-cpu=native'`; that flag was not retained.

The frozen case list is [`tools/perf/bellhop-cases.json`](../../tools/perf/bellhop-cases.json):

| Case | Role | Rays | Receivers |
|---|---|---:|---:|
| `bh-arr-small` | small, sparse arrivals | 1,001 | 1 |
| `bh-tl-medium` | representative coherent Gaussian field | 1,000 | 501 × 201 |
| `bh-tl-big` | demanding coherent Gaussian field | 4,000 | 1,001 × 401 |
| `bh-arr-big` | arrivals regression guard | 2,001 | 100 × 49 |

**Primary endpoint:** end-to-end CLI wall time for `bh-tl-big`, 14 threads,
legacy output to a fresh directory. Also measure every case at 1 and 14 threads.
Retain only a ≥5% primary median reduction exceeding noise, no >5% median
regression on another selected workload, and passing correctness. Memory
trade-offs must be explicit.

### Measurement details

- Initial baseline and each isolated experiment: seven independent measured
  processes per case/thread/variant, following warm-up. Every experiment
  remeasured baseline in the same randomized, interleaved run.
- Final confirmation: 21 CLI rounds, including a second label pointing to the
  **same baseline binary** as a noise control; 11 solver-only rounds. Seeds,
  orders, commands, raw nanosecond timings, p10/p90 and binary/input hashes are
  preserved. Bootstrap resamples process-level paired ratios, not correlated
  repetitions inside a process. The speedup of paired medians need not equal
  the ratio of the two overall medians.
- `bellhop_cli.py` uses `/usr/bin/time -l` for per-process peak RSS (bytes on
  macOS). Its wall clock includes launching the time wrapper and CLI, loading,
  solving, publishing/syncing output, and process exit. Temporary-directory
  creation, output hashing, and cleanup are outside that timer. Each process
  writes to a fresh directory on the local filesystem. Every artifact's bytes
  must match the baseline warm-up output.
- The existing `run.py bench` measures solver calls only, including their
  allocations. It warms each process once, then records one solve in these
  experiments. Parsing, serialization and destruction of the last result are
  outside its timer. These separate runs are not an exact additive phase timer.
- No builds, tests or other benchmark jobs were run alongside timing trials.
  Ordinary desktop activity and heterogeneous-core scheduling remain sources
  of noise; cores were not pinned and no thermal policy was changed.

The first CLI build failed because Xcode's linker could not read the newer
Command Line Tools SDK (`arm64e.x1`). A minimal C program reproduced it.
Selecting Xcode's matching MacOSX26.2 SDK via per-command `SDKROOT` and
`CMAKE_OSX_SYSROOT` fixed the build. No system settings or dependencies changed.
All CLI variants used that SDK. The per-variant provenance generator records
those SDK settings on both binary entries, but applies them only to the CLI
build; the pure Rust perf build runs before that export. The failed build log
is preserved as `build-cli-baseline.log`.

## Correctness gate (fixed before solver changes)

Require **exact equality** of complete round-trip-float Debug snapshots against
the baseline, including signed zeros, ray/arrival order, pressure real/imaginary
components, arrival delays/amplitudes/phases/bounces and trajectory coordinates.
This strict gate is appropriate for compiler/scheduling experiments that should
not change arithmetic. Use the existing `run.py check` inventory restricted to
BELLHOP: 62 fixtures/examples/large benchmarks at 1, 2, 4 and 14 threads.
Errors must retain their diagnostics; timeouts/crashes never count as matches.
No changes to ray counts, step sizes, grids, tolerances or numerical precision.

Also retain the existing BELLHOP golden-reference tests and their tolerances.
For example, `field_golden.rs` compares pressure components to Acoustics Toolbox
v2023.5 within `5e-8`; that tolerance was not relaxed. Existing limit,
cancellation, irregular-grid and thread-invariance tests exercise both direct
application and recorded/replayed influence.

## Profiling and hypothesis selection

Five-second, 1 ms `sample` profiles were captured before changing the solver,
at both 1 and 14 threads. A CLI profile separated computation from publication.
The compressed original reports are in the local evidence archive.

For the single-thread demanding case, out of 3,925 main-thread samples:

1. Influence evaluation occupied approximately **93.4%**; tracing occupied
   **6.4%**. Even eliminating tracing entirely would give only about 1.07×,
   so changing integrator/SSP algorithms was not the first experiment.
2. The sine/cosine path under `record` accounted for approximately **43.3%**.
   The compiler already emitted combined `__sincos_stret`; rewriting separate
   `sin`/`cos` calls was therefore not a useful hypothesis. Eliminating all
   this work would bound speedup near 1.76×, but that is not an available exact
   optimization. Recurrences, approximate libm, and reduced precision would
   need a different numerical contract and were excluded.
3. Self samples in the Gaussian loop and recorder were about **37%** combined;
   exponential evaluation/stubs were about **12%**. This left room for improving
   data movement, call boundaries, and generated code without changing libm.
   These are overlapping conceptual costs, not independently removable terms.
4. In the short 14-thread CLI profile, 86/104 main-thread samples were solving
   and 18/104 were publishing. Twelve samples were in worker creation/join and
   fifteen in ordered effect replay. These are main-thread observations, not
   percentages of aggregate worker CPU time; startup before attachment is missed.

Final-run baseline medians reinforce the distinction: demanding-case solver
196.394 ms versus CLI 221.986 ms; small-arrivals solver 5.051 ms versus CLI
16.498 ms. The residual includes process/setup, output and teardown, and is
not solely filesystem I/O. This explains why improving sparse-arrivals compute
can have a much smaller effect on its end-to-end time.

External references informed the choices, not the correctness verdict:

- [Rust Performance Book: Inlining](https://nnethercote.github.io/perf-book/inlining.html)
  explains how a hot call boundary can prevent further optimization and warns
  that inlining can also regress performance. This motivated an isolated,
  measured experiment, not blanket inlining.
- [bellhopcxx/bellhopcuda performance notes](https://github.com/A-New-BellHope/bellhopcuda/blob/main/doc/performance.md)
  distinguish ray count, receiver layout, I/O and precision effects. They support
  measuring sparse arrivals and dense fields separately, and retaining precision.
- [A-New-BellHope numerical edge cases](https://github.com/A-New-BellHope/bellhop)
  describe sensitivity to which side of a boundary a floating-point step lands.
  This supported leaving boundary/integration arithmetic alone. Its alternative
  algorithms and backend were not imported into Pelagic.

## Three isolated experiments

Each candidate started from the baseline, **not** from the previous candidate.
Each passed all 248 case/thread checks, and every measured CLI output was
byte-identical to baseline. No combinations were needed or retained.

Primary-workload results from each experiment's own seven-round CLI comparison:

| Experiment | Baseline → candidate median ms | Paired speedup, 95% interval | Median peak RSS MiB | Decision |
|---|---:|---:|---:|---|
| Inline `record` | 222.708 → 174.941 | 1.274× [1.270, 1.297] | 55.19 → 54.91 | Retain and confirm |
| Rays/thread 4 → 16 | 225.084 → 210.675 | 1.055× [1.047, 1.112] | 55.61 → 108.03 | Revert: poor memory trade-off |
| Native CPU, unchanged source | 225.146 → 223.710 | 1.008× [0.991, 1.026] | 56.17 → 55.05 | Reject: no meaningful gain |

### 1. Inline the contribution recorder

Hypothesis: the per-receiver call forces candidate materialization and prevents
optimization with the influence loop. Add only `#[inline(always)]`, preserving
the recorder's cancellation, resource checks, target dispatch and arithmetic.

The gain survives separate solver-only and CLI trials. `llvm-nm` shows a
standalone `influence::record` in baseline and none in the candidate. An
additional single-thread run (one warm-up plus five solves) retired
124.434 → 110.674 billion instructions, **11.1% fewer**, and took
35.319 → 25.291 billion cycles. This supports a generated-code improvement,
not merely a favorable scheduling sample. It does not isolate exactly how
much came from call elimination, register allocation or subsequent optimization.

The final source adds a short rationale and a narrowly scoped
`clippy::inline_always` allowance: this normally discouraged annotation has a
measured reason here. Rebuilding after that lint-only edit produced **identical
complete CLI and perf binaries**, recorded in `lint-only-equivalence.json` and
`build-checked.json`. The confirmation trials therefore measure the retained
executable, not a stale earlier implementation.

### 2. Larger batches

Hypothesis: four rays per thread creates too many short-lived worker groups.
Increase only `RAYS_PER_THREAD` to 16; retain ordered replay, the 256 MiB batch
budget and 32 MiB per-ray fallback.

It helped sparse-arrivals solver time substantially (about 1.42× small and
1.37× large at 14 threads) but the primary end-to-end benefit was modest.
Primary RSS nearly doubled and medium-field RSS rose from roughly 31 to
70 MiB because more ray effects stay live. This is not worth changing the
shared default in this experiment. The constant remains **4**.

### 3. Native CPU code generation

Hypothesis: the arithmetic-heavy loop or output hashing might benefit from
host-specific scheduling/instruction selection with unchanged floating-point
semantics. Build both binaries with `-C target-cpu=native`, without source edits.

The primary gain was below 1%, with an interval crossing no change; other cases
were mixed. The dominant external libm functions are not recompiled by this
flag, and generic AArch64 code was already effective here. This is a plausible
explanation, not proof that native codegen never helps. No CPU-specific flags
or Cargo configuration were retained.

## Final confirmation

CLI medians over **21 independent rounds**. Positive change means less time;
RSS is the median of per-process peaks. Full p10/p90, minima/maxima, bootstrap
intervals and individual runs are in `final-cli.summary.json` / `.jsonl`.

| Case | Threads | Baseline ms | Retained ms | Time reduction | Peak RSS MiB, baseline → retained |
|---|---:|---:|---:|---:|---:|
| `bh-arr-small` | 1 | 39.929 | 40.141 | -0.53% | 3.78 → 3.83 |
| `bh-arr-small` | 14 | 16.498 | 16.896 | -2.42% | 5.42 → 5.44 |
| `bh-tl-medium` | 1 | 315.730 | 229.978 | +27.16% | 8.08 → 8.16 |
| `bh-tl-medium` | 14 | 56.890 | 47.054 | +17.29% | 30.91 → 31.27 |
| `bh-tl-big` | 1 | 1475.431 | 1069.670 | +27.50% | 17.38 → 17.38 |
| `bh-tl-big` | 14 | 221.986 | 175.567 | +20.91% | 55.58 → 55.23 |
| `bh-arr-big` | 1 | 96.544 | 97.431 | -0.92% | 6.00 → 6.05 |
| `bh-arr-big` | 14 | 41.967 | 41.851 | +0.28% | 9.31 → 9.34 |

Primary p10–p90: baseline **220.160–224.846 ms**, retained
**173.800–177.121 ms**. The duplicate-baseline control's paired speedup is
0.996× [0.991, 1.002], far smaller than the retained gain. Worst median slowdown
among the eight configurations is 2.42%, below the frozen 5% limit.

Solver-only confirmation (11 rounds): primary 14-thread median
**196.394 → 150.192 ms**, 23.53% less time; paired speedup
1.309× [1.287, 1.319]. At one thread it is 1,467.809 → 1,051.068 ms,
28.39% less time. Arrivals remain effectively unchanged; this is chiefly a
Gaussian-field optimization, not a blanket speedup across run kinds.

## Verification

- Baseline BELLHOP tests: **82 passed, 0 failed, 4 ignored**.
- Final `cargo test --locked -p bellhop -p native-output -p output -p cli`:
  **135 passed, 0 failed, 6 ignored**. Includes existing golden tests, cancellation,
  resource limits, thread invariance, output serialization and CLI publication.
- Five snapshot runs (baseline, inline, batch16, native, final): **248 successful
  case/thread executions each**, zero mismatches/timeouts/crashes. The four
  candidates were compared directly against baseline, not just against their
  own single-thread results. `checks.json` and compressed per-case logs retain
  input/output hashes and comparison outcomes. Large full snapshots remain
  local under `target/bellhop-experiments/`; they can be regenerated.
- All **504** measured final CLI invocations (baseline/control/retained) produced
  identical artifacts for the same input/thread configuration.
- Both Python harness self-checks pass; the perf crate's **7 tests** pass.
  New CLI-runner checks exercise timing/RSS extraction, changed-artifact rejection
  and nonzero-exit rejection. The solver change adds no non-trivial logic needing
  a new behavior test; existing exact-result/limit/cancellation coverage is used.
- Targeted all-targets Clippy with `-D warnings`, workspace formatting check,
  Python compilation, and `git diff --check` on tracked source pass. No unrelated
  source changed.

The six ignored tests require external reference artifacts: three fresh Fortran
ray/arrival/field comparisons, irregular-pairing Docker reference, original
KRAKEN broadband input, and fresh-reference CLI-layout comparison. They were
**not** run and are not counted as passes. Checked-in Fortran goldens did run.
No ordinary-test failures were observed. This is not full pre-release acceptance.

## Reproduce

Use the same hardware/toolchain for comparable numbers. The end-to-end runner
is intentionally macOS-specific; the existing solver runner is portable.
Build baseline and candidate into separate target directories, as explained in
[`tools/perf/README.md`](../../tools/perf/README.md). For example, from the candidate checkout:

```sh
git worktree add --detach /tmp/pelagic-bh-base a0bdb0ca05bec2c747bcc4c9f21f378e8252ffb6

CARGO_TARGET_DIR=/tmp/bh-base-perf cargo build --locked --release --manifest-path /tmp/pelagic-bh-base/tools/perf/Cargo.toml
CARGO_TARGET_DIR=/tmp/bh-new-perf cargo build --locked --release --manifest-path tools/perf/Cargo.toml

# On this host, required for CLI's C/HDF5 build:
export SDKROOT=/Applications/Xcode.app/Contents/Developer/Platforms/MacOSX.platform/Developer/SDKs/MacOSX.sdk
export CMAKE_OSX_SYSROOT="$SDKROOT"

CARGO_TARGET_DIR=/tmp/bh-base-cli CARGO_PROFILE_RELEASE_DEBUG=line-tables-only cargo build --locked --release --manifest-path /tmp/pelagic-bh-base/Cargo.toml -p cli
CARGO_TARGET_DIR=/tmp/bh-new-cli CARGO_PROFILE_RELEASE_DEBUG=line-tables-only cargo build --locked --release -p cli

# The 62 BELLHOP entries of the original inventory, byte for byte as archived:
python3 tools/perf/run.py inventory --original-only | python3 -c 'import json,sys; print(json.dumps([c for c in json.load(sys.stdin) if c["model"] == "bellhop"], indent=2, sort_keys=True))' > /tmp/bh-inventory.json
python3 tools/perf/run.py check --cases /tmp/bh-inventory.json --threads 1 2 4 14 --binary /tmp/bh-base-perf/release/pelagic-perf --output /tmp/bh-check-base
python3 tools/perf/run.py check --cases /tmp/bh-inventory.json --threads 1 2 4 14 --binary /tmp/bh-new-perf/release/pelagic-perf --baseline /tmp/bh-check-base --output /tmp/bh-check-new

python3 tools/perf/bellhop_cli.py --binary baseline=/tmp/bh-base-cli/release/pelagic --binary control=/tmp/bh-base-cli/release/pelagic --binary new=/tmp/bh-new-cli/release/pelagic --threads 1 14 --rounds 21 --seed 8675309 --output /tmp/bh-cli.jsonl
python3 tools/perf/run.py bench --cases tools/perf/bellhop-cases.json --binary baseline=/tmp/bh-base-perf/release/pelagic-perf --binary control=/tmp/bh-base-perf/release/pelagic-perf --binary new=/tmp/bh-new-perf/release/pelagic-perf --threads 1 14 --rounds 11 --reps 1 --warmup 1 --seed 8675309 --output /tmp/bh-solver.jsonl

cargo test --locked -p bellhop -p native-output -p output -p cli
python3 tools/perf/bellhop_cli.py --self-test
python3 tools/perf/run.py self-test
cargo test --locked --manifest-path tools/perf/Cargo.toml
```

Output paths must not already exist. Do not build or run other experiments while
benchmarking. Supply `--build-provenance FILE` with records keyed by the actual
binary hashes when creating new audit evidence; archived provenance describes
these particular builds, not arbitrary rebuilds. To replay an isolated source
experiment on a clean baseline checkout, add `#[inline(always)]` to `record` in
`crates/bellhop/src/solver/influence.rs`, or set `RAYS_PER_THREAD` to 16 in
`crates/bellhop/src/solver/mod.rs`. The native-CPU experiment changed only the
build flag.

## Limits and next experiment

No new GPU/backend, approximate math, dependency, integration change, permanent
thread pool or unrelated refactor was introduced. Cross-platform performance,
other CPUs/toolchains, HDF5 timing, and fresh external-reference runs were not
measured. Byte identity was checked only on aarch64-apple-darwin; Linux, Windows
and Intel macOS builds still need their own check before a release. The `llvm-nm`
and instruction-count observations, and the claim that the protocol and gate were
fixed before solver changes, are stated here without an archived command or
timestamp. The full fixture inventory protects correctness outside the four timed
workloads, but does not establish performance for those other workloads.

The most promising next scheduling experiment is reusing bounded workers across
existing four-ray batches while preserving ordered replay, cancellation and
memory caps. The batch16 measurements indicate potential value, but also show
why merely retaining more ray effects is not an attractive general solution.
That would be a larger change requiring its own profile, failure-path tests and
memory measurements; it was deliberately not bundled with this small proven win.

## Archive notes

After the runs, `PATH`, `HOME` and `TMPDIR` were removed from the archived run
manifests, and the `run_manifest_sha256` and baseline-results pins were recomputed.
Nothing else in those manifests or rows changed; `environment_sha256` still hashes
the original environment.

`source-sha256.json` records the runners as measured; for `bellhop_cli.py` only that
hash is kept, not its source. Afterwards, `bellhop_cli.py` gained failure rows,
process-group cleanup, `--binary` validation and a platform guard on its self-test,
and `run.py` stopped recording those variables plus `TMP` and `TEMP`, and started
hashing `bellhop_cli.py`. Running each process in its own session adds one `setsid`
call inside the timed window; nothing else on a successful run's timed path is
intended to differ.

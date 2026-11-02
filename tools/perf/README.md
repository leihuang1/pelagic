# Performance measurements

A standalone crate and driver for timing the solvers and for checking that a
change leaves every result byte-identical. It depends on `crates/kraken` and
`crates/bellhop` by path and has no HDF5, so it builds in seconds. Its own
workspace and lockfile keep it out of `cargo test --workspace`.

Nothing here replaces [full acceptance](../../docs/development/reference-workflows.md#full-pre-release-acceptance):
the kit compares a change with an earlier Pelagic, not with the Fortran reference.

## What it measures

| Tool | Scope |
|---|---|
| `pelagic-perf` | Solver calls only, with their allocation. Parsing, output, formatting and destroying the last result are outside the timer. `modal` is the single-profile `solve` API including FIELD, not a modes-only timer. |
| `resources` | One complete frequency sequence through `solve_frequencies`: time to the first result, total time, and result dimensions, with results dropped as they arrive (`stream`) or kept (`retain`). Wrap it in `/usr/bin/time` for peak memory. |
| `run.py` | `check`: run every input at several thread counts, keep a compressed snapshot of each complete result, and compare. `bench`: seeded, interleaved timing. |

Snapshots use the Debug format of the thread-invariance tests: shortest
round-trip floats, including signed zero. Load errors, solve errors and timeouts
are recorded as what they are and never count as numerical successes.

## Check that results are unchanged

Build each checkout into **its own target directory**. Cargo can reuse another
worktree's executable from a shared target directory, however plausible the
reported path looks.

```sh
# In a checkout of the base commit:
CARGO_TARGET_DIR=/tmp/perf-base cargo build --locked --release \
  --manifest-path tools/perf/Cargo.toml
python3 tools/perf/run.py check --binary /tmp/perf-base/release/pelagic-perf \
  --output /tmp/snapshots-base

# In the candidate checkout:
CARGO_TARGET_DIR=/tmp/perf-new cargo build --locked --release \
  --manifest-path tools/perf/Cargo.toml
python3 tools/perf/run.py check --binary /tmp/perf-new/release/pelagic-perf \
  --baseline /tmp/snapshots-base --output /tmp/snapshots-new
```

`check` exits 1 on any difference from the one-thread result, from the baseline
snapshot or from the baseline diagnostic. It also exits 1 on any crash or timeout,
even a repeated one, because those leave nothing to compare; `results.jsonl` keeps
their exit code and stderr. The default thread counts are 1, 2, 4 and the machine's
CPU count. Output directories must not exist yet. Inputs are
read from the checkout that holds `run.py`, so compare two builds of the same
inputs.

The inventory is every KRAKEN and BELLHOP fixture, example case and `bench/`
input, through both KRAKEN engines and APIs where they apply, plus the ENV/FLP
pairs in `pairings.json` that the tests use but whose stems differ. A pair is
listed because a test or script uses it; its expansion to every API and engine
is for coverage, and some of those are expected load errors. The table also
lists upstream and runtime-generated inputs that this kit cannot reach. Run
`inventory`, with `--original-only` (same-stem) or `--supplementary-only`
(pairs) if you like, to see the list, and pass a subset to `check --cases FILE`.

## Compare timings

```sh
python3 tools/perf/run.py bench --cases tools/perf/cases.json \
  --binary baseline=/tmp/perf-base/release/pelagic-perf \
  --binary new=/tmp/perf-new/release/pelagic-perf \
  --output /tmp/timings.jsonl
```

Each of the 11 rounds runs every case and thread count, with the case order and
the order of the binaries shuffled from a recorded seed. A process warms up once
and then takes three timings. The `.summary.json` beside the output has medians,
spread and, for any variant compared with one labelled `baseline`, a paired
speedup with a 95% bootstrap interval.

**A process is the sampling unit.** Its repetitions are not independent, so the
bootstrap resamples process-level ratios, not repetitions. Compare only within
one interleaved run; absolute times from separate runs differ with the machine's
load. Run one measurement at a time, with no builds alongside. Include a second
copy of the baseline binary under another label when an effect is small; that
shows how much difference the machine alone produces. Tiny cases (under a
millisecond) can be bimodal on a desktop.

## Measure memory

```sh
/usr/bin/time -l /tmp/perf-new/release/resources stream 14 json examples/cases/kraken/gulf/gulf.json  # macOS
/usr/bin/time -v /tmp/perf-new/release/resources retain 4 env krakenc case.env case.flp               # Linux
```

One invocation prints one JSON object. Exit 0 means every expected frequency was
produced. Peak memory covers the whole process, including loading and whatever
the allocator keeps.

## Provenance

`check` writes `run-manifest.json` beside its results and `bench` writes
`OUTPUT.jsonl.manifest.json`: binary and input hashes, the command, relevant
environment values and the state of the checkout. The manifest's `harness_tree`
describes the checkout running the driver, **not** how a binary was built. To
record that, pass `--build-provenance FILE`, a JSON object keyed by the binary's
SHA-256.

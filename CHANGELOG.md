# Changelog

User-visible changes in each release. Full notes, with verification details,
are on the [releases page](https://github.com/leihuang1/pelagic/releases).
Changes to public interfaces are listed under **Compatibility**. The rules are
in the [interface contract](docs/development/interface-contract.md).

## Unreleased

- **Faster durable CLI output, especially on Linux.** A result's files are now
  synced concurrently, up to 16 at a time, instead of one after another, in
  single `run` and in `run-many`. Each file still gets its own sync, so the
  durability guarantee is unchanged. Sync helpers leave room for a sweep's own
  open files: under a tight `ulimit -n`, each worker syncs one file at a time,
  as before. The CLI also raises its soft open-file limit to the hard limit at
  startup. Indicative measurements against the previous `main`, at one solver
  thread per case with legacy output into fresh result directories, on an AMD
  EPYC 7B13 Linux VM (overlay filesystem): 100 separate Pekeris runs, one after
  another, were 2.1 times faster; `run-many` sweeps of 200 Pekeris cases 1.6 to
  2.9 times faster at 1 to 24 workers; surface-duct sweeps 1.1 to 1.2 times; and
  one 101-frequency broadband result of 204 files 3.9 times faster. Such results
  now come from multi-profile runs, which keep indexed pairs; single-profile
  broadband runs write one shared pair. A 24-worker sweep that also writes HDF5
  took the same time. On a 14-CPU Apple M4 Pro, `run-many` workers that finish
  together share one drive flush: Pekeris sweeps on 8 and 14 workers were 1.2
  and 1.5 times faster, and the 204-file result 1.3 times. New result directories
  on Linux and macOS are staged beside the destination and renamed into place in
  one step. See
  [publication and interrupted batches](docs/guide/native-output.md#run-many-barriers-and-interrupted-batches).
- **Faster fluid KRAKEN root search.** Fluid problems now isolate modes with
  shared Sturm-count bisections and refine each one with a safeguarded secant
  (Illinois) iteration, which Sturm counts then check. A mode that fails any
  check uses the previous exact bisection. On one thread of a 14-CPU Apple M4
  Pro, solver-only runs are about 1.3 times faster for surface duct, 1.4 times
  for Gulf, 1.7 times for Munk and 1.9 times for broadband. Results are no
  longer bit-for-bit those of the previous release. Across the changed examples,
  the largest per-example 99th-percentile transmission-loss change is 0.0003 dB.
  Single cells at pressure nulls (below 1e-10 of the field's peak) change by up
  to 74 dB, and the largest such difference from the pinned Fortran reference
  falls from 69.4 to 37.8 dB. Mode counts are unchanged, results are still
  identical across thread counts, each refinement mesh costs the same against
  the work limit, and elastic KRAKEN and KRAKENC keep their existing search. See the
  [measurement record](docs/history/kraken-fluid-roots.md).
- **Faster fluid KRAKEN shooting.** The shooting function behind that search
  now keeps its eight lanes in registers and rescales growing values out of
  line. On one thread of an AMD EPYC 7B13, solver-only fluid runs are 1.02 to
  1.24 times faster with both the default and the x86-64-v3 build: Gulf about
  1.07 times, surface duct 1.13, broadband 1.2 and Pekeris 1.2. Elastic KRAKEN
  and KRAKENC are unchanged. Results are bit-for-bit those of the new root
  search. See the [measurement record](docs/history/kraken-fluid-roots.md#shooting-kernel).
- **Optional Linux x86-64-v3 CLI for clusters.** A separate `linux-x64-v3`
  archive is built for CPUs with AVX2, FMA and BMI2. On an AMD EPYC 7B13,
  one-thread solver runs are about 1.02 to 1.47 times faster, with
  byte-identical results across the measured inventory. Existing archives,
  Python wheels and MATLAB's bundled CLIs are unchanged. Older CPUs must use
  the baseline archive; see
  [CPU checks and source builds](docs/guide/cluster.md#choose-a-linux-x86-64-build).
- **Faster KRAKEN FIELD on Apple silicon.** Range-independent FIELD now sums
  16 ranges per loop instead of 8. On one thread of a 14-CPU Apple M4 Pro,
  solver-only runs are about 1.5 times faster for the large Munk benchmark,
  1.2 times for surface duct and layered seabed, and 1.15 times for elastic
  sediment. On an AMD EPYC 7B13 they take about the same time. Results are
  bit-for-bit unchanged.
- **Faster CLI result publication.** In interleaved local measurements on a
  14-CPU Apple M4 Pro, KRAKEN broadband runs are about 2 times faster end to end,
  and Gulf runs on 14 threads about 1.5 times. Sweeps of 200 Pekeris cases on
  14 workers are about 2.2 to 2.5 times faster with native output and 1.6 to 1.9
  times with HDF5. Sweeps of 50 surface-duct cases on 14 workers are about 1.6 to
  1.8 times faster. Durability, input protection, quotas and rollback are
  unchanged. Results and native output bytes are unchanged; HDF5 datasets and
  attributes are unchanged (its container bytes already vary between runs).
- **Faster BELLHOP ray tracing and multi-threaded runs.** On a 14-CPU Apple M4
  Pro, arrival and eigenray runs solve about 1.5 times faster on one thread, and
  hat-beam fields about 1.2 times. With several threads, workers keep tracing
  while results are applied, so runs solve 1.2 to 2 times faster on 4 threads and
  1.4 to 2.4 times on 10. End to end, `pelagic bellhop run` on the benchmark cases
  is up to 1.34 times faster on one thread and 1.1 to 1.35 times on 14. Peak
  memory is unchanged. Results were bit-for-bit unchanged on that machine. See the
  [experiment record](docs/history/bellhop-trace-and-scheduling-experiments.md).
- **Faster BELLHOP geometric-Gaussian fields.** On a 14-CPU Apple M4 Pro,
  `pelagic bellhop run` on field cases is about 1.4 times faster on one thread and
  1.2 to 1.3 times faster on 14, end to end. Arrival runs take about the same time.
  Results were bit-for-bit unchanged on that machine. See the
  [experiment record](docs/history/bellhop-performance-experiments.md).
- **Explicit real-KRAKEN mode reuse in Rust.** Prepare an immutable modal
  solution once, then synthesize range-independent fields at new receiver
  ranges and horizontal offsets without repeating the modal solve. All other
  inputs remain frozen, including `max_range_m`, so choose it for the farthest
  range you will query. Queries use the existing arithmetic and limits. See
  [mode reuse](docs/kraken/prepared-modes.md). Existing solve APIs are unchanged.
- **Unused single-file HDF5 functions removed from Rust.** Nothing in this
  repository called `output::bellhop::write_hdf5_atomic`, `output::kraken::run_legacy`
  or `output::kraken::run_json`; the CLI already publishes through result
  directories, and its behaviour does not change.

### Compatibility

- Multi-frequency KRAKEN/KRAKENC native output now uses one `CASE.mod` and one
  `CASE.shd`, in Fortran's multi-frequency layout, when every block has exactly
  one profile and shares the native headers and source/receiver grids. All
  single-profile broadband `.env` inputs qualify. Multi-profile runs retain
  byte-identical `CASE.fNNNN.mod/.shd` pairs so unmodified pinned FIELD and MATLAB
  mode readers can traverse them; incompatible headers also retain indexed pairs.
  This applies to the CLI and Python; consumers should use the manifest rather
  than assume indexed filenames. Single-frequency bytes and HDF5 are unchanged.
  This public filename change requires a minor release before 1.0.
- KRAKEN `run` and `run-many` now report a JSON solver mismatch under the field
  `kraken.solver`, the JSON path, as `validate` and `export` already did. It was
  `mode_solver`.
- KRAKEN `validate`, `export` and `run -` now word a read failure as
  `unable to read input: ...`, as `run FILE` already did.
- Python `pg.kraken.load` of a JSON file now reports what the CLI does.
  Diagnostics name the real path instead of `request.json`. A solver mismatch has
  the field `kraken.solver`, the CLI's message and a source location, and is
  reported before the 1 MiB check on the case's canonical form. A file that cannot
  be opened was already `KR0001`, but a failure after opening it, such as non-UTF-8
  content, was `PG0001` and is now `KR0001` too. Both now have a source location and
  the `unable to read input:` prefix. The `flp` rejection has the CLI's message and
  a source location.

## 0.6.1 — 2026-11-02

- **Small KRAKEN solves no longer start threads they cannot use.** Mode search,
  mode shapes and FIELD stay on the calling thread when the work is too small to
  repay starting workers. The Pekeris example is about 10% faster on 4 and 14
  threads. Results are bit-for-bit unchanged.
- **Faster KRAKEN mode shapes.** Real KRAKEN and KRAKENC compute first-mesh mode
  shapes in parallel, and real KRAKEN fluid modes share each pass over the mesh
  in fours. On a 14-CPU Apple M4 Pro, the surface-duct and Gulf examples are about
  10% faster on one thread and 2.1 to 2.7 times faster on 14. Results are
  bit-for-bit unchanged. More threads hold more shape scratch: up to 64 MiB in
  all, then about 60 bytes per mesh row per thread.
- **Faster range-independent KRAKEN FIELD.** Modal sums for eight ranges now
  share a loop, which the compiler packs into vector instructions. A 962,000-point
  Munk field is about 14% faster on one thread. Each range still adds its modes in
  the same order, so results are bit-for-bit unchanged.
- **KRAKEN frequencies and range-dependent profiles run side by side on eight
  or more threads, where they are independent.** After the first, they share the
  threads, and results still arrive in order. On 14 threads, the 101-frequency
  broadband example is about 7.7 times faster than in 0.6.0, and the Gulf example
  goes from 2.2 to 3.2 times faster. Real KRAKEN with finite elastic layers or an
  elastic surface half-space stays one at a time, because each solve continues
  from the last. Results and diagnostics are bit-for-bit unchanged. In exchange,
  about 128 MiB of finished results can wait their turn: streaming 64 mid-size
  frequencies on 14 threads now peaks at 175 MiB rather than 19 MiB.
- **Faster KRAKEN mode search.** The interior of each Sturm count now uses packed
  divides, with no CPU-specific build or unsafe code. On one thread of the same
  machine, the surface-duct, broadband and Gulf examples are 1.3 to 1.5 times
  faster. Each lane performs the same IEEE operations in the same order, so results
  are bit-for-bit unchanged.
- **KRAKEN on x86-64.** On a 9-CPU AMD EPYC 9V74 cloud VM, this release's KRAKEN
  changes together make heavy cases 1.76 to 2.32 times faster on one thread and
  2.41 to 3.88 times faster on 9, and the broadband example 8.02 times faster on 9.
  Results are bit-for-bit unchanged there too.

## 0.6.0 — 2026-10-29

- **Multi-threaded solves with unchanged results.** BELLHOP shares the rays of
  each source between threads. KRAKEN shares its mode search and FIELD
  receivers. Results and diagnostics are bit-for-bit the same as with one
  thread, and every native and HDF5 product of the examples is unchanged. On a
  14-CPU machine, `pelagic bellhop run` on the field examples is 5 to 8 times
  faster, and a 960,000-point KRAKEN field 25 times faster. One thread uses the
  same memory as before. More threads let BELLHOP hold up to 256 MiB plus
  32 MiB per thread of ray records, whatever the grid size.
- **Faster KRAKEN FIELD on one thread.** Modal phases are computed once per
  range instead of once per receiver, and range-dependent FIELD finds each
  receiver's mode-shape samples once per profile. With one thread that
  960,000-point field is 4 times faster, and the Gulf of Mexico example 1.8
  times faster.
- **Faster KRAKEN mode search.** Each pass over the mesh now advances the
  bisection of up to eight modes instead of one, so their Sturm counts
  overlap. Results are bit-for-bit unchanged. With one thread on an Apple M4
  Pro, `pelagic kraken run` is 3.3 times faster on the surface-duct example,
  2.7 times on the 960,000-point field and 1.8 times on the Gulf of Mexico
  example.
- **Faster range-dependent KRAKEN FIELD.** Adiabatic and coupled FIELD
  interpolate each receiver depth's mode shapes once per profile instead of
  once per range, and share receiver depths and adiabatic range factors
  between threads. Results are bit-for-bit unchanged, and peak memory stays
  within 3 MiB of before. On an Apple M4 Pro, `pelagic kraken run` on the Gulf
  of Mexico example takes 0.17 s instead of 0.30 s with one thread and 0.12 s
  instead of 0.27 s with all 14; the coupled example takes 0.19 s instead of
  0.22 s, and 0.11 s instead of 0.16 s.
- **Faster BELLHOP geometric beams.** Hat and Gaussian geometric beams in
  Cartesian coordinates find the receiver depths each ray segment can reach
  by binary search when the depths are in increasing order, instead of testing
  every depth. Each coherent contribution also skips `exp(0) = 1` when the
  medium has no attenuation. Results are bit-for-bit unchanged. With one
  thread on an Apple M4 Pro, `pelagic bellhop run` on the Munk and seamount
  examples is 1.2 times faster, and on a 401 × 1001 field 1.6 times.
- **Thread controls.** The CLI takes `--threads N` or `PELAGIC_THREADS` and
  defaults to the available CPUs. Python adds `case.run(threads=)` and
  `run_many(..., threads=)`, which by default shares the CPUs between workers.
  MATLAB adds `run(c, 'Threads', n)`. `pelagic-server` adds
  `BELLHOP_SOLVER_THREADS`, which defaults to one thread per request.
- **Python sweeps split the CPUs by themselves.** `run_many` starts one worker
  per case, up to one per available CPU, and gives each solve the CPUs left
  per worker. On a 14-CPU Apple M4 Pro, 28 Gulf of Mexico cases take 0.8 s
  instead of 5.7 s with the previous default of one worker. With
  `return_exceptions=True`, a failed case's `PelagicError` is yielded in its
  place and the other cases carry on.
- **`pelagic bellhop run-many` and `pelagic kraken run-many`** solve many
  cases in one command, each into `DIR/<stem>/` exactly as `run` writes it.
  Cases come from files, directories or `--list`. A failed case doesn't stop
  the others. Each case adds a line to a status file, and `--resume` skips
  cases whose input files, options and results are unchanged. `--shard I/N` splits
  the cases between the tasks of a cluster job array. By default the CPUs are
  split between cases as in Python: 28 Gulf of Mexico cases take 1.1 s,
  against 6.6 s running them one after another with `pelagic kraken run`.
- **MATLAB sweeps with `runMany`.** `pelagic.bellhop.runMany` and
  `pelagic.kraken.runMany` solve a cell array of case structs or file paths
  with one `run-many` call, sharing the CPUs between cases without the
  Parallel Computing Toolbox. Results come back in input order, as `run`
  returns them. A failed case leaves `[]` and an entry in the second output,
  `failures`; with one output, the first failure is thrown after every case
  has finished. `'Output'` keeps the result files. It works in Octave too. 28
  Gulf of Mexico cases take 2.0 s, against 7.5 s in a loop of `run`.
- **Cluster sweeps guide.** [Sweeps on a cluster](docs/guide/cluster.md)
  covers Slurm and PBS job arrays with `run-many --shard --resume`, CPU
  binding, memory sizing, resubmitting killed tasks, and `xargs` or GNU
  `parallel` on one machine. New examples: `examples/cli/slurm-sweep.sh`, run
  shard by shard in CI, and `examples/cli/sweep-status.py`, which lists done,
  failed and missing cases, and cases whose status files disagree.

**Compatibility**

- The CLI, Python `case.run()` and MATLAB `run` now use every available CPU by
  default. Set `--threads 1`, `PELAGIC_THREADS=1` or `threads=1` to keep the
  previous single-threaded behaviour, for example when running many processes
  side by side.
- Python `run_many` now defaults to one worker per case, up to the CPU count,
  instead of one worker, so it can hold that many results in memory. Pass
  `workers=1` for the previous behaviour.
- New public format: the `run-many` status file, version 1, also reported by
  `pelagic version --json` as `run_many_status_version`. New code `PG0404`:
  the status file could not be written.

## 0.5.0 — 2026-10-25

- **Case JSON version 2**, designed to be written by hand. It follows the
  `.env` with names instead of positions: `media` with sound-speed rows
  `[depth, cp, cs, rho, ap, as]`, `surface`/`bottom` boundaries, `source`,
  `receivers`, and a `bellhop` or `kraken` block. Lists can be generated from
  their end points (`{"from": 0, "to": 100, "count": 501, "unit": "km"}`),
  defaults follow the `.env`, and BELLHOP and KRAKEN share names and units.
  `munk-tl.json` drops from 271 lines to 79; the 101-frequency KRAKEN
  `broadband.json` from 5459 to 42.
- **KRAKEN JSON keeps the loss recipe.** Losses are stored in the `.env` unit,
  with the volume model and power law, and are converted for each frequency by
  the same code as `.env` input. A broadband case is one environment and a
  frequency grid; range-dependent profiles list only what they change.

**Compatibility**

- Case JSON is now schema version 2 (`"pelagic": 2`, plus `"model"`), for both
  models. Version 1 documents are no longer read: re-export them from their
  `.env` files with `pelagic bellhop export` or `pelagic kraken export`.
  `pelagic version --json` reports `case_schema_versions` of 2.
- Python `Case.to_dict()`/`from_dict()` and MATLAB `load`/`run` structs use the
  version 2 field names. Validation diagnostics name version 2 JSON paths, such
  as `bellhop.angles` and `media[0].ssp`.
- Units changed where version 1 differed between models: BELLHOP density is
  g/cm³ (was kg/m³), and KRAKEN reflection-table phase is degrees (was radians).
- KRAKEN JSON has one FIELD geometry for all frequencies. Version 1 allowed
  geometry, profiles and solver settings to differ per frequency block.
- The MATLAB codec no longer needs `+internal/array_fields.m`, which is removed.

## 0.4.1 — 2026-10-23

- **Examples:** 24 ready-to-run cases, each as Acoustics Toolbox files and as
  JSON. They cover rays, eigenrays, arrivals, bathymetry and altimetry,
  reflection tables, beam patterns, elastic, layered, ice-covered and rough
  boundaries, leaky modes, range-dependent adiabatic and coupled modes, and
  broadband pulses. There are 16 Python, 8 MATLAB, command-line and HTTP
  scripts, all run in CI.
- **Documentation reorganized:** getting started, separate Python, MATLAB and
  CLI guides, troubleshooting with error codes, and maintainer guides for
  releases and interface stability.

**Fixed**

- The arlpy backend returned wrong range coordinates (the DataFrame's columns)
  from `compute_transmission_loss`. arlpy 1.9 reads receiver ranges as float32,
  but Toolbox v2023.5 `.shd` files store them as float64. Pressure values were
  correct.

**Compatibility**

- `examples/field-g.json` moved to `examples/cases/bellhop/field-g/field-g.json`,
  and `examples/kraken-pekeris.json` to `examples/cases/kraken/pekeris/pekeris.json`.

## 0.4.0 — 2026-10-23

- **Python package** on PyPI (`pip install pelagic`). Runs BELLHOP and
  KRAKEN/KRAKENC in-process and returns read-only NumPy results. Includes
  parallel sweeps (`run_many`) with Ctrl-C cancellation, native file output
  without HDF5, pandas/xarray conversion and an opt-in arlpy 1.9 backend.
- **MATLAB/Octave toolbox** (`pelagic-vX.Y.Z.mltbx`), with the CLI for all five
  platforms bundled.
- **macOS and Linux CLI downloads**, alongside Windows.
- **CLI machine protocol:** `-` reads JSON from stdin, `--diagnostics json`
  gives NDJSON diagnostics, `pelagic version --json` reports versions, and
  there is a new `pelagic kraken validate`.
- `CITATION.cff` for citing a release.

**Compatibility**

- Case JSON v1, BELLHOP HDF5 v3 and KRAKEN HDF5 v1 are unchanged.
- Diagnostic `field` names now match the JSON schema:
  - `frequency` → `frequency_hz`
  - `run_options.beam_family` → `run.beam_family`
  - `trace.launch_count` → `trace.launch_angles_degrees`
- Malformed JSON now reports the offending path instead of `json`. Codes and
  exit codes are unchanged.

## 0.3.0 — 2026-10-16

- Rough fluid interfaces (surface, internal, bottom) in KRAKEN and KRAKENC.
- More reliable KRAKENC mode discovery for separated ducts and rigid tops.
- Five BELLHOP beam-normalization and precision fixes. Non-finite scaled
  fields are rejected (`BH0302`).

**Compatibility**

- Existing JSON v1 cases are unchanged and default to smooth interfaces.
- KRAKEN HDF5 v1 gains additive roughness attributes.

## 0.2.0 — 2026-10-07

- One `pelagic` CLI for BELLHOP and KRAKEN/KRAKENC, with legacy or JSON input.
- Native `.ray`/`.arr`/`.shd`/`.mod` output alongside HDF5
  (`--format legacy|hdf5|both`), and protected result directories.
- Windows x64 download.

**Compatibility**

- The old per-solver executables are replaced by `pelagic bellhop` and
  `pelagic kraken`.
- `--output` names a directory, and `--overwrite` is removed.
- Native files are now the default; use `--format hdf5` for the previous
  output.

## 0.1.0 — 2026-09-18

First release, as `bellhop-rs`: BELLHOP 2D with legacy and JSON input, HDF5
output and an HTTP server.

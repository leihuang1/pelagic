# Architecture

## Crates

Seven workspace crates. None is published to crates.io; the Python package is
distributed through PyPI.

```text
cli (pelagic)          ──> bellhop, kraken, output
server (pelagic-server) ──> bellhop, output
output (HDF5)          ──> bellhop, kraken, native-output
python (pelagic._core) ──> bellhop, kraken, native-output
native-output          ──> bellhop, kraken
bellhop, kraken        ──> (no workspace dependencies)
```

| Crate | Owns |
| --- | --- |
| `bellhop`, `kraken` | Case intake, input parsing, validation into immutable Cases, solvers, results, diagnostics |
| `native-output` | Toolbox-format writers (`.ray`, `.arr`, `.shd`, `.mod`), the KRAKEN frequency-block stream and result-directory publication. No HDF5 |
| `output` | BELLHOP HDF5 v3 and KRAKEN HDF5 v1 writers. Re-exports the directory API |
| `cli` | The `pelagic` program: argument handling per model, behind one dispatch |
| `server` | The BELLHOP-only HTTP adapter |
| `python` | The PyO3 extension behind `python/pelagic` |

Outside `crates/`:

| Path | Owns |
| --- | --- |
| `python/pelagic` | The Python API: Cases, result dataclasses, errors, `run_many` |
| `matlab/+pelagic` | The MATLAB/Octave API; `+internal` holds the CLI process and JSON codec |
| `tools/reference` | The Fortran reference image and comparison drivers |

## Design rules

- **The two solvers are independent.** There is no shared Case, result model,
  JSON schema, solver trait or root-finding engine across BELLHOP and KRAKEN.
  Code that looks similar keeps its own numerical locality, because the
  reference arithmetic differs.
- **Adapters add no physics.** The CLI, server, Python and MATLAB move cases in
  and results out. Validation happens once, in the solver crate. See the
  [interface contract](interface-contract.md#adapter-architecture).
- **Adapters load through the Case intake.** `kraken::input` and `bellhop::input`
  take a file path or a reader and return one value: cases, snapshots and consumed
  inputs. The library never touches process stdin; the CLI maps `-` to the reader
  entry in one small helper. Three paths stay outside it. The server, which already
  holds bytes, parses them with `bellhop::json::load_case_document`, in the one
  request extractor that every `/v1/` route takes. Python's
  `from_json`, which has no file, keeps the name `request.json`. `pg.bellhop.load`
  of a JSON file keeps its own read and parse, so that its diagnostics stay unchanged.
- **Results are serialized as they are.** Writers never re-solve, interpolate,
  pad or reconstruct inputs.

## Solver crate layout

Both solver crates follow the same layout:

```text
crates/{bellhop,kraken}/src/
  lib.rs          public exports
  model.rs        unvalidated input types
  case.rs         validation and immutable Case invariants
  diagnostic.rs   diagnostics
  result.rs       numerical products
  input/          Case intake, legacy and JSON parsing, auxiliary-file acquisition
  solver/         the numerical implementation
```

Files follow concepts rather than line counts. Input material conversion lives
in KRAKEN's `input/`; the solver only sees canonical solve-frequency material.
The [KRAKEN internals](kraken-internals.md) page maps the solver in detail, and
the [porting map](porting-map.md) links Rust modules to the Fortran sources.

## Output paths

- `output/src/{bellhop,kraken}.rs` write the two HDF5 schemas separately.
- `native-output/src/{bellhop,kraken}.rs` write the native products. `native.rs`
  shares only record and primitive I/O.
- `native-output/src/directory.rs` publishes a file group: verified ownership,
  exclusive lock and scratch, and rollback on handled errors. The rules are in
  [native output](../guide/native-output.md#ownership-quota-and-failure-contract).
  On Apple platforms, `directory/gate.rs` admits at most four `--no-sync`
  publications at a time to `begin`, installation and cleanup, including cleanup
  after a failed writer. Caller code (the writer, its result and error
  conversion) always runs outside the gate.
- BELLHOP's `result::FieldGrid` yields pressure depth-major for native SHD and
  Python, without changing the range-major solver/HDF5 layout.
- `native-output/src/kraken/stream.rs` owns the KRAKEN frequency-block stream:
  one solve iterator and, for each block, the HDF5 block writer's write, the
  native pair, the report lines and their flush, the directory quota, then the
  writer's check. The order decides which diagnostic wins and what a failure leaves, so its
  tests live there, without HDF5.
  Each block is written, then dropped, before the next result is taken from the
  iterator, which may already have solved it; the solver state that carries across
  frequencies is kept.
- `output::kraken` is the stream's HDF5 block writer. Its header precedes the stream,
  which creates the report, and its totals follow the last block. Its check
  flushes again after the block's group closes, which can grow the file.
  Python writes native files from finished results, so it uses the same
  `native_output::kraken::Writer` as the stream, without solving again. That writer
  combines only single-profile blocks with compatible native headers, appending
  each frequency's MOD/SHD payload without retaining results or moving existing
  records. The pair stays open only with two slots from the publication sync
  helpers' shared descriptor budget; tight-limit workers close and reopen it.
  Multi-profile runs keep indexed single-frequency pairs for unmodified
  pinned FIELD/MATLAB reader compatibility. Each block is flushed and
  quota-accounted from bytes written, including growth of the shared MOD/SHD
  files. The report counts only unbuffered bytes; HDF5 still refreshes its size
  from the filesystem at the same checkpoints.
- HTTP writes private HDF5 files and keeps its own protocol. It does not use the
  CLI's directory publication. Its response types are also the schemas that
  `/openapi.json` publishes; the arrival and field types are views over the solver
  result, so serializing them copies no arrays. A test checks a response for every
  published route and status against that document. The `/v1/run` and `/v1/field`
  500s are the exception: no request in the test reaches them, so it checks
  constructed error bodies.

## Concurrency and cancellation

The solvers are deterministic: **every result and diagnostic is bit-for-bit the
same for any thread count.** The count is thread-scoped, like cancellation:
adapters wrap a solve in `bellhop::parallelism::with_threads` or
`kraken::parallelism::with_threads`, and the default is one thread. The CLI and
Python default to the available CPUs and the server to one thread per request.

Each crate's `parallelism::map` evaluates independent items on scoped threads
and returns them in item order. Workers inherit the caller's cancellation token,
and an item never fans out again. A call starts at most 1024 workers, and a
thread the system refuses leaves its share to the others. KRAKEN's map also takes
an estimate of its total work and stays on the calling thread when that is too
small to repay starting workers. Order-dependent work stays on the calling
thread:

- **BELLHOP** traces rays in parallel. Each ray's influence is
  recorded as `Effects`: single-precision field contributions, arrival
  candidates or eigenray hits. The calling thread applies each ray in
  launch order. Pressure sums, arrival merging, resource limits and error
  precedence are therefore those of one thread. With one thread, each ray is
  applied as it is evaluated, as before. Otherwise the workers stay up for the
  whole source depth and keep evaluating while the calling thread applies, which
  it does whenever the next ray is ready, evaluating a ray itself otherwise. No
  ray starts 4 rays per thread or more ahead of the next to apply, or while the
  rays waiting to be applied hold 256 MiB of records or more. A worker gives up a ray
  whose records reach 32 MiB; the calling thread then applies that ray, and
  every later one, as it evaluates it, so memory stays bounded on any grid. A
  ray beyond the ray-point limit is rejected before its influence is
  evaluated. Applied effect buffers are reused, because some allocators keep a
  freshly grown buffer per ray.
- **Fluid KRAKEN** shares Sturm-count bisections to isolate roots on meshes one
  and two. Later meshes start from raw earlier-mesh roots, with local brackets
  certified by current-mesh Sturm counts. Both paths refine signed fluid
  dispersion using safeguarded Illinois steps, refilling up to 16 shooting lanes
  as modes finish. Each mode keeps its own bracket, history and iteration count;
  short tails use eight or four lanes. One-thread meshes share one queue;
  multiple workers use fixed, ordered chunks of 16 modes, independent of worker
  count, keeping small profiles parallel too. Endpoint evaluations and final
  count checks are packed across modes too. Results are bit-for-bit unchanged.
  Invalid brackets or count checks fall back per mode to the original
  full-interval bisection. Each mesh pays the original bisection's fixed work
  charge. Elastic KRAKEN and
  KRAKENC retain their separate searches; elastic counts need not be monotonic.
  See [fluid root search](kraken-internals.md#fluid-root-search) for the invariant,
  tolerance and fallback. First-mesh mode shapes run in parallel too, also in root order. Real fluid
  modes share each pass in fours, each with its own pivots, stopping point and
  failure, as long as those batches' scratch stays within 64 MiB across all
  workers; otherwise, and for elastic media, a worker holds one mode at a time.
  Range-independent FIELD tabulates each mode's amplitude, receiver offset and
  range phase once, then computes rows of receivers in parallel with the same
  operations per modal term. Eight ranges share each modal loop, with one running
  total per range, which the compiler packs without changing any sum. The phase
  and amplitude tables hold about a million modal terms at most, in blocks of
  ranges and sources.
  Range-dependent FIELD reuses each profile's receiver-depth brackets.
- **KRAKEN frequency blocks and profiles** carry the previous mode count forward.
  Real KRAKEN's searches with finite elastic layers read it as their first root
  limit, so those sequences run in order. So do those with an elastic surface
  half-space, although that search replaces the count unread, and any sequence
  below eight threads, where one solve's own parallelism beats solving side by
  side. Otherwise the first item solves alone, with every thread, and then up to
  four items per thread together. Results are consumed in order, so diagnostics,
  cumulative limits and cancellation are decided as with one thread; a window can
  only do work that a failure then discards. Twice the largest result so far
  decides how many fit in 128 MiB. That limits speculation, not memory: a later
  result can be larger, and solver scratch is separate.

`tests/parallelism.rs` in both crates compares every fixture and example, and
BELLHOP resource-limit failures, across thread counts; KRAKEN's include eight, where
solves first run side by side. Independent cases may
also run concurrently, through Python's `run_many` or the CLI's `run-many`,
which publishes each case through the same code as `run`. Cancellation is a
thread-scoped token that the solvers check inside ray tracing, influence,
modal searches and FIELD. Without a token, behaviour is unchanged.

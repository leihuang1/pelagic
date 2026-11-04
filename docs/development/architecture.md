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
| `native-output` | Toolbox-format writers (`.ray`, `.arr`, `.shd`, `.mod`) and result-directory publication. No HDF5 |
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
  holds bytes, parses them with `bellhop::json::load_case_document`. Python's
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
- BELLHOP's `result::FieldGrid` yields pressure depth-major for native SHD and
  Python, without changing the range-major solver/HDF5 layout.
- KRAKEN writes each frequency block in both formats before dropping it. The
  solver state that carries across frequencies is kept.
- HTTP writes private HDF5 files and keeps its own protocol. It does not use the
  CLI's directory publication.

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

- **BELLHOP** traces a batch of rays in parallel. Each ray's influence is
  recorded as `Effects`: single-precision field contributions, arrival
  candidates or eigenray hits. The calling thread then applies each ray in
  launch order. Pressure sums, arrival merging, resource limits and error
  precedence are therefore those of one thread. With one thread, each ray is
  applied as it is evaluated, as before. Otherwise a batch ends at 4 rays per
  thread or 256 MiB of records, whichever comes first. A worker gives up a ray
  whose records reach 32 MiB; the calling thread then applies that ray, and
  every later one, as it evaluates it, so memory stays bounded on any grid. A
  ray beyond the ray-point limit is rejected before its influence is
  evaluated. Applied effect buffers are reused, because some allocators keep a
  freshly grown buffer per ray.
- **KRAKEN** bisects every mode over the whole interval on its own, so modes
  run in parallel. Up to eight modes share each pass over the mesh, which
  evaluates all of their Sturm counts with each count's own operations. The
  interior rows are written so that the compiler packs those operations into
  vector divides; packed IEEE operations round each lane as scalar ones do.
  First-mesh mode shapes run in parallel too, also in root order. Real fluid
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
- **KRAKEN frequency blocks and profiles** carry the previous mode count forward,
  but only real elastic searches read it: finite elastic layers and an elastic
  surface half-space. Those sequences run in order, and so does any sequence
  below eight threads, where one solve's own parallelism beats solving side by
  side. Otherwise the first item solves alone, with every thread, and then up to
  four items per thread together. Results are consumed in order, so diagnostics,
  cumulative limits and cancellation are decided as with one thread; a window can
  only do work that a failure then discards. Twice the largest result so far
  decides how many fit in 128 MiB. That limits speculation, not memory: a later
  result can be larger, and solver scratch is separate.

`tests/parallelism.rs` in both crates compares every fixture and example, and
BELLHOP resource-limit failures, across thread counts. Independent cases may
also run concurrently, through Python's `run_many` or the CLI's `run-many`,
which publishes each case through the same code as `run`. Cancellation is a
thread-scoped token that the solvers check inside ray tracing, influence,
modal searches and FIELD. Without a token, behaviour is unchanged.

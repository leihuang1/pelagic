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
| `bellhop`, `kraken` | Input parsing, validation into immutable Cases, solvers, results, diagnostics |
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
  input/          legacy and JSON parsing, auxiliary-file acquisition
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
  exclusive lock and scratch, and rollback on handled errors. The single-file
  publisher remains for Rust callers. The rules are in
  [native output](../guide/native-output.md#ownership-quota-and-failure-contract).
- BELLHOP's `result::FieldGrid` yields pressure depth-major for native SHD and
  Python, without changing the range-major solver/HDF5 layout.
- KRAKEN writes each frequency block in both formats before dropping it. The
  solver state that carries across frequencies is kept.
- HTTP writes private HDF5 files and keeps its own protocol. It does not use the
  CLI's directory publication.

## Concurrency and cancellation

The solvers are single-threaded and deterministic. Independent cases may run
concurrently, for example through Python's `run_many`. KRAKEN frequency blocks
inside one case always run in order. Cancellation is a thread-scoped token that
the solvers check inside ray tracing, influence, modal searches and FIELD.
Without a token, behaviour is unchanged.

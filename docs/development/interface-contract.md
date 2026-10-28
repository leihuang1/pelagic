# Interface contract

This page defines what Pelagic promises its users, and the steps for changing
any of it. User-facing descriptions of the same interfaces are in the
[guides](../README.md#use-pelagic). Where this page and a guide disagree, fix
whichever is wrong in the same PR.

## Public interfaces

Anything in this table is public. A change to one needs an entry in the
**Compatibility** section of `CHANGELOG.md` and of the release notes.

| Interface | Definition | Version | Where the version lives |
| --- | --- | --- | --- |
| BELLHOP case JSON | [bellhop/json.md](../bellhop/json.md) | 2 (`"pelagic"`) | `bellhop::json::SCHEMA_VERSION` |
| KRAKEN case JSON | [kraken/json.md](../kraken/json.md) | 2 (`"pelagic"`) | `kraken::json::SCHEMA_VERSION` |
| BELLHOP HDF5 | [guide/bellhop-hdf5.md](../guide/bellhop-hdf5.md) | 3 | `output::bellhop::SCHEMA_VERSION` |
| KRAKEN HDF5 | [guide/kraken-hdf5.md](../guide/kraken-hdf5.md) | 1 | `output::kraken::SCHEMA_VERSION` |
| Native files and result directory manifest | [guide/native-output.md](../guide/native-output.md) | manifest 1 | `native-output/src/directory.rs` |
| Diagnostics protocol | [below](#diagnostics) | 1 | `crates/cli/src/main.rs` (`diagnostics_version`) |
| `run-many` status file | [guide/cli.md](../guide/cli.md#status-file-and-resuming) | 1 (`status_version`) | `crates/cli/src/many.rs` (`STATUS_VERSION`) |
| CLI commands, flags, exit codes | [guide/cli.md](../guide/cli.md) | package version | `Cargo.toml` |
| Python API: `pelagic` and its submodules, except names starting with `_` | [guide/python.md](../guide/python.md) | package version | `Cargo.toml` |
| MATLAB API: `pelagic.*`, except `pelagic.internal.*` | [guide/matlab.md](../guide/matlab.md) | package version | `matlab/VERSION` |
| HTTP API | [guide/http.md](../guide/http.md), `/openapi.json` | `/v1/` paths | `crates/server` |

**Not public:**

- the Rust crate APIs (the crates are not published to crates.io);
- diagnostic message text, and `.prt` report contents;
- `pelagic._core`, and `pelagic.internal.*` in MATLAB;
- the order of keys in JSON output.

The pinned numerical reference (Acoustics Toolbox v2023.5, commit `4751085`) is
reported by `pelagic version --json`, `pelagic.compatibility_reference` and
every result's provenance. Changing it is a major decision: it requires a new
acceptance baseline, and is out of scope for ordinary releases.

## Version rules

**Package version** (`X.Y.Z`, one number for the CLI, Python, MATLAB and the
server):

- Before 1.0, a minor release (`0.Y.0`) may change public interfaces, and lists
  every such change under **Compatibility**.
- A patch release (`0.Y.Z`) only fixes bugs, and changes no public interface.

**Schema versions** (case JSON, HDF5, manifest, diagnostics protocol):

- An existing version is never reinterpreted. A document that was valid under
  version N keeps its meaning under version N.
- A new **optional** field whose default keeps the old meaning does not need a
  new version. Example: `interface_roughness_m` in v0.3.0 defaults to smooth.
- Anything else needs a new version: removing or renaming a field, changing
  its meaning or unit, or making a field required. Readers keep accepting the
  old version until a release notes the removal.
- When a version changes, update all of: the constant in the table above, the
  schema doc, `matlab/+pelagic/version.m` (which checks every version it
  supports), the Python and MATLAB tests, and `CHANGELOG.md`.

## Diagnostics

A diagnostic is `{code, severity, field, message, source}`:

| Member | Stability |
| --- | --- |
| `code` | Stable. A code never changes meaning and is never reused |
| `severity` | `error` or `warning` |
| `field` | Stable for JSON input: the JSON path of the value, such as `frequencies[0].propagation`. Otherwise a named concept such as `input`, `output` or `execution` |
| `message` | Free text; may improve in any release |
| `source` | `{path, line, column}` or `null` |

The CLI's `--diagnostics json` writes one `{"diagnostics":[...]}` object per
line on stderr. Python exposes the same records on exceptions and warnings, and
MATLAB through `pelagic.diagnostics(err)`.

### Code allocation

`PREFIX` + four digits. Prefix `BH` is BELLHOP, `KR` is KRAKEN/KRAKENC and `PG`
is an adapter or the CLI front end. The first two digits name the stage:

| Digits | Stage |
| --- | --- |
| `00` | Reading files, arguments |
| `01` | Parsing: legacy records, JSON syntax, input size |
| `02` | Validation: values, consistency, unsupported combinations, limits |
| `03` | Solving; `0304` is reserved for cooperative cancellation |
| `04` | Output publication |
| `10` | Warnings |

To add a code:

1. Pick the prefix and stage. Reuse an existing code **only** if its meaning
   is exactly the same. For example, `BH0201` means "invalid case value" and
   covers many fields.
2. If you need a new one, check that it is unused:
   `git grep -n '"BH0205"'`.
3. Add a test that asserts the code, not the message.
4. Users act on adapter (`PG`) codes and warnings, so add those to the
   [troubleshooting table](../guide/troubleshooting.md#error-codes).

Current adapter codes: `PG0001` adapter I/O or process start, `PG0002` invalid
arguments, `PG0103` JSON transport, `PG0201` unknown model, `PG0202` CLI
version or protocol mismatch, `PG0402` Python native output, `PG0403` scratch
cleanup warning, `PG0404` `run-many` status file.

### Exit codes

`0` success, `2` input or arguments, `3` numerical, `4` output. These do not
change.

## Adapter architecture

```text
             JSON v2 (one per model)          validation + solve in Rust
Python  ── in-process (PyO3, pelagic._core) ─────────┐
MATLAB  ── pelagic CLI: JSON on stdin, NDJSON stderr, │──> bellhop / kraken crates
           HDF5 result read back ─────────────────────┘
other   ── same CLI protocol
```

Rules that every adapter follows:

- **Rust is the only validator.** Adapters move JSON in and arrays out. They do
  not check acoustic values, add defaults or reimplement parsing.
- **One contract.** A case that is valid in one adapter is valid in all of them,
  with the same meaning. There are no adapter-specific case fields.
- **Results are snapshots.** Python arrays are read-only and owned by the
  result. Native files are written from that snapshot, never by solving again.
- **Provenance travels with results:** version, reference, `case_json`,
  `case_sha256` and `input_sha256`.
- **Exact version match** between the MATLAB toolbox and its CLI
  (`pelagic.version()`). Adapters never download or update software.
- **Threads change speed, never results.** Every adapter can set the solver
  thread count (CLI `--threads`/`PELAGIC_THREADS`, Python `threads=`, MATLAB
  `'Threads'`, server `BELLHOP_SOLVER_THREADS`). It is not a case field and is
  not recorded in provenance, because no product depends on it.

### Python (`crates/python`, `python/pelagic`)

- Solves release the GIL. Cancellation uses a thread-local token checked inside
  the solvers (`BH0304`/`KR0304`), and Ctrl-C maps to it.
- `run_many` uses a thread pool with at most `workers` undelivered results.
  Unless `workers` is given, there are `available CPUs // threads` workers
  (`threads` counting as 1 when not given), but no more than the number of
  cases when the input has a length, and at least one. Unless `threads` is
  given, each solve gets `available CPUs // workers` threads (at least one);
  `case.run()` gets all of them.
- `run_many` raises a case's error and cancels the rest by default. With
  `return_exceptions=True`, a `PelagicError` other than `CancelledError` is
  yielded in that case's place, with `case_index` set, and the sweep goes on.
- Arrays are converted once to little-endian bytes and exposed through
  `np.frombuffer`. A field briefly needs about 2× its size in memory.
- Wheels are abi3 (`cp39-abi3`). No HDF5 is linked: `native-output` is the
  HDF5-free writer crate.

### MATLAB/Octave (`matlab/`)

- Starts the CLI with Java `ProcessBuilder`, never a shell string.
- Looks for the CLI in `PELAGIC_CLI`, then `bin/<platform>/` inside the
  toolbox, then `PATH` (`+internal/cli.m`).
- `+internal/encode.m` and `decode.m` are the JSON codec, shared by MATLAB and
  Octave. It needs no schema knowledge: every case-JSON list also accepts a
  single number, and every table row has at least two values, so a cell of plain
  numbers is always a flat list. **Keep both properties when adding a field.**

### Changing the case JSON

1. Change the Rust model and validation in `crates/{bellhop,kraken}/src`.
2. Update the schema doc (`docs/{bellhop,kraken}/json.md`).
3. Lists take a number, an array or a grid (the `Grid` type); tables are rows
   of at least two numbers. Defaults follow the `.env`, and `export` omits them.
4. Add or update fixtures. Python and MATLAB run every curated fixture through
   their transports (`python/tests/test_curated.py`,
   `matlab/tests/test_curated.m`).
5. Run `python3 tools/export-examples.py` to regenerate the example JSON.
6. Follow the [version rules](#version-rules), and add a `CHANGELOG.md` entry.

### Writing a new adapter

Use the CLI protocol rather than linking the Rust crates:

- call `pelagic version --json` once, and refuse unknown schema or protocol
  versions;
- send `run - --format hdf5 --output <private dir>` with the case JSON on stdin;
- always pass `--diagnostics json`, and treat any stderr line that is not a
  diagnostics object as an adapter error;
- read the HDF5 result using its schema doc;
- map exit codes 2/3/4 to the language's error types.

`matlab/+pelagic/+internal/invoke.m` and `run.m` implement exactly this, in
about 70 lines.

## Adapter verification

These checks run in `.github/workflows/bindings.yml`. They verify the adapters,
not the numerics. Numerical correctness comes from the
[reference comparisons](reference.md).

- **Python:** every curated fixture is run through Python and through the CLI
  (HDF5 and native). Fields, modes, rays, arrivals and native bytes must match
  exactly on the same platform, and rejections must give the same codes. On
  Windows the comparison uses a CLI with the same dynamic C runtime as the
  wheel. The standalone static-CRT CLI can differ in the last bits.
- **Python compatibility:** Python 3.9 with NumPy 1.26, and the newest Python
  with NumPy 2.x.
- **MATLAB (Linux, Windows) and Octave:** `matlab/tests/test_bindings.m`
  (smoke, geometry, codec, failures), and `test_curated.m`, which round-trips
  every curated fixture through the MATLAB codec.
- **Notebook:** `examples/notebooks/bindings-gallery.ipynb` is executed.
- **Examples:** every case in `examples/cases` must match its Toolbox files and
  run, and every Python, MATLAB and command-line example script must run
  (`test_examples.py`, `test_examples.m`). Examples use only public interfaces,
  so they also catch accidental API changes.
- **arlpy bridge:** tested separately against arlpy 1.9.

Locally: see [testing](testing.md#language-adapter-checks).

## Licence

Everything is GPL-3.0-or-later, including the toolbox and wheels. Bundled
dependency notices are maintained as described in
[packaging](../../packaging/README.md#dependency-notices). Do not relicense any
part to satisfy a distribution channel's upload form.

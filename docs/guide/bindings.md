# Python adapter and machine protocol

These adapters expose the existing supported numerical subset, not new physics.
Python runs Rust in-process and does not duplicate
acoustic validation. BELLHOP and KRAKEN retain **separate** JSON v1 contracts,
shared unchanged across languages; input schema versions are not HDF5 versions.

## Python

Build a checkout with `python -m pip install .`, or use `maturin develop --release`
in a virtual environment. Release wheels target CPython 3.9+ (GIL builds), Linux
x86-64/aarch64 (glibc 2.28+), macOS x86-64/ARM64 and Windows x64. Python 3.9 support
requires the compatible older NumPy/pandas/xarray releases selected by pip.
Free-threaded CPython and PyPy are not covered by the abi3 wheel contract.
Publishing a workflow is not evidence that every wheel has passed its runner.

The base dependency is NumPy. There is **no CLI or HDF5 dependency**, even for
native output. Optional extras are `pandas`, `xarray`, `examples`, and `arlpy`.
Use h5py separately for your own HDF5 workflows; Python does not reimplement the
versioned Rust HDF5 writer. There is no native-output `[io]` extra: the small
shared native writer ships in the base wheel.

```python
import pelagic as pg

case = pg.bellhop.load("examples/field-g.json")  # also ENV + consumed resources
specification = case.to_dict()                 # independent, JSON-compatible dict
specification["frequency_hz"] = 200.0
case = pg.bellhop.Case.from_dict(specification) # Rust validates; Case is immutable
result = case.run()                            # releases the GIL during solving
field = result.field
pressure = field.pressure                     # read-only complex64 ndarray
processed = pressure.copy()                   # independent writable postprocessing
loss_db = field.transmission_loss()            # -20 log10(abs(p)); zero -> +inf
result.write_native("results/python-run")      # serialize THIS result, never re-solve
```

`from_dict` also accepts NumPy arrays/scalars as JSON transport values. Unsupported
Python objects produce a transport diagnostic; Rust parses and validates all
acoustic values. Do not pass a second independently specified physics model.
Legacy loading normalizes through the existing JSON exporter before execution.
Unsupported legacy export combinations and existing input-size limits still apply.

### Results and units

All original arrays are backed by immutable storage, including through NumPy
views; changing `WRITEABLE` back to true is rejected. Native output always saves
the paired validated Case and original numerical result. Pandas/xarray conversions
are independent of that snapshot. Conversion may allocate: this is **not** a
zero-copy guarantee. A result retains Rust numerical data plus exported arrays
so it can be written later without rerunning the solver.

| Product | Shape / layout |
|---|---|
| Rectilinear field | complex64 `[source, receiver_depth, receiver_range]` |
| Irregular BELLHOP field | complex64 `[source, receiver]`, paired range/depth coordinates |
| KRAKEN wavenumbers | complex128 `[mode]`, radians/metre |
| KRAKEN eigenfunctions | complex128 `[sample_depth, mode]`, reference normalized |
| Ray points | float64 `[point, 6]`: range m, depth m, travel time s, attenuation time s, amplitude, phase radians |
| Arrivals | float32 numeric columns, uint32 bounces, uint64 offsets; `receiver_offset` groups receivers by source and `arrival_offset` groups arrivals by receiver |

Coordinates are float64 metres, depth positive down. No singleton axes are removed;
no irregular samples are interpolated, padded or expanded into a Cartesian product.
Pressure is relative, not calibrated pascals. Existing numerical rounding and
BELLHOP irregular-beam limitations remain unchanged. KRAKEN's float64 shape storage
does not restore precision already rounded in the reference-compatible solver.

`result.rays` / `result.eigenrays` are tuples of records with `points`, launch angle,
bounce counts, source association and termination. Eigenrays additionally identify
the receiver. No-field BELLHOP runs return `field=None`, not invented empty grids.
`result.arrivals.to_pandas()` creates a long table; empty receivers are retained in
the original offsets even when they naturally contribute no DataFrame rows.
`field.to_xarray()` labels axes; irregular fields have a `receiver` dimension with
range/depth coordinates. BELLHOP `result.to_xarray()` is a pressure-field shortcut.

### KRAKEN/KRAKENC

```python
case = pg.kraken.load("pekeris.env", solver="krakenc") # defaults to same-stem .flp
result = case.run()
frequency = result.frequencies[0]
modes = frequency.profiles[0].modes
field = frequency.field
```

This is **modes + FIELD**, not a modes-only API. `flp=...` selects legacy FIELD
geometry; it is invalid for JSON. A supplied `solver` must match a JSON document,
never override it. Single-frequency and single-profile results retain the same
nested layout. Frequencies keep order and duplicates; each profile has independent
mode count/sample depths. `to_xarray()` returns a tuple per frequency containing a
field and separate profile datasets rather than padding different mode sets.

Ordered frequency blocks use `solve_frequencies`, retaining reference search state.
Do not split one document into independent solves and assume equivalence. JSON
losses are effective **solve-frequency dB/wavelength**; changing `frequency_hz`
alone does not recompute legacy attenuation/volume-loss recipes. See
[KRAKEN JSON](../kraken/json.md).

### Sweeps, cancellation and memory

```python
with pg.bellhop.run_many(cases, workers=8) as results:
    for result in results:
        consume(result)
```

The iterator delivers in input order and retains at most `workers` running or
completed-but-undelivered tasks. Only independent documents run concurrently.
Any task failure stops submission and cooperatively cancels/joins other work;
the exception has `case_index`. Previously delivered results remain valid.
Ctrl-C uses the same cancellation path, including Jupyter kernel interrupts that
produce Python `KeyboardInterrupt`. Use a `with` block or `.close()` when breaking
early; merely abandoning a referenced iterator cannot guarantee prompt cleanup.

Cancellation checks occur within ray tracing/influence, modal searches and FIELD
loops. Cancellation latency depends on the bounded work between checks; it is not
hard real time. Force-killing the interpreter is not graceful cancellation.
`list(run_many(...))` intentionally retains all results; `workers` is not a global
byte quota. Input/solver limits remain in Rust. Native file publication defaults
to a 256 MiB group quota, adjustable via `max_output_bytes`; this is a file quota,
not a memory limit. Allocation failure is not guaranteed to be recoverable.

### Safe native output

`write_native(directory)` emits `case.ray`, `case.arr`, `case.shd`, or `case.mod` as
appropriate, plus canonical `case.json`, a Pelagic `.prt` report and ownership
manifest. Multiple frequencies use `.f0000`, `.f0001`, ...; irregular arrivals
retain the existing per-paired-receiver file convention. It reuses CLI ownership,
input/alias protection and handled-error rollback. It is not a crash journal.
Modified owned files and unrelated conflicts are never silently overwritten.
The names are not a drop-in replacement for every historical `bellhop.exe` script.

### Provenance and diagnostics

`pg.__version__`, `pg.compatibility_reference`, `pg.reference_commit` identify the
implementation and pinned oracle. Every result has `provenance`, including:

- exact submitted JSON hash (`input_sha256`); for legacy loading this is the
  portable exporter document, **not** a claim to hash every original legacy file;
- canonical exported document and its separate `case_sha256`;
- implementation version and reference identity.

`ValidationError`, `SimulationError`, `CancelledError`, `OutputError` carry an
ordered `diagnostics` tuple. Each item has stable `code`, `severity`, `field`,
`message` and optional `source` (filename, line, column). Convenience `code`,
`severity`, `field`/`path` refer to the first diagnostic; `path` here is a field
path, not a filename. Warnings use `PelagicWarning` and remain on Cases/results;
a warning does not normally fail a run. Users may elect Python's warning-as-error
policy. Diagnostic text is not a machine-readable API.

## Machine CLI protocol

- `pelagic {bellhop|kraken} {validate|export|run} -` reads JSON from stdin, never
  an ENV fragment. Runs default to the `stdin` result directory and `stdin.*` names.
- `--diagnostics json` is global. **stderr is NDJSON**: each record is
  `{"diagnostics":[...]}`. stdout remains the command's normal output, so `export`
  can still emit JSON without mixing warnings into it. Empty stderr means no
  diagnostics. Exit codes remain 0/2/3/4. Argument failures use `PG0002`.
- `pelagic version --json` reports implementation, case/HDF5 schema versions,
  diagnostics protocol version and reference identity.

JSON case schema versions remain 1 for each model. Incompatible input shape or
meaning requires a new schema version; existing versions are not silently
reinterpreted. Diagnostic codes must not be recycled for different meanings;
messages can improve. Fields identify JSON inputs when applicable, or named
execution/output/legacy-only concepts otherwise. `BH0304`/`KR0304` mean cooperative
cancellation. `PG0103` is binding JSON transport failure, `PG0201` invalid adapter
model, `PG0202` protocol/version mismatch, `PG0001` adapter I/O, and `PG0402` native
publication failure. `PG0403` warns about retained scratch after cleanup failure;
these warnings do not replace the primary error. Malformed tagged KRAKEN boundary
payloads identify the containing boundary object, independent of JSON member order. The error-report envelope is diagnostics protocol v1.

## arlpy bridge

```python
import arlpy.uwapm as pm
from pelagic.arlpy import register
register()
env = pm.create_env2d(frequency=500, nbeams=101)
pressure = pm.compute_transmission_loss(env, model="pelagic")
```

This optional bridge targets arlpy 1.9 and inherits upstream Python/NumPy limits
(the bridge test uses Python 3.11 with NumPy 1.26). It uses arlpy's existing legacy input
serializer and native readers in a private temporary directory, but **never runs
a Bellhop/Pelagic executable**. The solve and native write use this package.
Registration is opt-in, idempotent and does not change default model preference.
`fname_base` is explicitly unsupported: use the core API for persistent outputs.
The private upstream registry is a compatibility risk, tested separately; this is
not a claim that upstream has accepted or ships the backend.

## Verification and release gates

The Python tests compare committed examples and every input in the curated fixture
trees through both Python and CLI HDF5/native output, including matching rejection
paths, every pressure/mode/shape, rays/arrivals and native bytes. These are adapter
checks, not replacement Fortran goldens. Same-platform matching-build paths are
expected to agree exactly; cross-platform/reference comparisons retain the
[existing numerical criteria](../development/reference.md). On Windows the exact
adapter comparison uses a CLI with the same dynamic C runtime as the wheel. The
separately packaged standalone CLI retains a static C runtime; its math routines
can differ in the last bits, so cross-runtime byte identity is not promised.
The [notebook gallery](../../examples/notebooks/bindings-gallery.ipynb) executes in
CI. Full
numerical release acceptance remains required on the release commit.

## License and citation

GPL-3.0-or-later remains unchanged. Private use, including running your own scripts,
is permitted; redistribution of modified or combined software may carry GPL
obligations. Importing the module is not a blanket exception to those obligations.
See LICENSE and the bundled dependency notices; obtain legal advice for a specific
redistribution arrangement. Cite the exact release using [CITATION.cff](../../CITATION.cff), and retain input
and result provenance. No release DOI is fabricated by the bindings.

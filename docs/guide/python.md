# Python guide

The `pelagic` package runs BELLHOP and KRAKEN/KRAKENC inside Python and returns
NumPy arrays. It does not need the command-line program, HDF5 or a Fortran build.

```sh
python -m pip install pelagic
```

Wheels are available for CPython 3.9 and newer on Windows x64, macOS (Apple
silicon and Intel) and Linux x86-64/ARM64 (glibc 2.28+). NumPy is the only
required dependency. Optional extras:

| Extra | Adds |
| --- | --- |
| `pelagic[pandas]` | `Arrivals.to_pandas()` |
| `pelagic[xarray]` | `to_xarray()` on fields and modes |
| `pelagic[examples]` | matplotlib and a Jupyter kernel for the [notebook gallery](../../examples/notebooks/bindings-gallery.ipynb) |
| `pelagic[arlpy]` | The [arlpy backend](#use-pelagic-from-arlpy) |

## Load, change and run a case

```python
import pelagic as pg

case = pg.bellhop.load("field-g.json")   # or a legacy .env file
result = case.run()
```

A `Case` is immutable and already validated. To change it, edit its dictionary
form and build a new case:

```python
spec = case.to_dict()                    # a plain JSON-compatible dict
spec["frequency_hz"] = 200.0
case = pg.bellhop.Case.from_dict(spec)   # validates again
```

The dictionary follows the JSON format exactly. See [BELLHOP JSON](../bellhop/json.md)
or [KRAKEN JSON](../kraken/json.md) for every field. NumPy arrays and scalars
are accepted as values.

`load` accepts:

- **JSON**: a self-contained `.json` case.
- **BELLHOP legacy:** a `.env` file. Auxiliary files with the same stem
  (`.ssp`, `.bty`, `.ati`, `.brc`, `.trc`, `.irc`, `.sbp`) are read automatically.
- **KRAKEN legacy:** a `.env` file plus a `.flp` FIELD file.
  `pg.kraken.load("case.env", flp="geometry.flp", solver="krakenc")`. `flp`
  defaults to the same-stem file, and `solver` defaults to `"kraken"`.

## BELLHOP results

`case.run()` returns a `BellhopResult`. Which parts are filled depends on the
case's run type:

| Attribute | Contents |
| --- | --- |
| `field` | Pressure field, or `None` for ray/arrival runs |
| `rays`, `eigenrays` | Tuples of `Ray` records |
| `arrivals` | An `Arrivals` table, or `None` |
| `frequency_hz`, `title`, `warnings`, `provenance` | Run metadata |

**Fields.** `field.pressure` is complex64 with axes
`[source, receiver_depth, receiver_range]`. Coordinates are `field.source_depths_m`,
`field.depths_m` and `field.ranges_m`, in metres, depth positive down. For an
irregular (paired) receiver grid, `field.geometry == "irregular"` and pressure
is `[source, receiver]`. Pressure is relative, not calibrated pascals.

```python
loss_db = result.field.transmission_loss()     # -20 log10 |p|; zero gives +inf
da = result.to_xarray()                        # labelled DataArray (needs xarray)
```

**Rays.** Each `Ray` has `points`, a float64 array `[point, 6]` whose columns
are range (m), depth (m), travel time (s), attenuation time (s), amplitude and
phase (radians). It also has `launch_angle_degrees`, `top_bounces`,
`bottom_bounces`, `source_index` and `termination`. Eigenrays also carry
`receiver_index`, `range_m` and `depth_m`.

**Arrivals.** Columns such as `amplitude`, `phase_radians`, `travel_time_s` and
`source_angle_degrees` are flat arrays. `receiver_offset` and `arrival_offset`
group them by source and receiver. The easiest way to use them is a table:

```python
table = result.arrivals.to_pandas()   # one row per arrival (needs pandas)
```

## KRAKEN/KRAKENC results

KRAKEN always computes modes and the FIELD pressure together.

```python
result = pg.kraken.load("kraken-pekeris.json").run()
block = result.frequencies[0]           # one entry per input frequency, in order
modes = block.profiles[0].modes         # one entry per range profile
modes.wavenumbers                       # complex128 [mode], rad/m
modes.eigenfunctions                    # complex128 [sample_depth, mode]
modes.phase_speed_mps, modes.group_speed_mps, modes.attenuation_nepers_per_m
block.field.pressure                    # complex64 [source, depth, range]
```

The nesting is the same even for one frequency and one profile, so code
written for simple cases also works for complex ones. Each profile keeps its own
mode count and sample depths. `result.to_xarray()` returns one
`{"field": ..., "profiles": (...)}` dictionary per frequency rather than padding
different mode sets together.

Multiple frequencies in one document are solved in order and share search
state, as in the Toolbox. Splitting them into separate runs can give slightly
different modes.

## Results are read-only

Every array in a result is read-only, so a result always matches what was
computed. Use `.copy()` when you need to modify the data:

```python
p = result.field.pressure.copy()
p[:, :, 0] = 0
```

## Save native files

```python
result.write_native("results/run1")
```

This writes the Toolbox-format files for the result (`case.shd`, `case.ray`,
`case.arr` or `case.mod`), plus `case.json` (the exact case), a `case.prt`
report and a `pelagic-manifest.json`. It saves the result you already have; it
does not solve again. KRAKEN cases with several frequencies use
`case.f0000.mod`, `case.f0001.mod`, and so on.

Writing again to the same directory updates the files Pelagic wrote earlier.
Other files are left alone, and a file you have edited is never overwritten:
you get an `OutputError` instead. The total size is capped at 256 MiB by
default; raise it with `max_output_bytes=`. See
[native files and result directories](native-output.md) for the formats and
the exact rules.

Python does not write HDF5. Use the [command line](cli.md) with `--format hdf5`,
or write your own file with h5py.

## Parameter sweeps in parallel

`run_many` runs independent cases on worker threads and yields results in input
order:

```python
base = pg.bellhop.load("field-g.json").to_dict()

def at(frequency):
    spec = dict(base, frequency_hz=frequency)
    return pg.bellhop.Case.from_dict(spec)

cases = [at(f) for f in (50, 100, 200, 400)]
with pg.bellhop.run_many(cases, workers=4) as results:
    for case, result in zip(cases, results):
        loss = result.field.transmission_loss()[0, :, -1]   # last range
        print(case.to_dict()["frequency_hz"], loss.round(1))
```

`pg.kraken.run_many` works the same way.

- At most `workers` results are held in memory at once, unless you keep them,
  for example with `list(...)`.
- If one case fails, the remaining work is cancelled and the exception's
  `case_index` tells you which case failed.
- Ctrl-C, or interrupting a Jupyter kernel, cancels running solves promptly.
- Use the `with` block, or call `.close()`, if you stop iterating early.

A single `case.run()` also releases the GIL, so you can run cases from your own
threads.

## Errors and warnings

| Exception | Raised when |
| --- | --- |
| `pg.ValidationError` | The input is malformed or unsupported |
| `pg.SimulationError` | The solver cannot produce a valid result |
| `pg.CancelledError` | A run was interrupted (a subclass of `SimulationError`) |
| `pg.OutputError` | Native files could not be written safely |

All inherit from `pg.PelagicError` and carry `diagnostics`, a tuple of records
with a stable `code`, `severity`, `field` (the JSON path of the problem),
`message` and an optional `source` file/line/column:

```python
try:
    pg.bellhop.Case.from_dict(spec)
except pg.ValidationError as error:
    for d in error.diagnostics:
        print(d.code, d.field, d.message)
```

Match on `code`, not on message text, which may change between releases.
Warnings are issued as `pg.PelagicWarning` and also kept in `case.warnings` and
`result.warnings`. To treat them as errors, use
`warnings.simplefilter("error", pg.PelagicWarning)`. The codes are listed in
[Troubleshooting](troubleshooting.md#error-codes).

## Reproducibility

Every result records how it was produced:

```python
p = result.provenance
p.version, p.compatibility_reference, p.reference_commit
p.case_json       # the exact validated case that ran
p.case_sha256     # hash of that case
p.input_sha256    # hash of the submitted JSON
```

For a legacy input, `input_sha256` hashes the converted JSON document, not the
original `.env` file. `pg.__version__` and `pg.case_schema_versions` describe
the installed package.

## Use Pelagic from arlpy

Pelagic can serve as a backend for [arlpy](https://github.com/org-arl/arlpy)
1.9's `uwapm` module, in place of the BELLHOP executable:

```python
import arlpy.uwapm as pm
from pelagic.arlpy import register

register()                                      # opt-in; safe to call twice
env = pm.create_env2d(frequency=500, nbeams=101)
loss = pm.compute_transmission_loss(env, model="pelagic")
```

The backend uses arlpy's own input writer and file readers, and Pelagic for
the solve. It never runs an external program. `fname_base` is not supported:
use the Pelagic API directly when you want to keep output files. The backend
relies on arlpy internals, so it is tested only against arlpy 1.9.

## Limits

- Python wheels are abi3 builds for standard CPython. Free-threaded CPython and
  PyPy are not supported.
- The supported physics is exactly that of the solvers. See the
  [BELLHOP](../bellhop/compatibility.md) and
  [KRAKEN/KRAKENC](../kraken/compatibility.md) capability pages.
- Results are converted from Rust into new arrays, so a large field briefly
  needs about twice its size in memory.

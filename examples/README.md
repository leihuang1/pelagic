# Examples

Ready-to-run cases and scripts covering every interface: Python, MATLAB/Octave,
the command line and the HTTP service. Each one runs in seconds, and CI runs all
of them, so they always match the current release.

```text
examples/
  cases/       24 input cases: Acoustics Toolbox files plus the same case as JSON
  python/      16 Python scripts
  matlab/      8 MATLAB/Octave scripts
  cli/         shell, PowerShell and HDF5-reading examples
  http/        a client for the HTTP service
  notebooks/   a Jupyter gallery with pinned Fortran comparisons
```

## Where to start

| You are… | Start with |
| --- | --- |
| New to Pelagic, using Python | [`quickstart.py`](python/quickstart.py), then [`edit_case.py`](python/edit_case.py) |
| New to Pelagic, using MATLAB | [`example_quickstart.m`](matlab/example_quickstart.m), then [`example_edit_case.m`](matlab/example_edit_case.m) |
| An Acoustics Toolbox user with `.env` files | [`run-all.sh`](cli/run-all.sh): the same commands work on your files |
| Building a case without Toolbox files | [`build_case.py`](python/build_case.py) |
| Studying ray paths and arrival structure | [`rays.py`](python/rays.py), [`eigenrays.py`](python/eigenrays.py), [`arrivals.py`](python/arrivals.py) |
| Studying normal modes | [`modes.py`](python/modes.py), [`seabeds.py`](python/seabeds.py), [`leaky_modes.py`](python/leaky_modes.py) |
| Working with range-dependent or broadband problems | [`range_dependent.py`](python/range_dependent.py), [`broadband_pulse.py`](python/broadband_pulse.py) |
| Running many cases | [`sweep.py`](python/sweep.py) |
| Integrating Pelagic into other software | [`pipeline.sh`](cli/pipeline.sh), [`read_hdf5.py`](cli/read_hdf5.py), [`client.py`](http/client.py) |
| An arlpy user | [`arlpy_backend.py`](python/arlpy_backend.py) |

## Running them

From a clone of the repository (`git clone https://github.com/leihuang1/pelagic.git`),
or from the **source code** archive of a release. Figures and files are written
to `results/` in the current folder.

```sh
python -m pip install "pelagic[examples,pandas,xarray]"
python examples/python/quickstart.py
```

```matlab
addpath('examples/matlab')      % with the Pelagic toolbox installed
example_quickstart
```

```sh
examples/cli/run-all.sh         # needs `pelagic` on PATH, or PELAGIC_CLI set
```

## Python scripts

| Script | Shows | Cases |
| --- | --- | --- |
| [`quickstart.py`](python/quickstart.py) | Load, run and plot transmission loss | `munk-tl` |
| [`rays.py`](python/rays.py) | Ray fans coloured by boundary interaction; plotting `.ati`/`.bty` boundaries | `munk-rays`, `curved-boundaries`, `gulf` |
| [`eigenrays.py`](python/eigenrays.py) | Rays that reach one receiver, with travel times | `munk-eigenrays` |
| [`arrivals.py`](python/arrivals.py) | Impulse response, a received pulse, CSV export | `munk-arrivals` |
| [`transmission_loss.py`](python/transmission_loss.py) | Six environments side by side | `munk-tl`, `seamount`, `bottom-transition`, `elastic-bottom`, `reflection-table`, `beam-pattern` |
| [`edit_case.py`](python/edit_case.py) | Change a case as a dictionary, save it as JSON | `gulf` |
| [`build_case.py`](python/build_case.py) | A case from scratch; an irregular receiver track | none |
| [`sweep.py`](python/sweep.py) | Parallel runs with `run_many`; stacking with xarray | `munk-tl` |
| [`modes.py`](python/modes.py) | Mode shapes, phase and group speed, modal TL | `munk-modes` |
| [`seabeds.py`](python/seabeds.py) | An interface wave, rough-surface loss, a layered seabed | `elastic-sediment`, `rough-surface`, `layered-seabed` |
| [`leaky_modes.py`](python/leaky_modes.py) | KRAKENC complex wavenumbers and mode attenuation | `leaky-modes` |
| [`range_dependent.py`](python/range_dependent.py) | Adiabatic versus coupled modes | `gulf`, `gulf-coupled` |
| [`broadband_pulse.py`](python/broadband_pulse.py) | A received pulse from 101 frequencies | `broadband` |
| [`save_results.py`](python/save_results.py) | Toolbox files, NumPy, CSV, netCDF, provenance | `munk-tl`, `munk-arrivals`, `pekeris` |
| [`errors.py`](python/errors.py) | Error codes, warnings, warnings as errors | `munk-tl`, `field-g`, `pekeris` |
| [`arlpy_backend.py`](python/arlpy_backend.py) | Pelagic behind arlpy's `uwapm` (needs `pelagic[arlpy]`) | arlpy's own |

## MATLAB/Octave scripts

| Script | Shows |
| --- | --- |
| [`example_quickstart.m`](matlab/example_quickstart.m) | Load, run and plot transmission loss |
| [`example_rays.m`](matlab/example_rays.m) | Ray traces from the flat ray arrays and their offsets |
| [`example_arrivals.m`](matlab/example_arrivals.m) | The arrival table and impulse response |
| [`example_modes.m`](matlab/example_modes.m) | Mode shapes, dispersion and modal TL |
| [`example_range_dependent.m`](matlab/example_range_dependent.m) | Adiabatic versus coupled modes, choosing the `.flp` |
| [`example_sweep.m`](matlab/example_sweep.m) | Changing a case struct in a loop |
| [`example_edit_case.m`](matlab/example_edit_case.m) | Turning a ray case into a TL case; saving the JSON |
| [`example_errors.m`](matlab/example_errors.m) | Error identifiers, `pelagic.diagnostics` and load warnings |

These are tested in MATLAB. In Octave they need graphics for the plots.

## Command line and HTTP

| File | Shows |
| --- | --- |
| [`cli/run-all.sh`](cli/run-all.sh) / [`cli/run-all.ps1`](cli/run-all.ps1) | Validate and run every case, from JSON and from Toolbox files |
| [`cli/pipeline.sh`](cli/pipeline.sh) | JSON on stdin, `--diagnostics json`, `version --json` |
| [`cli/read_hdf5.py`](cli/read_hdf5.py) | Reading the HDF5 results with h5py |
| [`http/client.py`](http/client.py) | Validate, `/v1/field`, `/v1/arrivals` and `/v1/run` from Python |

## Cases

Each folder in `cases/` holds one environment as **Acoustics Toolbox files**
(`.env`, plus `.flp` for KRAKEN and any `.ssp`/`.bty`/`.ati`/`.brc`/`.sbp`)
**and** as **self-contained JSON**. Both give identical results; use whichever
suits you. Most are taken from the Toolbox's own tests or Pelagic's
reference fixtures, with larger receiver grids for plotting.
[`cases/catalog.json`](cases/catalog.json) records where each one came from.

**BELLHOP**

| Case | What it models | Products |
| --- | --- | --- |
| `field-g` | A tiny 7-beam field, for quick checks (it warns `BH1003` on purpose) | field |
| `munk-rays` | Deep-water Munk sound channel | rays |
| `munk-eigenrays` | Rays connecting the source to a receiver 50 km away | eigenrays |
| `munk-arrivals` | Amplitudes and delays at that receiver | arrivals |
| `munk-tl` | Coherent transmission loss to 100 km, Gaussian beams | field |
| `gulf` | Gulf of Mexico: range-dependent sound speed from a `.ssp` file | rays |
| `seamount` | The Dickins seamount, from a `.bty` file | field |
| `curved-boundaries` | A curved surface (`.ati`) over a parabolic bottom (`.bty`) | rays |
| `bottom-transition` | Shallow water; the bottom changes from fast to slow sediment (long-format `.bty`) | field |
| `elastic-bottom` | Shallow water over an elastic half-space | field |
| `reflection-table` | Shallow water over a tabulated reflection coefficient (`.brc`) | field |
| `beam-pattern` | A directional source in free space (`.sbp`) | field |

**KRAKEN/KRAKENC**

| Case | What it models | Solver |
| --- | --- | --- |
| `pekeris` | A tiny Pekeris waveguide, for quick checks | KRAKEN |
| `munk-modes` | 102 modes of the Munk sound channel | KRAKEN |
| `surface-duct` | Modes trapped in a surface duct | KRAKEN |
| `layered-seabed` | Two lossy fluid sediment layers | KRAKEN |
| `elastic-sediment` | An elastic sediment layer; includes an interface wave | KRAKEN |
| `arctic-ice` | An elastic ice cover over deep water | KRAKEN |
| `rough-surface` | A 5 m RMS rough sea surface | KRAKEN |
| `leaky-modes` | Complex and leaky modes over a lossy bottom | KRAKENC |
| `reflection-table` | A tabulated bottom reflection coefficient (`.brc`) | KRAKENC |
| `broadband` | 101 frequencies, 50–150 Hz, at one receiver | KRAKEN |
| `gulf` | Gulf of Mexico, 8 range-dependent profiles, adiabatic (`gulf.flp`) | KRAKEN |
| `gulf-coupled` | The same environment with coupled modes (`gulf-coupled.flp`) | KRAKEN |

Run a case directly:

```sh
pelagic bellhop run examples/cases/bellhop/seamount/seamount.env --output results/seamount
pelagic kraken run examples/cases/kraken/leaky-modes/leaky-modes.env --solver krakenc --output results/leaky
pelagic kraken run examples/cases/kraken/gulf/gulf.json --format hdf5 --output results/gulf
```

## Notebook

[`notebooks/bindings-gallery.ipynb`](notebooks/bindings-gallery.ipynb) compares
Pelagic's results with the pinned Acoustics Toolbox output for a pressure field,
a ray, KRAKEN modes and FIELD, and a case with auxiliary files.

## Adding an example

1. **A case:** create `cases/<model>/<name>/` with the Toolbox files, all named
   `<name>.*`. Add an entry to `cases/catalog.json`, run
   `python3 tools/export-examples.py` to write the JSON, and add a row above.
2. **A script:** add it to `python/` or `matlab/` (`example_*.m`). It should
   find cases relative to its own file, write to `results/`, run in under a
   minute, and be listed above.
3. Run `python -m pytest python/tests/test_examples.py`. For MATLAB, also run
   `matlab -batch "addpath('matlab/tests'); test_examples"`.

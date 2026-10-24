# Changelog

User-visible changes in each release. Full notes, with verification details,
are on the [releases page](https://github.com/leihuang1/pelagic/releases).
Changes to public interfaces are listed under **Compatibility**. The rules are
in the [interface contract](docs/development/interface-contract.md).

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

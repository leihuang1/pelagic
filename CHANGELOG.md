# Changelog

User-visible changes in each release. Full notes, with verification details,
are on the [releases page](https://github.com/leihuang1/pelagic/releases).
Changes to public interfaces are listed under **Compatibility**. The rules are
in the [interface contract](docs/development/interface-contract.md).

## Unreleased

- Documentation reorganized: getting started, separate Python, MATLAB and CLI
  guides, troubleshooting with error codes, and maintainer guides for releases
  and interface stability.

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

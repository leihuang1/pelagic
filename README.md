# Pelagic

Pelagic runs two-dimensional underwater acoustics models from familiar input
files and writes results you can inspect, archive or pass to other tools. It is
a Rust command-line program, so common workflows do not require a local Fortran
Acoustics Toolbox build.

Pelagic includes two independent model families:

- **BELLHOP** for rays, eigenrays, arrivals and sound fields.
- **KRAKEN / KRAKENC** for normal modes and acoustic fields.

You can start from Acoustics Toolbox-style legacy inputs or from Pelagic's
self-contained JSON. One calculation can write native numerical files, HDF5, or
both.

## Install

### Windows x64

Download `pelagic-vX.Y.Z-windows-x64.zip` from
[Releases](https://github.com/leihuang1/pelagic/releases), extract it, and run
`pelagic.exe`. No separate HDF5 installation or MSVC runtime is needed.

Each ZIP is accompanied by a SHA-256 checksum file. The archive includes license
notices and build information identifying the exact source commit. It contains
the CLI; the optional HTTP server is built separately.

### macOS and Linux

Releases after v0.3.0 also include `pelagic-vX.Y.Z-<platform>.tar.gz` for
`macos-arm64`, `macos-x64`, `linux-x64` and `linux-arm64`. Download one, check it against its
`.sha256` file, extract it and run `./pelagic`. The macOS binaries are not
notarized: if Gatekeeper blocks a downloaded copy, run
`xattr -d com.apple.quarantine pelagic` once. Linux binaries are built on
Ubuntu 22.04 (x86-64) and 24.04 (ARM64).

### Python wheels and MATLAB toolbox

Releases after v0.3.0 publish Python wheels to PyPI (`python -m pip install pelagic`)
and attach the MATLAB/Octave toolbox `pelagic-vX.Y.Z.mltbx`, which bundles the
matching CLI. Until then, build both from a checkout. See [Python and MATLAB](#python-and-matlab).

### Build from source

Install Rust, a C/C++ build toolchain and **CMake**. Development and CI use the
exact Rust version in `rust-toolchain.toml` (currently **1.99.0**); the declared
MSRV remains **1.88**. HDF5 is built and linked statically.

```sh
git clone https://github.com/leihuang1/pelagic.git
cd pelagic
cargo install --locked --path crates/cli
```

For test and release-verification policy, see
[daily tests and pre-release acceptance](docs/development/testing.md).

## Run a model

The repository includes small [example inputs](examples/README.md). From the
repository root:

```sh
# Check a BELLHOP input before calculating.
pelagic bellhop validate examples/field-g.json

# Calculate a sound field and save native files alongside HDF5.
pelagic bellhop run examples/field-g.json --format both --output results/bellhop

# Solve a self-contained KRAKEN example.
pelagic kraken run examples/kraken-pekeris.json --format both --output results/pekeris
```

Using the Windows ZIP? Download the example JSON files and point the commands at
your local copies. Use `.\pelagic.exe` instead of `pelagic` if the executable
isn't on your `PATH`.

Legacy BELLHOP inputs use `.env` files. Legacy KRAKEN/KRAKENC runs use an `.env`
file plus FIELD geometry in `.flp`; `--solver krakenc` selects KRAKENC. You can
also export legacy inputs to portable JSON:

```sh
pelagic bellhop export path/to/case.env > case.json
pelagic kraken export path/to/case.env --solver krakenc > case.json
```

Use `pelagic --help` or a subcommand's `--help` for the full argument list.

## Choose outputs

| `--format` | Result |
| --- | --- |
| `legacy` *(default)* | Native numerical files for the selected model/run |
| `hdf5` | A structured HDF5 result |
| `both` | Native files and HDF5 from the same solve |

`--output` names a **result directory**, not a single file. If you omit it,
Pelagic uses the input filename's stem in the current directory.

Rerunning a model updates only Pelagic's verified, owned outputs. Unrelated files
are left in place; modified outputs and input files are protected from
replacement. See the [CLI guide](docs/guide/cli.md) and
[output formats and directory safety](docs/guide/native-output.md) for details.

## Model support and limits

Pelagic uses Acoustics Toolbox **v2023.5** as its fixed numerical reference.
Pinned reference results document the cases that have been checked; they do not
imply support for every Toolbox option or every option combination.

Start with the model-specific compatibility guide before preparing a new study:

- [BELLHOP capabilities and limitations](docs/bellhop/compatibility.md)
- [KRAKEN/KRAKENC capabilities and limitations](docs/kraken/compatibility.md)

For output schemas and examples, use the [documentation index](docs/README.md).
For how numerical comparisons are made and reproduced, see
[reference verification](docs/development/reference.md).

## Python and MATLAB

Python calls the Rust solvers in-process and returns immutable NumPy results;
native files can be written from those results without a CLI or HDF5 dependency.
Build the current checkout with `python -m pip install .`:

```python
import pelagic as pg
result = pg.bellhop.load("examples/field-g.json").run()
pressure = result.field.pressure  # [source, depth, range], complex64
result.write_native("results/python")
```

The MATLAB/Octave adapter under `matlab/` drives a version-matched CLI and reads
its HDF5 products. See the [bindings guide](docs/guide/bindings.md) for installation,
JSON struct encoding, array axes, diagnostics, parallel sweeps and release status.
The [executable gallery](examples/notebooks/bindings-gallery.ipynb) includes pinned
Fortran comparisons. Use [CITATION.cff](CITATION.cff) to cite a release.

## Optional HTTP API

`pelagic-server` is a separate **BELLHOP-only** HTTP service. It accepts
self-contained JSON cases and returns HDF5 or focused JSON responses.

```sh
cargo install --locked --path crates/server
pelagic-server
```

Open `http://localhost:8080/docs`, or submit a case directly:

```sh
curl --fail-with-body -H 'Content-Type: application/json' \
  --data-binary @examples/field-g.json http://localhost:8080/v1/run --output result.h5
```

Building the server also downloads Swagger UI assets. Before exposing it to a
network, review [authentication, configuration and limits](docs/guide/http.md).

## Documentation map

- [CLI guide](docs/guide/cli.md): validation, export, computation and exit codes
- [Native output guide](docs/guide/native-output.md): result directories,
  manifests and safe replacement rules
- [BELLHOP HDF5 schema v3](docs/guide/bellhop-hdf5.md)
- [KRAKEN/KRAKENC HDF5 schema v1](docs/guide/kraken-hdf5.md)
- [Examples](examples/README.md)
- [Development and verification docs](docs/README.md#develop-and-verify)

## License

GPL-3.0. See [LICENSE](LICENSE) and [NOTICE](NOTICE).

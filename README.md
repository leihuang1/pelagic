# Pelagic

Underwater acoustics, from model inputs to results—without a Fortran toolchain.

Pelagic brings two families of **2D acoustic models** to one Rust command-line tool:

- **BELLHOP** traces rays and computes eigenrays, arrivals and sound fields.
- **KRAKEN / KRAKENC** solve normal modes and compute acoustic fields.

Use familiar Acoustics Toolbox input files or self-contained JSON, then save
native numerical files, HDF5, or both from a single calculation.

## Get Pelagic

### Windows x64

Download `pelagic-vX.Y.Z-windows-x64.zip` from
[Releases](https://github.com/leihuang1/pelagic/releases), extract it, and run
`pelagic.exe`. No separate HDF5 installation or MSVC runtime is needed.

Each ZIP comes with a SHA-256 checksum file, license notices and build information
that identifies the exact source commit. The prebuilt package contains the CLI;
the optional HTTP server is built separately.

### Build from source

You'll need Rust, a C/C++ build toolchain and **CMake**.
Development and CI use the exact version in `rust-toolchain.toml` (currently
**1.99.0**); the declared MSRV remains **1.88**. HDF5 is built and linked statically.
See [daily tests and pre-release acceptance](docs/development/testing.md).

```sh
git clone https://github.com/leihuang1/pelagic.git
cd pelagic
cargo install --locked --path crates/cli
```

## Run your first model

The repository includes small [example inputs](examples/README.md) to get started.
From the repository root, try:

```sh
# Check a BELLHOP input before calculating.
pelagic bellhop validate examples/field-g.json

# Calculate a sound field and save native files alongside HDF5.
pelagic bellhop run examples/field-g.json --format both --output results/bellhop

# Solve a self-contained KRAKEN example.
pelagic kraken run examples/kraken-pekeris.json --format both --output results/pekeris
```

Using the Windows ZIP? Download the example JSON files and point the commands at
your local copies. Use `.\pelagic.exe` instead of `pelagic` if the executable isn't
on your `PATH`.

For legacy inputs, BELLHOP accepts `.env` files. KRAKEN/KRAKENC use an `.env`
file together with FIELD geometry in `.flp`; `--solver krakenc` selects KRAKENC.
You can also export legacy inputs to portable JSON:

```sh
pelagic bellhop export path/to/case.env > case.json
pelagic kraken export path/to/case.env --solver krakenc > case.json
```

Use `pelagic --help` or a subcommand's `--help` to explore the available options.

## Choose your output

| `--format` | What you get |
| --- | --- |
| `legacy` *(default)* | Native numerical files for the selected model/run |
| `hdf5` | A structured HDF5 result |
| `both` | Both formats, without calculating twice |

`--output` names a **result directory**, not a single file. If you leave it out,
Pelagic uses the input filename's stem in the current directory.

Rerunning a model updates only Pelagic's verified, owned outputs. Unrelated files
stay untouched; modified outputs and input files are protected from replacement.
See the [CLI guide](docs/guide/cli.md) and
[output formats and directory safety](docs/guide/native-output.md) for details.

## Need an HTTP API?

The separate `pelagic-server` provides a **BELLHOP-only** API, including HDF5 and
JSON responses, validation and interactive API documentation.

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

## Models, compatibility and documentation

Pelagic uses Acoustics Toolbox **v2023.5** as its fixed numerical reference.
Supported cases are checked against pinned reference results; not every Toolbox
option or combination is supported. Check the model-specific guides when choosing
inputs for a new problem:

- [BELLHOP capabilities and limitations](docs/bellhop/compatibility.md)
- [KRAKEN/KRAKENC capabilities and limitations](docs/kraken/compatibility.md)
- [Documentation index](docs/README.md)—input schemas, output layouts, examples
  and development guides
- [Reference verification](docs/development/reference.md)—how numerical comparisons
  and reproducibility checks work

## License

GPL-3.0. See [LICENSE](LICENSE) and [NOTICE](NOTICE).

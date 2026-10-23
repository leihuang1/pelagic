# Getting started

This page takes you from nothing installed to a first result. It takes about
five minutes in each interface.

## Choose an interface

All interfaces run the same solvers and accept the same JSON cases, so you
can switch later without changing results.

| Interface | Best for | How it runs |
| --- | --- | --- |
| [Python](guide/python.md) | Scripts, notebooks, parameter sweeps | Solvers run inside Python; results are NumPy arrays |
| [MATLAB/Octave](guide/matlab.md) | Existing MATLAB workflows | The toolbox calls a bundled `pelagic` program and returns structs |
| [Command line](guide/cli.md) | Batch jobs, shell scripts, other languages | Writes result files to a directory |
| [HTTP service](guide/http.md) | Running BELLHOP for other programs over a network | A separate server; BELLHOP only |

## Get an example case

The commands below use [`field-g.json`](../examples/field-g.json), a small
BELLHOP pressure-field case. [`kraken-pekeris.json`](../examples/kraken-pekeris.json)
is a small KRAKEN case. Download them, or clone the repository:

```sh
curl -LO https://raw.githubusercontent.com/leihuang1/pelagic/main/examples/field-g.json
curl -LO https://raw.githubusercontent.com/leihuang1/pelagic/main/examples/kraken-pekeris.json
```

You can also use your own Acoustics Toolbox files. See
[Using existing Toolbox files](#using-existing-toolbox-files).

## Python

```sh
python -m pip install pelagic
python -c "import pelagic; print(pelagic.__version__)"
```

```python
import pelagic as pg

result = pg.bellhop.load("field-g.json").run()
print(result.field.pressure.shape)          # (sources, depths, ranges)

modes = pg.kraken.load("kraken-pekeris.json").run()
print(modes.frequencies[0].profiles[0].modes.wavenumbers)
```

Optional extras add conversions and plotting:
`python -m pip install "pelagic[pandas,xarray,examples]"`. Continue with the
[Python guide](guide/python.md).

## MATLAB

You need MATLAB R2023a or newer, with Java enabled (the default).

1. Download `pelagic-vX.Y.Z.mltbx` from
   [Releases](https://github.com/leihuang1/pelagic/releases).
2. Double-click it, or run `matlab.addons.install('pelagic-vX.Y.Z.mltbx')`.
3. Check the installation:

```matlab
pelagic.version()        % reports the version and the pinned reference
c = pelagic.bellhop.load('field-g.json');
r = pelagic.bellhop.run(c);
size(r.pressure)
```

The toolbox includes the `pelagic` program for every supported platform, so
nothing else needs to be installed. Continue with the
[MATLAB/Octave guide](guide/matlab.md).

## GNU Octave

Octave cannot install `.mltbx` files, so set up the two parts by hand. You
need Octave 7 or newer with Java enabled: `usejava('jvm')` must return true.
If it doesn't, install a Java runtime, then restart Octave.

1. Download and extract the command-line archive for your platform (see
   [Command line](#command-line)).
2. Download the **source code** archive from the same release, and extract it.
   The two must be the same version.
3. In Octave:

```matlab
addpath('/path/to/pelagic-X.Y.Z/matlab');
setenv('PELAGIC_CLI', '/path/to/pelagic-vX.Y.Z-<platform>/pelagic');
pelagic.version()
```

Put those lines in your `~/.octaverc` to keep them across sessions.

## Command line

Download the archive for your platform from
[Releases](https://github.com/leihuang1/pelagic/releases):

| Platform | Archive |
| --- | --- |
| Windows x64 | `pelagic-vX.Y.Z-windows-x64.zip` |
| macOS, Apple silicon | `pelagic-vX.Y.Z-macos-arm64.tar.gz` |
| macOS, Intel | `pelagic-vX.Y.Z-macos-x64.tar.gz` |
| Linux x86-64 (glibc 2.35+) | `pelagic-vX.Y.Z-linux-x64.tar.gz` |
| Linux ARM64 (glibc 2.39+) | `pelagic-vX.Y.Z-linux-arm64.tar.gz` |

On macOS or Linux:

```sh
shasum -a 256 -c pelagic-vX.Y.Z-macos-arm64.tar.gz.sha256   # optional check
tar -xzf pelagic-vX.Y.Z-macos-arm64.tar.gz
cd pelagic-vX.Y.Z-macos-arm64
./pelagic version
./pelagic bellhop run ../field-g.json --output ../results/field-g
```

On Windows, extract the ZIP and run `.\pelagic.exe version` in PowerShell.
To run `pelagic` from any directory, move it onto your `PATH`.

You will see `warning[BH1003]: coherent run may use too few beams`. The
example deliberately uses only 7 beams to keep it small. Warnings never stop a
run. The output directory now contains `field-g.shd` (readable by the
Toolbox's MATLAB plotting scripts), a `field-g.prt` run report and a small
ownership manifest. Add `--format hdf5` or `--format both` for an HDF5 file. Continue
with the [command-line guide](guide/cli.md).

If macOS says the program "cannot be opened", see
[Troubleshooting](guide/troubleshooting.md#macos-blocks-the-program).

## Using existing Toolbox files

Pelagic reads the same 2D input files as Acoustics Toolbox v2023.5:

- **BELLHOP:** a `.env` file, plus any `.ssp`, `.bty`, `.ati`, `.brc`, `.trc`,
  `.irc` or `.sbp` files it uses. These must sit next to it with the same
  file stem.
- **KRAKEN/KRAKENC:** a `.env` file plus a `.flp` FIELD file. KRAKEN always
  computes the field as well as the modes.

```sh
pelagic bellhop run munk.env --output results/munk
pelagic kraken run pekeris.env --solver krakenc --output results/pekeris
```

```python
pg.bellhop.load("munk.env").run()
pg.kraken.load("pekeris.env", solver="krakenc").run()
```

To share a case, or to edit it as data, convert it to one self-contained JSON
file that already includes the auxiliary files:

```sh
pelagic bellhop export munk.env > munk.json
pelagic kraken export pekeris.env --solver krakenc > pekeris.json
```

The JSON formats are documented in [BELLHOP JSON](bellhop/json.md) and
[KRAKEN JSON](kraken/json.md).

## Next steps

- Check that your options are supported:
  [BELLHOP](bellhop/compatibility.md), [KRAKEN/KRAKENC](kraken/compatibility.md).
- Learn the output files: [native files and result directories](guide/native-output.md),
  [BELLHOP HDF5](guide/bellhop-hdf5.md), [KRAKEN HDF5](guide/kraken-hdf5.md).
- Explore the [notebook gallery](../examples/notebooks/bindings-gallery.ipynb).
- Something went wrong? See [Troubleshooting](guide/troubleshooting.md).

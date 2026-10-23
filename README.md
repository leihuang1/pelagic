# Pelagic

[![PyPI](https://img.shields.io/pypi/v/pelagic)](https://pypi.org/project/pelagic/)
[![CI](https://github.com/leihuang1/pelagic/actions/workflows/ci.yml/badge.svg)](https://github.com/leihuang1/pelagic/actions/workflows/ci.yml)
[![License: GPL-3.0](https://img.shields.io/badge/license-GPL--3.0-blue)](https://github.com/leihuang1/pelagic/blob/main/LICENSE)

Pelagic runs the classic two-dimensional underwater acoustics models from
Python, MATLAB/Octave or the command line, without building the Fortran
Acoustics Toolbox:

- **BELLHOP**: ray traces, eigenrays, arrivals and pressure fields.
- **KRAKEN / KRAKENC**: real and complex normal modes, plus FIELD pressure.

It reads your existing `.env`/`.flp` files or a self-contained JSON case. It
writes the familiar `.ray`/`.arr`/`.shd`/`.mod` files, structured HDF5, or both.
Results are checked against the official Acoustics Toolbox **v2023.5** release.

## Install

| You use | Install |
| --- | --- |
| Python 3.9+ | `python -m pip install pelagic` |
| MATLAB R2023a+ | Download `pelagic-vX.Y.Z.mltbx` from [Releases](https://github.com/leihuang1/pelagic/releases) and open it in MATLAB |
| GNU Octave 7+ | See [Getting started](https://github.com/leihuang1/pelagic/blob/main/docs/getting-started.md#gnu-octave) |
| The command line | Download `pelagic-vX.Y.Z-<platform>` from [Releases](https://github.com/leihuang1/pelagic/releases) |

Prebuilt packages cover Windows x64, macOS (Apple silicon and Intel) and
Linux (x86-64 and ARM64). Nothing else needs to be installed: HDF5 is built
in. To build from source, see [CONTRIBUTING](https://github.com/leihuang1/pelagic/blob/main/CONTRIBUTING.md).

## Quick start

These use [`munk-tl.json`](https://github.com/leihuang1/pelagic/blob/main/examples/cases/bellhop/munk-tl/munk-tl.json),
deep-water transmission loss. [`examples/`](https://github.com/leihuang1/pelagic/blob/main/examples/README.md)
has 24 cases, from rays and arrivals to elastic seabeds, range-dependent modes
and broadband pulses, each with Python, MATLAB and command-line scripts.

**Python**

```python
import pelagic as pg

result = pg.bellhop.load("munk-tl.json").run()
pressure = result.field.pressure            # complex64, [source, depth, range]
loss_db = result.field.transmission_loss()
result.write_native("results/munk-tl")      # .shd and friends, no re-solve
```

**MATLAB / Octave**

```matlab
c = pelagic.bellhop.load('munk-tl.json');
c.frequency_hz = 200;
r = pelagic.bellhop.run(c);
imagesc(r.ranges_m, r.depths_m, 20*log10(abs(squeeze(r.pressure(1,:,:)))));
```

**Command line**

```sh
pelagic bellhop run munk-tl.json --format both --output results/munk-tl
pelagic kraken run case.env --solver krakenc --output results/case
```

## What it can and cannot do

Pelagic covers a large, tested part of 2D BELLHOP and KRAKEN/KRAKENC, but not
every option combination in the Toolbox. Three-dimensional programs (BELLHOP3D,
FIELD3D) and BOUNCE table generation are not included. Unsupported input is
rejected with a clear error rather than approximated. Before starting a study,
check your options against:

- [BELLHOP capabilities and limits](https://github.com/leihuang1/pelagic/blob/main/docs/bellhop/compatibility.md)
- [KRAKEN/KRAKENC capabilities and limits](https://github.com/leihuang1/pelagic/blob/main/docs/kraken/compatibility.md)

## Documentation

- [Getting started](https://github.com/leihuang1/pelagic/blob/main/docs/getting-started.md):
  install, first run and choosing an interface
- Guides: [Python](https://github.com/leihuang1/pelagic/blob/main/docs/guide/python.md) ·
  [MATLAB/Octave](https://github.com/leihuang1/pelagic/blob/main/docs/guide/matlab.md) ·
  [Command line](https://github.com/leihuang1/pelagic/blob/main/docs/guide/cli.md) ·
  [HTTP service](https://github.com/leihuang1/pelagic/blob/main/docs/guide/http.md)
- [Examples](https://github.com/leihuang1/pelagic/blob/main/examples/README.md):
  24 cases with Python, MATLAB, command-line and HTTP scripts
- [Troubleshooting and error codes](https://github.com/leihuang1/pelagic/blob/main/docs/guide/troubleshooting.md)
- [Full documentation index](https://github.com/leihuang1/pelagic/blob/main/docs/README.md),
  including file formats and JSON schemas
- [Changelog](https://github.com/leihuang1/pelagic/blob/main/CHANGELOG.md)

Questions and bug reports are welcome in
[Issues](https://github.com/leihuang1/pelagic/issues). To contribute, start with
[CONTRIBUTING](https://github.com/leihuang1/pelagic/blob/main/CONTRIBUTING.md).

## Citing Pelagic

If Pelagic contributes to published work, cite the exact release you used. See
[CITATION.cff](https://github.com/leihuang1/pelagic/blob/main/CITATION.cff), or
use "Cite this repository" on GitHub. Each result also records its Pelagic
version and an input hash for reproducibility.

## License

GPL-3.0-or-later. See [LICENSE](https://github.com/leihuang1/pelagic/blob/main/LICENSE)
and [NOTICE](https://github.com/leihuang1/pelagic/blob/main/NOTICE).

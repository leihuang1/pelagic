# Command-line guide

The `pelagic` program validates, converts and runs cases, and writes the results
to a directory. See [Getting started](../getting-started.md#command-line) to
install it. `pelagic --help`, and `--help` on any subcommand, list every option.

```text
pelagic version                         show version and pinned reference
pelagic bellhop  validate|export|run    BELLHOP rays, arrivals, fields
pelagic kraken   validate|export|run    KRAKEN/KRAKENC modes and FIELD
```

## Validate, export and run

| Command | What it does |
| --- | --- |
| `validate CASE` | Checks the input completely without solving |
| `export CASE` | Prints the case as self-contained JSON on stdout |
| `run CASE` | Solves and writes results to a directory |

`CASE` is a `.json` file, a legacy `.env` file, or `-` for JSON on stdin.

### BELLHOP

```sh
pelagic bellhop validate munk.env
pelagic bellhop run munk.env --output results/munk
pelagic bellhop export munk.env > munk.json      # includes .ssp/.bty/... data
```

A legacy `.env` reads its auxiliary files (`.ssp`, `.bty`, `.ati`, `.brc`,
`.trc`, `.irc`, `.sbp`) from the same directory, with the same stem, when its
options call for them. A JSON case is always self-contained and never reads
neighbouring files.

### KRAKEN/KRAKENC

```sh
pelagic kraken run pekeris.env --output results/pekeris                 # KRAKEN
pelagic kraken run pekeris.env --solver krakenc --output results/pekc   # KRAKENC
pelagic kraken run pekeris.env --flp other.flp --output results/other
pelagic kraken export pekeris.env --solver krakenc > pekeris.json
```

Legacy KRAKEN input is a `.env` plus a `.flp` FIELD file. `--flp` defaults to
the same-stem file and `--solver` defaults to `kraken`. A JSON case names its
own solver, so `--flp` is not allowed, and `--solver`, if given, must match.

Frequencies run in input order, including repeats. If one frequency fails, the
run stops and nothing is published.

## Output

```sh
pelagic bellhop run case.json --output results/case --format both
```

| `--format` | Writes |
| --- | --- |
| `legacy` *(default)* | Toolbox-format files: `.shd`, `.ray`, `.arr` or `.mod` |
| `hdf5` | One structured `.h5` file ([BELLHOP schema](bellhop-hdf5.md), [KRAKEN schema](kraken-hdf5.md)) |
| `both` | Both, from a single solve |

`--output` is always a directory. It defaults to the input's stem in the
current directory, and is created if missing. Every run also writes a `.prt`
report and a `pelagic-manifest.json` listing the files Pelagic owns.

Running again into the same directory replaces Pelagic's own earlier files and
removes ones the new run no longer produces. Pelagic **does not** touch:

- files it did not write;
- its own files that you have since modified;
- the input files.

If any of these would be overwritten, the run fails with exit code 4 and the
previous results stay in place. Move the file, or choose another directory. There is no
`--overwrite` flag. The exact rules and file layouts are in
[native files and result directories](native-output.md).

KRAKEN limits the total output to 256 MiB by default; change this with
`--max-output-bytes`. KRAKEN cases with several frequencies write
`case.f0000.mod`, `case.f0001.mod`, and so on, one pair per frequency.

## Exit codes

| Code | Meaning |
| --- | --- |
| `0` | Success, possibly with warnings |
| `2` | Invalid input or arguments |
| `3` | The solver could not produce a valid result |
| `4` | Results could not be written safely |

Errors and warnings print as `file:line:column: error[CODE]: message (field)`.
The codes are listed in [Troubleshooting](troubleshooting.md#error-codes).

## Use from other programs

The program has a small machine-readable interface for scripts and for adapters
in other languages, such as R or Julia. The MATLAB toolbox uses the same
interface.

- **JSON on stdin.** Pass `-` as `CASE`: `pelagic bellhop run - < case.json`.
  Only JSON is accepted on stdin. A run reading stdin writes to `./stdin/` with
  files named `stdin.*` unless `--output` is given.
- **Structured diagnostics.** `--diagnostics json` (before or after the
  subcommand) makes every message on stderr one JSON line:
  `{"diagnostics":[{"code":…,"severity":…,"field":…,"message":…,"source":…}]}`.
  stdout is unchanged, so `export` still prints only the case. Empty stderr
  means no diagnostics.
- **Versions.** `pelagic version --json` prints the version, the case and HDF5
  schema versions, the diagnostics protocol version and the pinned reference.
  Check it before relying on a format.

```sh
pelagic --diagnostics json bellhop run - --format hdf5 --output out < case.json
```

Rely on `code`, `severity` and `field`; message wording can change. The
compatibility promises behind these formats are listed in the
[interface contract](../development/interface-contract.md).

For complete scripts, see [`run-all.sh`](../../examples/cli/run-all.sh)
(every example case, from JSON and from Toolbox files),
[`pipeline.sh`](../../examples/cli/pipeline.sh) (stdin, diagnostics, versions)
and [`read_hdf5.py`](../../examples/cli/read_hdf5.py).

## HTTP service

The separate `pelagic-server` program serves BELLHOP over HTTP. See the
[HTTP guide](http.md).

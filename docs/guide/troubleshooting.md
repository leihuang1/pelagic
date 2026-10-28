# Troubleshooting

## Installation

### macOS blocks the program

macOS may refuse to open a downloaded `pelagic` with a message that it "cannot
be opened" or "cannot be verified". The binaries are not notarized by Apple.
Remove the download quarantine flag once:

```sh
xattr -d com.apple.quarantine pelagic
```

### Linux: `GLIBC_2.xx not found`

The prebuilt Linux programs need glibc 2.35+ (x86-64) or 2.39+ (ARM64). On an
older distribution, use the Python wheel (glibc 2.28+), or build from source:
`cargo install --locked --path crates/cli` (see [CONTRIBUTING](../../CONTRIBUTING.md)).

### pip builds from source or finds no wheel

Wheels exist for CPython 3.9+ on Windows x64, macOS and glibc Linux. pip falls
back to the source archive, which needs Rust and CMake, on:

- PyPy and free-threaded CPython;
- musl-based Linux such as Alpine;
- 32-bit Python;
- very old versions of pip. Upgrade with `python -m pip install -U pip`.

### MATLAB/Octave: `pelagic:PG0202` version mismatch

The toolbox only works with a `pelagic` program of exactly the same version. The
message names both versions. Unset `PELAGIC_CLI` (`setenv('PELAGIC_CLI', '')`)
to use the bundled copy. If you set `PELAGIC_CLI` on purpose, point it at the
matching release. Also check that an older `pelagic` on your `PATH` isn't
being picked up.

### MATLAB/Octave: Java errors

The toolbox starts `pelagic` through Java. In Octave, `usejava('jvm')` must
return true; install a Java runtime if it doesn't. MATLAB started with
`-nojvm` cannot use the toolbox.

## Running cases

### A warning appears, but the run succeeds

Warnings report something worth checking without stopping the run. Common ones:

| Code | Meaning |
| --- | --- |
| `BH1001` | Unknown legacy beam family; geometric hat beams are used, as in BELLHOP |
| `BH1002` | A legacy vector was not sorted; it is sorted before use, as in BELLHOP |
| `BH1003` | A coherent field may have too few beams; the message suggests a count |
| `BH1004` | A reflection table's frequency differs from the case frequency |

### "Unsupported" or "not supported" errors

Pelagic rejects options it has not verified against the reference instead of
guessing. Check the [BELLHOP](../bellhop/compatibility.md) or
[KRAKEN/KRAKENC](../kraken/compatibility.md) capability page for the option
named in the error's field. Some options are unavailable because the reference
Toolbox itself never implemented them.

### KRAKEN: mode count changed during refinement (`KR0303`)

The mode count changed between meshes because a mode sits very close to a
phase-speed limit. Move the phase-speed limits (`CLOW`/`CHIGH` in the `.env`,
`c_low_mps`/`c_high_mps` in JSON) slightly away from that mode.

### Exit code 4: results could not be written

Pelagic will not overwrite files it doesn't own, files you have edited since a
previous run, or your input files. The error names the conflicting file. Move
it, or choose another `--output` directory.

If the error says a rollback failed, the directory still contains
`.pelagic-lock` and `.pelagic-backup`. These hold your previous results. Restore
the files listed in the error from `.pelagic-backup` before you delete anything.
The details are in [native files and result directories](native-output.md#ownership-quota-and-failure-contract).

### Results differ slightly from my own Fortran build

Pelagic is compared against one fixed build: Acoustics Toolbox v2023.5 compiled
with GNU Fortran 12.2 on Linux x86-64. Other compilers, flags or platforms change
the Fortran results in the last digits too, and near nulls of a field those
differences look larger in dB. Known, intentional differences are listed in
[BELLHOP deviations](../bellhop/deviations.md). If you see a large difference on
a supported option, please [report it](#reporting-a-problem).

## Error codes

Every diagnostic has a stable code. The prefix names its source, and the digits
name the stage at which the problem was found:

| Prefix | Source |
| --- | --- |
| `BH` | BELLHOP |
| `KR` | KRAKEN/KRAKENC |
| `PG` | The Python or MATLAB adapter, or command-line arguments |

| Digits | Stage | Typical cause |
| --- | --- | --- |
| `00xx` | Reading input | File missing or unreadable; wrong file type |
| `01xx` | Parsing | Malformed legacy record or JSON; input too large |
| `02xx` | Validation | A value out of range, inconsistent, or unsupported |
| `03xx` | Solving | No valid result: no modes in the limits, non-finite values, work limits; `0304` is cancellation |
| `04xx` | Writing output | Output conflict, quota or file-system error |
| `10xx` | Warnings | The run continues |

Adapter codes:

| Code | Meaning |
| --- | --- |
| `PG0001` | The adapter could not read or write a file, or start `pelagic` |
| `PG0002` | Invalid command-line arguments or adapter options |
| `PG0103` | A value could not be converted to JSON, such as an unsupported Python or MATLAB type |
| `PG0201` | Unknown model name |
| `PG0202` | The `pelagic` program's version or protocol does not match the toolbox |
| `PG0402` | Native output from Python could not be written safely |
| `PG0403` | Warning: a temporary directory could not be removed |
| `PG0404` | `run-many` could not write its status file, or refused one that is a link, a case or another program's file |

A diagnostic's `field` names the input it refers to. For JSON it is the path
of the offending value, such as `frequencies[0].propagation`. For legacy files,
use it together with the reported file, line and column.

## Reporting a problem

Please [open an issue](https://github.com/leihuang1/pelagic/issues/new/choose)
with:

- the output of `pelagic version --json`, or `pelagic.__version__` in Python;
- the smallest case that shows the problem: a JSON file from
  `pelagic … export`, or the `.env` file and its auxiliary files;
- the full error, ideally with `--diagnostics json`;
- for numerical differences, how you produced the comparison result.

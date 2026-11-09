# Command-line guide

The `pelagic` program validates, converts and runs cases, and writes the results
to a directory. See [Getting started](../getting-started.md#command-line) to
install it. `pelagic --help`, and `--help` on any subcommand, list every option.

```text
pelagic version                                  show version and pinned reference
pelagic bellhop  validate|export|run|run-many    BELLHOP rays, arrivals, fields
pelagic kraken   validate|export|run|run-many    KRAKEN/KRAKENC modes and FIELD
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
`--max-output-bytes`. Single-profile KRAKEN cases with several frequencies and
common native headers/grids write one `case.mod` and one `case.shd`.
Multi-profile runs or incompatible headers/grids keep `case.f0000.mod/.shd`,
`case.f0001.mod/.shd`, and so on instead;
see the [layout rule](native-output.md#file-selection-and-ordering).

### Scratch results: `--no-sync`

Both `pelagic kraken run` and `pelagic bellhop run` accept `--no-sync`, for
scratch work that is cheaper to rerun than to sync. The default remains durable:
file contents are synced before installation. With the flag, all per-file syncs
and Apple's drive-cache flush are skipped, in every output format. Writing,
closing, hashing, quotas, locks, backups, installation and rollback are unchanged.
**After an OS crash or power loss, results may be empty, truncated or missing
even though they were installed and, in a sweep, recorded as `ok`.** Neither
mode guarantees directory-entry durability. A scheduler time limit, preemption,
`scancel` or Pelagic process crash alone does not discard completed results:
the OS remains running and normally writes its cache back. An OS crash, node
failure or power loss can lose that cache. A write error that the OS hits later,
such as a failing disk or a full network filesystem, also goes unseen: no sync
reports it, so the case does not fail. An unfinished publication can still leave
incomplete scratch data, a partial installation or locks in either mode.

```sh
pelagic kraken run pekeris.env --output scratch/pekeris --no-sync --threads 1
```

## Threads

A run uses every CPU available to the program. On Linux that honours CPU
affinity (`taskset`, batch schedulers) and container CPU quotas. Results are
bit-for-bit the same for any number of threads, so the setting only changes
the speed:

```sh
pelagic --threads 4 bellhop run case.json --output results/case
PELAGIC_THREADS=1 pelagic kraken run case.json --output results/case
```

`--threads` goes before or after the subcommand, like `--diagnostics`.
`PELAGIC_THREADS` applies when the flag is absent. If you run several `pelagic`
processes at once, for example from a job array, `xargs -P` or MATLAB `parfor`,
give each one a share of the CPUs so that they do not compete.

BELLHOP shares the rays of each source between threads. KRAKEN shares the mode
search, the mode shapes and the FIELD receivers. On eight or more threads, KRAKEN
frequencies and range-dependent profiles also solve side by side after the first,
and their results still arrive in order, so about 128 MiB of finished results can
wait in memory. Fewer solve together if their results are large. If a frequency
fails, any later ones already solved are discarded. Real KRAKEN with
finite elastic layers solves them one after another, because each one continues
from the last. So, conservatively, does real KRAKEN with an elastic surface
half-space.

## Many cases

`run-many` solves many independent cases in one command, each into its own
result directory, for parameter sweeps on a workstation and job arrays on a
cluster:

```sh
pelagic kraken run-many cases/ --output results
pelagic bellhop run-many a.json b.env --output results --format both
find cases -name '*.json' > cases.txt
pelagic kraken run-many --list cases.txt --output results --shard 3/50 --resume
```

- **Inputs** are case files (`.json` or legacy `.env`), directories, whose own
  `.json` and `.env` files are taken but not their subdirectories, and
  `--list FILE` with one path per line (`--list -` reads stdin). A list avoids
  the command-line length limit when there are many cases. The cases are
  sorted by path. A legacy KRAKEN `.env` uses its same-stem `.flp`, and
  `--solver` applies to every case, as in `run`.
- **Results.** Each case is written to `DIR/<stem>/` exactly as
  `run CASE --output DIR/<stem>` writes it, with the same files and the same
  protection for files Pelagic does not own. If two cases have the same stem,
  even differing only in letter case, the command exits with code 2 before
  solving anything. A directory holding both `gulf.env` and `gulf.json` is
  rejected that way; list the files you want instead.
- **Scratch sweeps.** Both models accept `--no-sync` here too. It skips every
  file sync and Apple's shared run-many drive barrier; the default still syncs.
  After an OS crash or power loss, results may be empty, truncated or missing
  even though they were installed and recorded as `ok`. Use it only when
  rerunning and, if needed, manually recovering damaged output is acceptable.
- **Failures.** A failed case does not stop the others. The exit code is the
  largest of the cases' codes, so 0 means every case succeeded or was skipped.
  The status file says which cases failed and why.
- **CPUs.** By default there is one worker per case, up to one per available
  CPU, and each case solves on the CPUs left per worker. `--workers` and
  `--threads` set the split yourself; with only `--threads`, there are CPUs ÷
  `--threads` workers. Results are bit-for-bit the same for any split. At most
  one case per worker is held in memory.
- **Messages.** stdout has one `wrote …` line per case and a final count. On
  stderr, each text line starts with the case path, and with
  `--diagnostics json` each line is `{"case": …, "diagnostics": […]}`.
- KRAKEN's `--max-output-bytes` applies to each case's directory.

### One case or many

KRAKEN receivers do not need a case each. When only the receivers change, put
every range, depth and offset in one case: it solves the modes once and computes
all of its receivers from them, which is much faster than solving the same modes
again for each case. The grid is every combination of source depth, receiver
depth and range, so a large union costs more FIELD work and counts towards the
field limit.

A change of frequency, medium, boundary, phase-speed limits, mesh or `max_range`
changes the modes, so it needs its own case. Choose `max_range` for the farthest
receiver in the case. If you set `mode_depths` yourself, they must reach every
receiver depth.

### Shards

`--shard I/N` solves only the cases whose position in the sorted list,
counting from 0, leaves remainder `I` when divided by `N`. The `N` shards
together cover every case exactly once, and neighbouring cases, which often
cost about the same, go to different shards. Every shard checks the whole list
for clashing stems, so all of them accept or reject the same input. See
[sweeps on a cluster](cluster.md) for Slurm and PBS job arrays.

### Status file and resuming

Each case appends one JSON line to `DIR/status.ndjson`, or to
`DIR/status-I-of-N.ndjson` for a shard. A new `ok` follows installation and,
by default, all file syncs before it. `--no-sync` skips those syncs and Apple's
shared drive barrier, not installation or status reporting. By default,
single `run` and each sweep worker sync their own
files concurrently with at most 16 I/O participants including the caller.
Helpers are shared across the process, at most 64, and only use open files
left after 8 per sweep worker and 32 for the process. Without that room,
workers sync one file at a time, as before. At startup the CLI raises its soft
open-file limit to the hard limit. None of this changes the numerical
`--threads` setting. Linux and Windows
workers do not coordinate publication across cases. Apple can share its final
drive-cache flush without a batching timer. This does not promise directory-entry
power-loss durability. The status file itself is not fsynced per line:
a crash can lose a record or result names,
or cut a line short. Resume rehashes the manifest and every listed artifact.
See [barriers and interrupted batches](native-output.md#run-many-barriers-and-interrupted-batches)
for the exact platform guarantees and recovery rules:

```json
{"status_version":1,"case":"cases/a.json","stem":"a","version":"0.6.2","format":"legacy","options":{"max_output_bytes":268435456,"solver":null},"input_sha256":"5d0c…","outcome":"ok","exit_code":0,"seconds":0.03,"manifest_sha256":"4a3b…","diagnostics":[]}
```

| Field | Meaning |
| --- | --- |
| `status_version` | `1` |
| `case`, `stem` | The input path as given, and the result directory `DIR/<stem>/` |
| `version` | The Pelagic version, which the results record |
| `format` | `legacy`, `hdf5` or `both` |
| `options` | Options that change the results: `{"solver": …, "max_output_bytes": …}` for KRAKEN, `{}` for BELLHOP |
| `input_sha256` | SHA-256 over every file the case read, auxiliary files included, each as its length in 8 little-endian bytes followed by its bytes. `null` if the case could not be loaded: a BELLHOP case or legacy KRAKEN files that cannot be read, parsed or validated, or a KRAKEN JSON file that cannot be read. A KRAKEN JSON file that was read keeps its hash even when it is invalid |
| `outcome` | `ok`, `failed` or `skipped` |
| `exit_code` | What `run` would have returned: `0`, `2`, `3` or `4` |
| `seconds` | Wall time for the case |
| `manifest_sha256` | SHA-256 of the case's `pelagic-manifest.json` after an `ok` or `skipped` case, otherwise `null` |
| `diagnostics` | The case's errors and warnings, as `--diagnostics json` prints them |

Each case is read once: the bytes that are fingerprinted are the ones that are
solved, even if a file changes during the run.

With `--resume`, every status file in `DIR` is read, and a case is skipped
when an `ok` or `skipped` line for its stem has the same `case`, `version`,
`format`, `options` and `input_sha256`, and its result directory still
matches that line's `manifest_sha256` with every listed file unmodified.
Anything else runs again: any change to an input file, another option or
version, a failure, a case without a complete line, or results that were
edited or removed. A rerun into results you have edited fails with exit code
4 and keeps your edits, as `run` does. A crash that leaves scratch, locks or
backups can require manual recovery: resume never adopts or removes them.

This also applies after `--no-sync`: an empty, truncated or zero-filled artifact
fails the size/SHA-256 check and is never skipped as current. The case is rerun,
but replacing a damaged existing file still fails the ownership check; inspect
and recover or remove the damaged data before retrying. Follow the worked
[recovery procedure](cluster.md#recovery-after-a-node-crash); do not just delete
the manifest or discard backups you still need. Missing owned files can be
regenerated. The flag is not recorded in the manifest or status file because
it does not change results. A later `--resume` without `--no-sync` rehashes, but
**does not sync skipped cases or upgrade their durability**. Rerun without
`--resume` and without `--no-sync` to publish them durably.

`run-many` appends only to its own status files. It stops with exit code 4
(`PG0404`) before solving anything if the status file is a symbolic link, has
other hard links, is one of the cases, or has content that does not start with
`{"status_version":`, as every status line does. A first line that a killed
job cut short still counts as a status line, so `--resume` continues after it.

Rely on the field names and `status_version`, not on the key order.
`pelagic version --json` reports the version as `run_many_status_version`.

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
  schema versions, the diagnostics protocol and `run-many` status versions,
  and the pinned reference.
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

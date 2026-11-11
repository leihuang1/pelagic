# Pull-based Linux x86 measurement kit

One standard-library Python entry point fetches exact sources, builds isolated
binaries, measures them serially, and packages **summaries and hashes**, not raw
acoustic outputs. Nothing is built in the checkout containing this kit.

```sh
python3 tools/perf/x86/run.py \
  --ref main=7ac64e127ab103ea6123037cc3945e53eae376cc --out RESULTS
```

Each label must be unique. A ref is a commit SHA (at least seven digits), branch,
tag, or `pr:N` (fetched from `refs/pull/N/head`). Full resolved commits are recorded
and frozen on resume. Use full SHAs for a repeat experiment: a PR can move.
One current-main ref tests durable versus `--no-sync`; two refs are only needed
when comparing commits, for example:

```sh
python3 tools/perf/x86/run.py \
  --ref before=b058f5464e0de081a135e7ead821361294ce641b \
  --ref after=7ac64e127ab103ea6123037cc3945e53eae376cc --out COMMITS
```

The first ref supplies the **common inputs and unchanged solver driver** for all
binaries. Changes to the input fixtures of subsequent refs are not benchmarked.

## Requirements and safety

- Linux, Python 3.9+, Git, a C/C++ compiler, make, curl, `gfortran`, **CMake ≥3.26**,
  `lscpu`, `findmnt`, and GNU `xargs`. Network access to GitHub, codeload,
  crates.io and Rust distribution servers. No third-party Python packages.
- Rust is taken from each ref's `rust-toolchain.toml`. Existing rustup is used;
  otherwise a missing Rust installation is bootstrapped **inside RESULTS**, with
  no root or shell-profile changes. A module-provided Rust must match the pin.
  **Cargo home is always private under `--out`**, including with preinstalled Rust.
  Existing `RUSTUP_HOME` toolchain storage may stay shared: that is the exception
  to build-write isolation. Missing Rust uses private rustup storage too.
- Missing system tools produce an error. Only `--install-system` permits
  `apt-get update/install` (Debian/Ubuntu, root or noninteractive sudo). Never
  use that flag on a managed cluster; load the site's modules instead.
  Bookworm's packaged CMake 3.25 is too old for the pinned HDF5 dependency; use
  a newer CMake (for example a private venv with `pip install cmake==3.31.10`).
  This is a build tool, not a Python dependency of the kit.
- `--out` must be an empty dedicated directory or this kit's existing run. It
  contains sources, the Cargo home/cache, targets, subprocess `TMPDIR`, and Fortran
  trees, as well as results. Allow several GB; don't use a production directory.
- Ambient `CARGO_BUILD_*`, `CARGO_TARGET_*`, `CARGO_PROFILE_*`, encoded Rust flags,
  `RUSTFLAGS`, `RUSTDOCFLAGS`, `RUSTC*` (including wrappers), native compiler flags,
  `CMAKE_*`, and make controls (`MAKEFILES`, `GNUMAKEFLAGS`, `MAKEFLAGS`, `MFLAGS`)
  are removed. `CC`/`CXX` compiler selections are preserved: use executable names
  or paths, not commands with flags. Preflight and provenance use those compilers,
  falling back to `cc`/`c++` when unset or empty; Rust's linker uses the selected C
  compiler too. Fortran stays pinned to `gfortran`, with kit flags on make's command
  line. The kit sets its own target directory and `CARGO_INCREMENTAL=0`.
  Module `PATH`/library paths and proxy/registry settings
  (`CARGO_HTTP_*`, `CARGO_NET_*`, `CARGO_REGISTRIES_*`) are retained; provenance
  records network variable **names only**, never their values.
- Builds refuse `.cargo/config` or `.cargo/config.toml` in the source tree or any
  ancestor, and `config`/`config.toml` in the private Cargo home: these can silently
  override the recorded variants. For ancestor/private-home configs, move `--out`
  outside that ancestry, or move the config before building. **Refs with their own
  Cargo config are not supported yet**: choose another ref, rather than editing
  the pinned source. The kit does not edit or temporarily hide user configs.
- The default and `codegen-units=1` builds also work on arm64 for smoke testing.
  v3 builds run only when Linux exposes all required x86-64-v3 CPU/OS features.
  Unsupported v3 is explicitly recorded, not replaced by `native`.
- Refs execute their build scripts and binaries: choose trusted repository refs.
  Do not edit inputs, source trees or binaries while a run is active.

## What it measures

All builds finish before any timing. Every CLI and solver variant has its own
Cargo target directory; Fortran O3/O1 have separate, clean source/build trees.
The source archive's commit and SHA-256 are read from the first ref's reference
build script, cross-checked against its Dockerfile, and verified before extraction.

| Experiment | Full run | Quick smoke test |
|---|---|---|
| CLI builds per ref | default, `-C target-cpu=x86-64-v3`, `-C codegen-units=1` | same (unsupported v3 skipped) |
| Solver harness per ref | default, v3 | same |
| Solver timing | all cases in `tools/perf/cases.json`; 7 interleaved rounds × 3 samples, one warmup per process | Pekeris + BELLHOP Munk TL; 1 round × 1 sample |
| Single-case end-to-end | 6 KRAKEN cases below + BELLHOP Munk TL; 15 rounds after one warmup | all 7 cases; 1 warmup + 1 measured round |
| Sweeps | Pekeris ×200, surface duct ×50; 1 warmup + 3 measured rounds | both workloads ×4; 1 warmup + 1 measured round |
| Solver identity | full upstream inventory, threads=1 | the 2 quick solver cases |

Builds use `cargo build --release --locked`, not release packaging. On x86,
`default` has the ISA settings of the shipped `linux-x64` CLI (no extra ISA flags),
and `v3` those of `linux-x64-v3` (`-C target-cpu=x86-64-v3`). cgu1 changes only
codegen units from default. This is not a byte-for-byte release-package rebuild.

The six KRAKEN inputs are Pekeris, MunkLossless, surface duct, Gulf adiabatic,
broadband (101 frequencies), and elastic sediment, as in the previous EPYC kit.
End-to-end tests use their ENV and adjacent companion files; both implementations
receive the same bytes. BELLHOP uses the Munk TL ENV. Inputs are hashed before use.

Every trial parses **every SHD** and checks record completeness, frequencies from
its frequency vector (not FIELD's unset `freq0`), and the six geometry dimensions.
Source-depth and receiver-depth vectors must fit their records. Only rectilinear
layouts are supported: `rectilin`, plus the pinned FIELD writer's exact ten-space
legacy tag **only for Fortran KRAKEN/FIELD**. Irregular, unknown, and all-NUL tags
are rejected as workload errors; Pelagic and BELLHOP must use `rectilin`.
Coverage is the sorted list of `(frequency, geometry dimensions)` entries across
all SHDs. One multi-frequency file and many indexed single-frequency files can
therefore match. This handles current grouped broadband output and older layouts.
Every Pelagic/Fortran variant must have equal coverage. Malformed, missing or
unequal products fail as **workload errors**, and are excluded from timings.
For sweeps, every expected case must independently match single-case coverage;
CLI `status.ndjson` must contain exactly one `ok`/exit-0 record per expected case.
Single-case warmups precede sweep warmups. Errors stop further blocks.

Every CLI variant runs durable, plus `--no-sync` **only if that model/subcommand's
`--help` advertises it**. Single cases use `--threads 1 --format legacy` and a
fresh output directory. Fortran runs KRAKEN then FIELD directly, or BELLHOP alone.
Fortran flags are:

```text
O3: -O3 -march=native -ffast-math -funroll-all-loops -fomit-frame-pointer -std=gnu
O1: -O1 -ffast-math -funroll-all-loops -fomit-frame-pointer -std=gnu
```

`-I../misc` is added outside `misc`. O1 reproduces upstream optimization flags,
not the reference Docker image's compiler version. Bundled stale objects are
removed before building.

Sweeps use **`run-many --workers W --threads 1`** against Fortran O3 and O1 through
`xargs -0 -n 1 -P W sh -c …` (KRAKEN then FIELD, checking products per case).
W is 1, the physical-core count, and the logical-CPU count, deduplicated. Counts
and affinity respect the process's allowed cpuset, not every CPU on the host.
One-worker work is pinned to one CPU; physical-core sweeps use one sibling per
core; logical sweeps use all allowed CPUs. Quick batches can be smaller than W.

Before each timing block the kit waits for one-minute load ≤ `max(2, CPUs/10)`.
Solver timing reuses `tools/perf/run.py bench`, wrapping only `invoke` to wait and
record load/affinity. CLI block order and variant order are seeded; solver rounds
use the seed plus round number. `--seed N` overrides the recorded seed. Waits
are logged; ten minutes continuously above the ceiling is a failure, not a noisy
success. `--max-load N` can set a site-appropriate ceiling; disclose overrides.
Inputs/caches are warm, outputs are fresh. Copying, hashing and cleanup are
outside timers. Process startup, computation, serialization and publication are
inside CLI/sweep timers. No builds run concurrently with measurement.

Solver identity runs **before timing** and reuses `tools/perf/run.py check`:
default snapshots are the baseline for v3 **within each ref**. Each ref/variant
has its own checkpoint; default snapshots survive interruption until its v3 check
finishes, then are hashed/removed like other raw outputs. Completed checks are not
repeated on resume. Identity failures stop further timing blocks.

The CLI separately compares all MOD/SHD/ARR/RAY hashes against the default CLI,
including `codegen-units=1` and no-sync. Sweeps compare **each case separately**
against the same ref's default at the same workload and W, across warmup and
measured rounds. Fortran hashes are not an identity gate. Thus cgu1 has CLI-product
coverage, not an extra solver-harness build. Every file, including
operational metadata and logs, is hashed, but timestamps, paths and run-many
status timing fields are not numerical identity checks. Within-ref differences
make the run failed; cross-ref differences are reported as data, not rejected.
This is **not Fortran numerical acceptance**; see the
[acceptance workflow](../../reference/full-acceptance.sh).

## Results, deadlines and resume

```sh
python3 tools/perf/x86/run.py status --out RESULTS
python3 tools/perf/x86/run.py self-test
# Cheap coverage, not performance evidence (exactly one ref):
python3 tools/perf/x86/run.py \
  --ref main=7ac64e127ab103ea6123037cc3945e53eae376cc \
  --out SMOKE --quick --deadline-minutes 15
```

- `setup.done.json`, `build.done.json`, `measure.done.json`, and
  `summarize.done.json` checkpoint phases. Completed builds and measurement
  blocks (including per-variant solver identity checks) are reused. An interrupted
  block is retried in full, never mixed with its earlier incomplete attempt.
  Rerun the **same command and kit files**.
  Only the deadline and system-install permission may change for an existing run.
- `--deadline-minutes N` is a measurement budget **per invocation**, starting
  after builds. It is checked while waiting for load and between invocations.
  The current invocation finishes (timeout: 120 seconds per solver process,
  600 seconds per CLI/Fortran executable or whole sweep; KRAKEN + FIELD are two
  executables), then summaries are written as **partial**. It is not a VM
  billing watchdog or a build deadline. Resume gives unfinished work a new budget.
- Exit codes: 0 complete, 1 failed (including identity), 2 deadline/partial.
  `status` prints one line. A stale running state with no lock holder is shown
  as interrupted. A lock excludes concurrent runners; the boot identity,
  cpuset, inputs and binaries are checked before measurement.
- `summary.md` / `summary.json`: medians, min–max, sample counts and ratios to
  Fortran O3 for the same case/worker count. Sweep times are **ms per case**.
  Solver timings have no Fortran end-to-end ratio because scopes differ.
- `timings.jsonl`: observations, commands, order, load, affinity and product hashes.
  Warmups and failures are retained but excluded from statistics.
- `provenance.json`, `machine.json`: resolved commits, source archive pin, flags,
  toolchains, every input/binary digest, topology, kernel, memory and output
  filesystem. Local machine evidence also retains full mounts, boot ID and hostname
  for diagnosis/resume; the archive omits those three private fields and the
  hostname in `uname -a`. It keeps the output-filesystem `findmnt` row.
  `identity.json` retains single/sweep comparisons and solver diagnostics;
  `workloads.json` reports per-case frequency/geometry coverage and failures.
- `output-hashes.jsonl`: every generated output's SHA-256 and size, including
  snapshots and logs. Raw measurement outputs are then removed unless
  `--keep-raw` is set. Retained raw files stay on the measurement machine only.
- `results.tar.gz`: summaries/evidence only, **≤10,000,000 bytes**.
  `archive.json` records its size, SHA-256 and outcome. Exceeding the cap fails
  packaging; it never silently drops observations. Sources, targets, raw outputs
  and verbose build logs are not transferred. Absolute output paths become `<out>`
  and non-root home paths become `~` in transfer copies only. Only whole path
  tokens are replaced, including after `@`, redirections, pipes, `file://` and
  joined options such as `-I`, `-L` or `-isystem`. A name continues through
  letters, digits and `_ - ~ + @ % = # $ ^ * ( [ {`, and through `.`, `!` or `?`
  unless it ends a sentence; any other character ends it. So URLs, relative
  paths and siblings such as `user.backup` or `results#old` are left alone.
  `HOME=/` is never substituted; normalization refuses dictionary-key collisions
  rather than losing evidence. Tar owners are uid/gid 0 with empty user/group
  names. Keep the original output tree to resume.
- **Failed-run archives include diagnostic log tails** (about 4 KB each), including
  failed commands and interrupted measurements. Earlier failed attempts' tails can
  also remain after successful resume. Known network environment values are removed
  from diagnostics, but this is not general secret detection:
  **review archives before sharing them**.

A completed experiment is not automatically representative: Colab is shared,
load average is not CPU isolation, and overlay sync costs are not a cluster
filesystem. Fortran does not offer Pelagic's transactional durability contract;
whole Fortran SHD hashes can vary in unset headers. Short quick samples establish
plumbing, not a statistically useful speedup. No power-failure tests, cold-cache
measurements, compiler/CPU-frequency locking or physical-accuracy gate are run.

## Colab CPU through the CLI

Requires **Colab CLI ≥0.7.5**; set `COLAB_CLI_REQUEST_TIMEOUT=300`. Use a private
config for **every** CLI command, CPU only (no accelerator flag), and an external
wall-clock/billing watchdog armed **before allocation**. Record usage before/after.
Never mount Drive or inject VM credentials. Example shell session:

```sh
CFG="$PWD/colab-sessions.json"
export COLAB_CLI_REQUEST_TIMEOUT=300
# Install cleanup BEFORE new, including when allocation returns an error.
trap 'colab --config "$CFG" stop -s x86kit-cpu; colab --config "$CFG" usage' EXIT
colab --config "$CFG" usage
colab --config "$CFG" new -s x86kit-cpu
```

Before the kit is published, upload **only its three small Python files**:

```sh
for f in run.py measure.py selftest.py; do
  colab --config "$CFG" upload -s x86kit-cpu "tools/perf/x86/$f" "/content/x86kit-$f"
done
```

Save this short bootstrap as `bootstrap.py` locally. `exec` sends it directly;
the detached process, not the Jupyter connection, owns the long-running work.
The benchmark's sources still come from GitHub, never the upload.

```python
from pathlib import Path
import shutil
import subprocess
kit = Path('/content/x86kit')
kit.mkdir(exist_ok=True)
for name in ('run.py', 'measure.py', 'selftest.py'):
    shutil.copyfile('/content/x86kit-' + name, kit / name)
log = open('/content/x86kit.log', 'a')
p = subprocess.Popen(['python3', '-u', str(kit / 'run.py'),
    '--ref', 'main=7ac64e127ab103ea6123037cc3945e53eae376cc',
    '--out', '/content/RESULTS', '--quick',
    '--deadline-minutes', '15', '--install-system'],
    stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
print('runner pid', p.pid)
```

```sh
colab --config "$CFG" exec -s x86kit-cpu --timeout 60 -f bootstrap.py
printf '%s\n' "import subprocess; print(subprocess.check_output(['python3','/content/x86kit/run.py','status','--out','/content/RESULTS'],text=True),end='')" |
  colab --config "$CFG" exec -s x86kit-cpu --timeout 60
# Poll status; inspect the log on failure. Download only after terminal status.
colab --config "$CFG" download -s x86kit-cpu /content/RESULTS/archive.json archive.json
colab --config "$CFG" download -s x86kit-cpu /content/RESULTS/results.tar.gz results.tar.gz
# Compare local SHA-256 and size with archive.json before discarding the VM.
colab --config "$CFG" stop -s x86kit-cpu
colab --config "$CFG" usage  # must show 0 active assignments
trap - EXIT  # only after confirming zero assignments
```

After publication, replace the three uploads with a short bootstrap that clones
the repository and checks out the **full reviewed kit commit**, then launches
that checkout's entry point. The measured refs remain separate clones.

## Slurm (no root)

Request a whole node and the site's local scratch filesystem. Adapt module names
and time/memory to the cluster. Submit this from a checkout containing the kit:

```sh
#!/bin/bash
#SBATCH --job-name=pelagic-x86
#SBATCH --nodes=1
#SBATCH --ntasks=1
#SBATCH --exclusive
#SBATCH --time=02:00:00
set -eu
module load python/3.11 gcc cmake rust/1.99.0  # site-specific; no apt-get
export CC="${CC:-gcc}" CXX="${CXX:-g++}"  # keep module selections; no cc alias needed
cd "$SLURM_SUBMIT_DIR"
python3 tools/perf/x86/run.py \
  --ref main=7ac64e127ab103ea6123037cc3945e53eae376cc \
  --out "$SCRATCH/pelagic-x86-$SLURM_JOB_ID" --deadline-minutes 60
```

The module must also provide `gfortran`; `FC` does not override the reference
compiler. Compiler flags, make controls and `CMAKE_*` remain scrubbed, while
module compiler selections and library search paths are preserved.

Allow time after the measurement deadline for hashing/summarizing and note that
builds precede that budget. To resume after scheduler interruption, reuse the
same output path on the same running node with the same cpuset; after a reboot
or node change, start a new experiment instead. Download/copy only the tarball
and `archive.json`; clean up the private scratch tree when no longer needed.

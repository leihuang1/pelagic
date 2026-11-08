# Native output and result directories

The computation CLI defaults to `--format legacy`. `--format hdf5` selects the
existing BELLHOP v3 or KRAKEN v1 schema; `--format both` writes both from **one
solve**, not a second numerical run. All formats use `--output DIRECTORY`,
defaulting to the input's file stem in the current directory. Missing parents
are created. The old single-file CLI path and `--overwrite` flag are gone.
The BELLHOP-only [HTTP adapter](http.md) and its response formats are unchanged.

```sh
pelagic bellhop run examples/cases/bellhop/field-g/field-g.json --output results/field --format both
pelagic kraken run crates/kraken/tests/fixtures/Pekeris.env --output results/pekeris --format both
# results/pekeris/{Pekeris.mod,Pekeris.shd,Pekeris.h5,Pekeris.prt,pelagic-manifest.json}
```

## File selection and ordering

Numerical files follow the pinned Acoustics Toolbox v2023.5 reader layouts,
not compiler-specific Rust struct layouts. They are not byte-identical copies
of Fortran output. Padding is deterministic zero/space; text uses sufficient
round-trip precision. Fixed native titles are ASCII, space-padded/truncated;
the full UTF-8 title remains in HDF5 and the Pelagic report.

| Solver/result | Native products |
|---|---|
| BELLHOP rays or eigenrays | `.ray`, all retained trajectories and bounce counts |
| BELLHOP arrivals | `.arr`, ASCII or binary according to the input's arrival encoding |
| BELLHOP coherent/semi-coherent/incoherent field | `.shd` |
| KRAKEN/KRAKENC | `.mod` with all modes/profiles, plus `.shd` FIELD |

Names use the input stem, not the acoustic title. Non-UTF-8 filename bytes are
lossily displayed; control characters/backslashes become underscores and
reserved `.pelagic-` stems get a `case-` prefix. Input paths/snapshots themselves
are not changed.

KRAKEN emits a **separate native pair per ordered frequency block** when there
is more than one: `CASE.f0000.mod/.shd`, `CASE.f0001.mod/.shd`, etc. Each MOD
contains one frequency and all its profiles in input order. This preserves
repeated/descending frequencies and the already accepted JSON contract in which
blocks can have different profile counts, sample grids and FIELD geometry;
a shared native frequency grid cannot represent that contract. No frequency
result collection is retained. The `.prt` report maps indices, frequencies,
profile ranges and counts. HDF5 remains one `CASE.h5` with all indexed blocks.
Native MOD does not store profile ranges or FIELD offsets; use the original FLP
for those, or the report/HDF5 metadata, just as an original MOD reader needs FLP
geometry to synthesize FIELD. No reconstructed inputs or debug files are emitted.

JSON keeps its existing transport contract: arrival output is ASCII; legacy
lowercase `a` selects binary. Exporting to JSON does not carry that legacy
encoding choice into the physics document.

ARR has no irregular-grid flag. BELLHOP paired/irregular arrivals therefore use
`CASE.r000000.arr`, `CASE.r000001.arr`, etc., each containing that receiver's
1x1 geometry and all sources, in paired-input order. Very large paired arrival
grids can hit the 1 MiB manifest cap (file count depends on name lengths); use
`--format hdf5` in that case. This is an output/publication limit, not an input
or numerical-acceptance rejection. This avoids fabricated
unrequested off-diagonal arrivals and quadratic file size. Irregular SHD uses
the original `irregular` plot flag and one pressure row per source.

## Layout and units

- Binary files are little-endian. Counts and record lengths are signed int32;
  no sequential markers wrap direct-access MOD/SHD records.
- RAY is quoted-title ASCII, with frequency (Hz), source/angle dimensions,
  top/bottom depths, `'rz'`, then launch angle (degrees), point/bounce counts
  and every range/depth pair (metres). Eigenrays share this reader layout.
- ASCII ARR stores source/depth/range counts and coordinates, then maximum
  arrivals per source, per-receiver counts and all eight quantities: amplitude,
  **phase in degrees**, real/imaginary delay (seconds), source/receiver angles
  (degrees), and top/bottom bounce counts. Result amplitudes already include
  spreading; serialization never applies it again.
- Binary ARR uses matching 4-byte byte-length markers around every Fortran
  sequential record. Frequency/source/receiver depths are float32, receiver
  ranges float64. An arrival is eight float32 words (including bounce counts),
  with the same units/order as ASCII. Headers/counts have their own records.
- SHD has ten fixed-length header records, including int32 word length,
  80-byte title, 10-byte plot type, seven dimensions, float64 nominal frequency
  and attenuation, frequency/bearing/source-x/source-y vectors (float64),
  source/receiver depths (float32), and ranges (float64). Pressure is complex32,
  source then depth then range; each depth row occupies one record. BELLHOP's
  range-major result/HDF5 arrays are transposed at this adapter, not in the solver.
- MOD has five descriptive records per profile (length/title/dimensions,
  first-mesh fluid intervals and 8-byte `ACOUSTIC` tags, layer top/density,
  float64 frequency, float32 sampled depths). Then come mode count, packed
  top/bottom half-space properties, one complex32 pressure-shape record per
  mode, and folded complex32 wavenumbers (rad/m). Record length is constant
  across profiles and even, preventing complex-value splits. All modes are
  written, including those above the FIELD cap. Shapes remain fluid pressure,
  not unimplemented solid displacements. Half-space sound speeds use pinned
  `AttenMod::CRCI`'s **positive** imaginary part from canonical dB/wavelength,
  separately from the stored wavenumber's attenuation sign. Wavenumber sign/attenuation and the
  first-mesh shapes are not reinterpreted. Native float32 conversions reject
  non-finite/overflowing values rather than publishing corrupt records.

`.prt` is **Pelagic's own run report**, not the original printed style or an
original numeric oracle. Numerical files intentionally do not reproduce debug,
mesh-search or intermediate artifacts. These adapters do not extend numerical
capabilities, change the fixed acceptance target or certify arbitrary inputs.

## Ownership, quota and failure contract

`pelagic-manifest.json` v1 identifies implementation `Pelagic` and every owned
regular artifact's filename, byte length and SHA-256. It is an ownership
assertion, not a signature or tamper-proof provenance system; do not hand-edit
it. Only same-directory component names are accepted. Manifest reads/writes
are bounded to 1 MiB. Missing old artifacts may be regenerated; changed old
artifacts, malformed manifests, symlinks, or same-name files not recognized by
the manifest are errors. Unrelated files/subdirectories are left alone.
Consumed primary/auxiliary inputs and their symlink **and hard-link** aliases
are never authorized for replacement, even by a manifest.

A cooperating writer exclusively reserves `.pelagic-lock` and `.pelagic-stage`
inside an existing result directory. For a missing directory on Apple and Linux,
it instead reserves the temporary sibling `.pelagic-NAME-stage`, builds the whole
group there, and publishes it with one **no-replace directory rename**. An inner
lock travels with that rename and remains held until publication completes.
A destination that appears meanwhile is never overwritten or merged. Long
names and other platforms keep the original in-directory path; a filesystem
without exclusive directory rename support uses exclusive per-file installation.
Existing scratch/locks/backups, including the sibling reservation, are never
adopted, truncated or removed. Treat `.pelagic-*` paths as reserved control data.

New artifacts are completely written and closed, then read back for SHA-256
and their file contents synced before installation. File identity, size and
timestamps alone cannot prove that a closed file was not modified: a same-size
rewrite can occur within one filesystem clock tick. Publication, old owned
artifacts, rollback targets and resume verification therefore retain read-back
hashing on every platform.
Both single `run` and `run-many` sync the verified artifacts and manifest
concurrently, with at most **16 participants including the calling thread**.
Smaller publications use fewer participants. Each helper holds one open file,
so helpers are shared across the process: at most 64, and at most a quarter of
the open files left after 8 for each `run-many` worker (one, for a publication
outside a sweep) and 32 for the process. When none is free, the calling thread
does the work, so publishing never needs more open files than one-at-a-time
syncing did. The CLI first raises its soft open-file limit to the hard limit
(at most 10240 on macOS); library callers keep their own limit, which is read
again for every publication. Scoped helpers finish before
installation; if a helper cannot be started, the calling thread and already
running helpers finish its work instead. Every file still gets its own sync,
and the first file error in filename order fails the publication. Helper-start
failure is not an output error.

Linux, Windows and other non-Apple targets use each file's `sync_all` (including
Windows `FlushFileBuffers`). Apple uses ordinary per-file `fsync`, then one
`F_FULLFSYNC` on the manifest after all file syncs succeed, flushing the shared
drive cache before installation. Apple sweeps can share this final drive flush,
as described below. No file sync is skipped or replaced by a filesystem-wide
operation. The I/O helpers do not change the numerical `--threads` setting.

KRAKEN keeps the positive `--max-output-bytes` default of 256 MiB: the known
FIELD payload is admitted before solving, HDF5 payload/flush checks remain, and
the cumulative physical size of all selected products, report and manifest is
checked. A running physical-byte total counts closed native files once and
refreshes the growing report/HDF5 files at the same check points; installation
still scans the entire stage. Native direct-record writers also check remaining
bytes before each write. As before, HDF5 has no byte-limited VFD, so a bounded
header/frequency write can temporarily exceed its physical quota before a
flush/check. BELLHOP does not inherit KRAKEN's byte quota. Numerical
mesh/mode/shape/FIELD work budgets are unchanged.

After all frequencies succeed, verified previous artifacts are moved into
`.pelagic-backup`; new files are exclusively renamed into place on Apple and
Linux targets, with the manifest installed last. Filesystems/kernels without
no-replace rename support fall back to exclusive hard links; other targets keep
the hard-link path. Rollback reads installed files against their pre-install
fingerprints and returns renamed files to scratch before restoring old
artifacts. Stale owned products are removed on success (for example `.h5` after
switching to legacy), without deleting unrelated files.
An installation error restores the previous group. A rollback error is an
explicit output failure naming backup/stage/lock locations; recoverable data
and the lock are retained for manual recovery. Never delete a retained backup
before restoring its old artifacts. Cleanup errors after installation explicitly
say results are installed and identify retained data.

**Replacing an existing group is not an atomic snapshot for concurrent readers.**
Readers can observe that installation window; writers must honor the lock.
The fresh-directory rename exposes the complete group at once, but neither path
is a process-crash recovery journal or automatic recovery. Neither `run` nor
`run-many` promises directory-entry power-loss durability. External noncooperating same-user
filesystem mutation is not covered by the lock. Filesystem hard-link/rename
support is required. Numerical/write/quota failure before installation leaves
all previous results intact; handled installation failures are regression-tested
with deterministic fault injection, including failed rollback and late unrelated
destinations.

### Run-many barriers and interrupted batches

Workers finish writing and verifying their staged files, then sync every file
concurrently as above. Only after all required syncs succeed may installation
move previous products. There are **no additional result-directory, parent or
ancestor syncs**: this preserves the single-run file-durability contract, not a
stronger promise about directory entries.

- **Linux, Windows and other non-Apple targets:** each worker independently
  syncs its own case's files. There is no cross-case coordinator, kernel-release
  check or filesystem-wide flush. A file error fails its own publication;
  other cases can still succeed. Each publication uses at most 16 I/O
  participants, and helpers across the sweep share the process-wide cap.
- **Apple:** after each case's concurrent ordinary file syncs succeed, ready
  cases on the same filesystem can share `F_FULLFSYNC`. One-worker pools flush
  independently. Pools of two to four workers allow W drive barriers in flight,
  avoiding a second wait behind another case's flush. Larger pools coalesce
  behind one active barrier. A shared drive-flush error fails every covered
  case on that filesystem, without moving their previous products.

On Apple the first ready case starts immediately if a barrier slot is free;
otherwise all cases ready when a slot becomes free share the next barrier.
There is no batching timer or minimum group size. A case waits for its own
covered barrier, not an earlier flush that may have begun before its writes
finished. There is no cross-case sharing between separate CLI processes.

A new **`ok` status line** means all artifact and manifest file contents passed
the platform's durability barrier, and installation and ordinary cleanup then
completed. It does **not** guarantee that their names, renames, directory
entries, cleanup removals or the status line survive power loss. A `failed`
line makes no claim about new products; a `skipped` line rehashes the recorded
group and inherits its earlier publication's guarantees. All guarantees depend
on the filesystem, storage server and hardware honoring sync operations.

The status file is appended/flushed to the OS, **not fsynced per line**. A crash
can lose a record even when its products survive, or leave a record whose
result names no longer exist. No new `ok` line precedes its products' barrier
and installation. The crash states are:

| Interruption point | Possible state and resume behaviour |
|---|---|
| Writing scratch or waiting for a barrier | No new success record. Scratch/reservation may remain; old products have not moved. |
| Handled barrier failure | No installation or new success record; previous products are intact. Normal owned-scratch cleanup is attempted. |
| Directory rename or per-file installation | No new success record. A fresh group may be visible; replacement may be partial, with backups. |
| Installation completed, before cleanup or record | File contents passed the barrier, but names have no power-loss guarantee; the record can be absent and controls can remain. |
| Appending the record | A complete, truncated or missing line, possibly with missing result names after power loss. Resume ignores truncated lines and revalidates every listed artifact. |

`--resume` skips only a matching complete earlier record whose manifest **and
all artifacts** still verify. An installed group without a record runs again,
subject to the same ownership checks. It never adopts abandoned scratch or
locks: an interrupted transaction can require manual recovery before retrying.
Inspect retained backups and restore old artifacts if necessary before removing
control paths; do not simply delete recoverable data. Recorded, verified cases
can be skipped even if a crash left their control directories behind.

## Validation

Independent readers check declared record sizes, marker pairs, precision,
units, dimensions, source/depth/range order, frequency/profile order, all
wavenumbers/shapes/pressures/arrival quantities and complete EOF. Actual CLI
`both` products are read back, not copied into a simulated CLI result. The
pinned differential gates still compare every accepted numerical path at their
existing tolerances; native readers check the same CLI products alongside
HDF5. No MATLAB installation is required. Old legacy/golden bytes and SHA
manifests remain fixed comparison evidence, not new Rust native outputs.

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
inside the result directory. Existing scratch/locks/backups are never adopted,
truncated or removed. New artifacts are completely written, closed, hashed and
file-synced before installation. KRAKEN keeps the positive `--max-output-bytes`
default of 256 MiB: the known FIELD payload is admitted before solving, HDF5
payload/flush checks remain, and the cumulative physical size of all selected
products, report and manifest is checked. Native direct-record writers also
check remaining bytes before each write. As before, HDF5 has no byte-limited
VFD, so a bounded header/frequency write can temporarily exceed its physical
quota before a flush/check. BELLHOP does not inherit KRAKEN's byte quota.
Numerical mesh/mode/shape/FIELD work budgets are unchanged.

After all frequencies succeed, verified previous artifacts are moved into
`.pelagic-backup`; new files are exclusively hard-linked into place, with the
manifest installed last. Stale owned products are removed on success (for
example `.h5` after switching to legacy), without deleting unrelated files.
An installation error restores the previous group. A rollback error is an
explicit output failure naming backup/stage/lock locations; recoverable data
and the lock are retained for manual recovery. Never delete a retained backup
before restoring its old artifacts. Cleanup errors after installation explicitly
say results are installed and identify retained data.

**This is not an atomic snapshot for concurrent readers**, a process-crash
recovery journal, automatic recovery, or a directory power-loss durability
promise. Readers can observe the installation window; writers must honor the
lock. External noncooperating same-user filesystem mutation is not covered by
the lock. Filesystem hard-link/rename support is required. Numerical/write/quota
failure before installation leaves all previous results intact; ordinary
handled installation failures are regression-tested with deterministic fault
injection, including failed rollback and late unrelated destinations.

## Validation

Independent readers check declared record sizes, marker pairs, precision,
units, dimensions, source/depth/range order, frequency/profile order, all
wavenumbers/shapes/pressures/arrival quantities and complete EOF. Actual CLI
`both` products are read back, not copied into a simulated CLI result. The
pinned differential gates still compare every accepted numerical path at their
existing tolerances; native readers check the same CLI products alongside
HDF5. No MATLAB installation is required. Old legacy/golden bytes and SHA
manifests remain fixed comparison evidence, not new Rust native outputs.

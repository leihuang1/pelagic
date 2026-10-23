# Pelagic architecture

Seven workspace crates (Rust crates remain unpublished; Python is packaged separately):

```text
cli (pelagic) ──> bellhop, kraken, output
server (pelagic-server) ──> bellhop, output
output ──> bellhop, kraken, native-output
python (pelagic._core) ──> bellhop, kraken, native-output
native-output ──> bellhop, kraken
```

`bellhop` and `kraken` are independent solver Modules. Each exposes validated
Cases and its own results; no common Case, result model, JSON schema, solver
trait or real/complex root engine is introduced.

```text
crates/{bellhop,kraken}/src/
  lib.rs          Interface exports
  model.rs        unvalidated input types
  case.rs         validation and immutable Case invariants
  diagnostic.rs   diagnostics
  result.rs       numerical products
  input/          legacy/JSON parsing and acquisition
  solver/         solver-local numerical Implementation
```

Root exports remain convenient Rust Interfaces; their implementations live in
the responsibility directories. Files follow actual concepts rather than line
counts. Shared layout does not imply symmetric arithmetic: KRAKEN/KRAKENC
root searches and FIELD precision stages retain their numerical Locality.
Legacy material conversion lives with KRAKEN input; its solver consumes
canonical solve-frequency material. See the [implementation map](kraken-internals.md).

`cli/src/{bellhop,kraken}.rs` retain solver-specific argument handling behind
one dispatch entry. `server` is an independent HTTP Adapter, still BELLHOP-only.
Neither introduces solver-independent transport models.

`output/src/{bellhop,kraken}.rs` own HDF5 v3/v1 separately.
`native-output/src/{bellhop,kraken}.rs` own solver-specific native products;
`native.rs` shares only pinned record/primitive I/O. This crate also owns
`directory.rs` and its file-group safety tests; `output` re-exports the directory
API for existing callers. It has no HDF5 dependency. BELLHOP's borrowed
`result::FieldGrid` centralizes depth-major pressure iteration for native SHD and
Python without changing the range-major solver/HDF5 representation. ARR retains
its own record ordering.
KRAKEN exposes its existing first-mesh interval policy for MOD metadata rather
than duplicating that algorithm. One ordered frequency iterator writes both
formats before dropping each result; cross-frequency solver state is retained.

`native-output/src/directory.rs` owns file-group publication, verified artifact ownership,
exclusive scratch/lock and handled-error rollback. The separate single-file
publisher stays available to existing Rust callers. HTTP still uses private
HDF5 files and its unchanged protocol; it never adopts CLI defaults.
See the [exact publication scope and native layouts](../guide/native-output.md).
HDF5 schemas, precision and numerical contents are unchanged.

Python wraps validated immutable Cases and original results, with read-only NumPy
snapshots and in-process native serialization. Solver-local thread-scoped
cancellation checks add no language dependencies or new physics. Independent cases
may run concurrently; KRAKEN frequency blocks retain ordered solver state. See the
[public bindings contract](../guide/bindings.md).

PR 1 isolated structural changes; PR 2 adds native adapters, format selection
and safe result-directory updates as an independently checked capability.
The original numerical/input target, references, tolerances, work budgets and
legacy fixture bytes remain unchanged. There is no new universal core or
solver-independent Case/result model.

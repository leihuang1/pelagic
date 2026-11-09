# Pelagic: acoustics inputs and results

Shared vocabulary for code, docs and reviews. Use these terms with exactly these
meanings, and avoid the listed alternatives. For structure, see
[architecture](docs/development/architecture.md); for public promises, the
[interface contract](docs/development/interface-contract.md).

BELLHOP and KRAKEN/KRAKENC are independent **Modules**, with separate models,
numerical implementations, JSON schemas and HDF5 schemas. The `pelagic` CLI,
the BELLHOP-only `pelagic-server`, the in-process Python binding and the
CLI-driven MATLAB/Octave toolbox are **Adapters** over them. Adapters share each
Module's JSON contract (schema version 2) and Rust validation, and add no physics.

## Language

**Case**:
An immutable validated acoustic input, constructed from an unvalidated case definition. A KRAKEN case describes one modal environment and its FIELD geometry; a BELLHOP case describes a complete ray/beam calculation.
_Avoid_: Job, generic acoustics configuration

**FIELD case**:
One KRAKEN solve frequency with ordered modal profiles, profile ranges and range-independent, adiabatic or coupled propagation. Frequency order and repetitions belong to the enclosing input sequence.
_Avoid_: Broadband response, time-domain response

**Frequency block**:
A FIELD case at its index in the enclosing input sequence, with its result once solved. The index identifies it, because frequencies can repeat, and names its HDF5 group. Physical native artifacts may contain several blocks: the shared native writer chooses the run's stem for a combined pair or an only block, and indexed stems otherwise.
_Avoid_: Frequency result, per-frequency output

**Frequency-block stream**:
The one order in which a KRAKEN run solves and writes its frequency blocks, through a single solve iterator, with HDF5 plugged in as a block writer. On a failure, this order decides which diagnostic wins and what partial state exists. Python writes native files from finished results without solving again; it and the stream use the same `native_output::kraken::Writer` for layout selection and serialization.
_Avoid_: Writer loop, adapter-side write sequence

**Input material**:
A parsed finite medium or half-space, from an ENV or a JSON case, whose absorption still uses the input's selected unit, volume-loss model and per-medium power law. It becomes canonical dB/wavelength only for a selected solve frequency, by the same conversion for both inputs.
_Avoid_: Legacy material, canonical layer containing raw loss, parallel power-law arrays

**Modal refinement**:
The bounded sequence of meshes retaining raw roots for later seeds, a separate Richardson eigenvalue table and first-mesh shapes/group speeds. KRAKEN retains first-mesh loss perturbations; KRAKENC extrapolates complex squared wavenumbers.
_Avoid_: Shared root solver, extrapolated mode shapes

**Input snapshot**:
The exact source bytes parsed for an acoustic input, paired with their supplied path and resource role. KRAKEN retains a JSON snapshot or ENV/FLP/TRC/BRC/IRC/SBP snapshots; BELLHOP retains its primary source for existing HDF5 provenance.
_Avoid_: Reconstructed input, reread provenance

**Consumed input**:
A primary or auxiliary path actually read under the selected acoustic options. A merely adjacent same-stem file is not a consumed input; ENV and FLP may own different resource stems.
_Avoid_: All neighboring files

**Case intake**:
A Module's single path from an input source, a file path or a reader, to one loaded value holding its Cases, input snapshots and consumed inputs. It owns routing, reading, parsing and validation; for KRAKEN also the 1 MiB read bound, the FLP and engine defaults and the solver assertion. Adapters choose the source and do not re-sequence those steps; the exception is `pg.bellhop.load` of a JSON file, which keeps its own read and parse so that its diagnostics stay unchanged. Each Module has its own, and no type crosses between them.
_Avoid_: Shared loader, adapter-side read, parse and check sequence

**HDF5 result**:
A complete numerical product in BELLHOP schema v3 or KRAKEN schema v1. Native MOD/SHD/RAY/ARR products use separate pinned reader layouts; Fortran goldens remain fixed comparison evidence, not new Rust outputs.
_Avoid_: Fortran golden, common solver result

**Native result**:
An existing solver result serialized for the pinned v2023.5 numerical readers, with explicit precision, units and order. KRAKEN single-profile blocks with common native headers share one multi-frequency pair; multi-profile or incompatible blocks keep per-block pairs for unmodified reader compatibility. Irregular BELLHOP arrivals are per paired receiver. Neither adapter changes physics or reconstructs inputs.
_Avoid_: Fortran-identical bytes, common solver output model

**Result directory**:
A CLI file group with a bounded ownership manifest, numerical artifacts and a Pelagic run report. Artifacts install only after writing, closing and syncing succeed. Consumed inputs are protected, scratch is exclusively owned, and destinations that appear during execution are refused. Verified owned artifacts are updated by default, stale owned artifacts removed and unrelated files retained. Installation can be rolled back; failed rollback retains named recovery data. This is not an atomic reader snapshot or crash-recovery journal.
_Avoid_: Atomic directory replacement, all neighboring files are ours, best-effort overwrite, early existence check alone

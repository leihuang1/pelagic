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

**Input material**:
A parsed finite medium or half-space, from an ENV or a JSON case, whose absorption still uses the input's selected unit, volume-loss model and per-medium power law. It becomes canonical dB/wavelength only for a selected solve frequency, by the same conversion for both inputs.
_Avoid_: Legacy material, canonical layer containing raw loss, parallel power-law arrays

**Modal refinement**:
The bounded sequence of meshes retaining raw roots for later seeds, a separate Richardson eigenvalue table and first-mesh shapes/group speeds. KRAKEN retains first-mesh loss perturbations; KRAKENC extrapolates complex squared wavenumbers.
_Avoid_: Shared root solver, extrapolated mode shapes

**Input snapshot**:
The exact source bytes parsed for an acoustic input, paired with their supplied path and resource role. KRAKEN retains ENV/FLP/TRC/BRC/IRC/SBP snapshots; BELLHOP retains its primary source for existing HDF5 provenance.
_Avoid_: Reconstructed input, reread provenance

**Consumed input**:
A primary or auxiliary path actually read under the selected acoustic options. A merely adjacent same-stem file is not a consumed input; ENV and FLP may own different resource stems.
_Avoid_: All neighboring files

**HDF5 result**:
A complete numerical product in BELLHOP schema v3 or KRAKEN schema v1. Native MOD/SHD/RAY/ARR products use separate pinned reader layouts; Fortran goldens remain fixed comparison evidence, not new Rust outputs.
_Avoid_: Fortran golden, common solver result

**HDF5 publication**:
Installing a complete local HDF5 result only after writing, closing and syncing succeeds. Consumed inputs are protected, scratch is exclusively owned, and no-overwrite installation must reject destinations that appear during execution.
_Avoid_: Best-effort overwrite, early existence check alone

**Native result**:
An existing solver result serialized for the pinned v2023.5 numerical readers, with explicit precision, units and order. KRAKEN native pairs are per ordered frequency block; irregular BELLHOP arrivals are per paired receiver. Neither adapter changes physics or reconstructs inputs.
_Avoid_: Fortran-identical bytes, common solver output model

**Result directory**:
A CLI file group with a bounded ownership manifest, numerical artifacts and a Pelagic run report. Verified owned artifacts are updated by default, stale owned artifacts removed and unrelated files retained. Installation can be rolled back; failed rollback retains named recovery data. This is not an atomic reader snapshot or crash-recovery journal.
_Avoid_: Atomic directory replacement, all neighboring files are ours

# Pelagic: acoustics inputs and results

Vocabulary for Pelagic's independent BELLHOP and KRAKEN/KRAKENC Modules. Their validated models, numerical Implementations and HDF5 schemas remain separate. See [architecture](docs/development/architecture.md) and the [documentation index](docs/README.md).

The `pelagic` CLI, BELLHOP-only `pelagic-server`, and in-process Python binding
are separate Adapters. Python's immutable result
snapshot can publish native files without HDF5, a CLI process or another solve;
`native-output` shares those writers and protected directory publication.
Language adapters share each Module's JSON v1 contract and Rust validation.
KRAKEN documents keep ordered frequencies and profiles, not modes-only inputs.
See the [bindings contract](docs/guide/bindings.md).

The `pelagic` CLI and BELLHOP-only `pelagic-server` remain independent. CLI calculation publishes native/HDF5/both products in protected result directories; HTTP keeps its existing single-response HDF5/JSON contract.

## Current numerical-validation scope

Full numerical validation uses the existing macOS host and pinned Linux-container
workflows. See [testing policy](docs/development/testing.md#current-numerical-validation-scope).

## Language

**Case**:
An immutable validated acoustic input, constructed from an unvalidated case definition. A KRAKEN case describes one modal environment and its FIELD geometry; a BELLHOP case describes a complete ray/beam calculation.
_Avoid_: Job, generic acoustics configuration

**FIELD case**:
One KRAKEN solve frequency with ordered modal profiles, profile ranges and range-independent, adiabatic or coupled propagation. Frequency order and repetitions belong to the enclosing input sequence.
_Avoid_: Broadband response, time-domain response

**Legacy material**:
A parsed finite medium or half-space whose absorption still uses the ENV's selected unit, volume-loss model and per-medium power law. It becomes canonical dB/wavelength only for a selected solve frequency; JSON already contains canonical values.
_Avoid_: Canonical layer containing raw loss, parallel power-law arrays

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

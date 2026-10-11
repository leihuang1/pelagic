# Documentation

## Use Pelagic

- [CLI: installation, validation, export and computation](guide/cli.md)
- [Native numerical layouts, result directories and safe replacement](guide/native-output.md)
- [BELLHOP HDF5 schema v3](guide/bellhop-hdf5.md)
- [KRAKEN/KRAKENC HDF5 schema v1, quotas and publication](guide/kraken-hdf5.md)
- [BELLHOP HTTP routes, authentication and limits](guide/http.md)

CLI computation defaults to native numerical files; HDF5 and `both` are
explicit choices, using one solve and result-directory publication. HTTP
formats remain unchanged. Fortran reference artifacts are fixed comparison
evidence, not newly generated Rust goldens.

## BELLHOP

- [Supported inputs, physics and limitations](bellhop/compatibility.md)
- [Strict self-contained JSON v1](bellhop/json.md)
- [Intentional numerical compatibility deviations](bellhop/deviations.md)

## KRAKEN/KRAKENC

- [Fixed target, current capabilities and checkpoint evidence](kraken/compatibility.md)
- [Legacy input syntax, precision and budgets](kraken/input.md)
- [Strict self-contained JSON v1](kraken/json.md)
- [Multi-profile adiabatic/coupled FIELD](kraken/field.md)
- [Elastic half-spaces](kraken/elastic-halfspaces.md)
- [Homogeneous and depth-varying finite elastic caps](kraken/finite-elastic-layers.md)

The original 2D target was recorded at `2a7a658:docs/kraken-compatibility.md`.
Directory moves, this refactor and subsequent output adapters do not redefine
that scope, its representative paths or exclusions. Checkpoint sections retain
their historical boundaries; later blocks describe extensions, not permission
to infer an untested option Cartesian product. Budget-rejection checks are
protection evidence, not numerical acceptance.

## Develop and verify

- [Domain vocabulary](../CONTEXT.md)
- [Architecture and module responsibilities](development/architecture.md)
- [KRAKEN implementation and arithmetic locality](development/kraken-internals.md)
- [Pinned oracle, numerical acceptance and evidence](development/reference.md)
- [KRAKEN/KRAKENC checkpoint history and fixed evidence](development/kraken-checkpoints.md)
- [Daily tests, CI and coverage responsibilities](development/testing.md)
- [Full pre-release acceptance and reproduction commands](development/reference-workflows.md)
- [Fortran-to-Rust porting map](development/porting-map.md)
- [Fixed three-layer KRAKENC refinement and historical diagnosis](development/krakenc-three-layer-refinement.md)
- [BELLHOP golden provenance](../crates/bellhop/tests/fixtures/golden/README.md)
- [KRAKEN/KRAKENC golden provenance](../crates/kraken/tests/fixtures/golden/README.md)

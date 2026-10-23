# Documentation

Use this page as the map from the root README into task-specific guides,
schemas and verification notes.

## Use Pelagic

Start here when you are installing Pelagic, running a model, choosing an output
format or integrating the HTTP service.

- [CLI: installation, validation, export and computation](guide/cli.md)
- [Python bindings, machine protocol and release gates](guide/bindings.md)
- [Native numerical layouts, result directories and safe replacement](guide/native-output.md)
- [BELLHOP HDF5 schema v3](guide/bellhop-hdf5.md)
- [KRAKEN/KRAKENC HDF5 schema v1, quotas and publication](guide/kraken-hdf5.md)
- [BELLHOP HTTP routes, authentication and limits](guide/http.md)

CLI computation defaults to native numerical files. HDF5 and `both` are explicit
choices; `both` writes both products from one solve through result-directory
publication. HTTP formats remain unchanged. Fortran reference artifacts are
fixed comparison evidence, not newly generated Rust goldens.

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
- [Rough fluid interfaces and first-mesh scattering](kraken/roughness.md)

The original 2D target was recorded at `2a7a658:docs/kraken-compatibility.md`.
Directory moves and later output adapters do not redefine that
scope, its representative paths or exclusions. Checkpoint sections keep their
historical boundaries; later blocks describe extensions, not permission to infer
an untested option Cartesian product. Budget-rejection checks are protection
evidence, not numerical acceptance.

## Develop and verify

- [Domain vocabulary](../CONTEXT.md)
- [Architecture and module responsibilities](development/architecture.md)
- [KRAKEN implementation and arithmetic locality](development/kraken-internals.md)
- [Pinned oracle, comparison rules and evidence requirements](development/reference.md)
- [Daily tests, CI and release responsibilities](development/testing.md)
- [Full acceptance, business diagnostics and focused reproduction](development/reference-workflows.md)
- [Fortran-to-Rust porting map](development/porting-map.md)
- [Development-tool catalogue](../tools/README.md)

## Numerical results and provenance

- [BELLHOP business coverage and acceptance with exceptions](development/bellhop-business-coverage.md)
- [KRAKEN business coverage, independent controls and remaining limits](development/kraken-business-coverage.md)
- [BELLHOP golden provenance](../crates/bellhop/tests/fixtures/golden/README.md)
- [KRAKEN/KRAKENC golden provenance](../crates/kraken/tests/fixtures/golden/README.md)

Capabilities live in the model guides, comparison rules in `reference.md`,
commands in `reference-workflows.md`, and measured outcomes in the coverage
records. Executable case lists stay in the scripts rather than being copied
across these pages. Local `target/` run paths identify evidence, not files
included in a fresh checkout.

## Historical investigations

These records explain earlier decisions and findings; they are not current
support matrices or active repair plans.

- [KRAKEN/KRAKENC implementation checkpoints](development/kraken-checkpoints.md)
- [KRAKEN input-only capability audit before numerical fixes](development/kraken-capability-audit.md)
- [Fixed three-layer KRAKENC refinement diagnosis](development/krakenc-three-layer-refinement.md)
- [BELLHOP boundary analysis — repair deferred](development/bellhop-boundary-analysis.md)
- [Official 2026 BELLHOP review — pinned oracle unchanged](development/bellhop-official-2026-review.md)

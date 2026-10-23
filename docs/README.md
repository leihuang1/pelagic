# Pelagic documentation

New to Pelagic? Start with [Getting started](getting-started.md).

## Use Pelagic

| I want to… | Read |
| --- | --- |
| Install Pelagic and run a first case | [Getting started](getting-started.md) |
| Use it from Python | [Python guide](guide/python.md) |
| Use it from MATLAB or Octave | [MATLAB/Octave guide](guide/matlab.md) |
| Run it from the command line or a script | [Command-line guide](guide/cli.md) |
| Serve BELLHOP over HTTP | [HTTP service](guide/http.md) |
| Fix an error, or look up a code | [Troubleshooting](guide/troubleshooting.md) |

## Model reference

Check these before starting a study: they list exactly which options are
supported, and the known limits.

| | BELLHOP | KRAKEN/KRAKENC |
| --- | --- | --- |
| Capabilities and limits | [compatibility](bellhop/compatibility.md) | [compatibility](kraken/compatibility.md) |
| JSON case format | [json](bellhop/json.md) | [json](kraken/json.md) |
| Legacy input details | — | [legacy input](kraken/input.md) |
| Known differences from the Toolbox | [deviations](bellhop/deviations.md) | — |

KRAKEN topics: [multi-profile FIELD](kraken/field.md) ·
[elastic half-spaces](kraken/elastic-halfspaces.md) ·
[finite elastic layers](kraken/finite-elastic-layers.md) ·
[rough interfaces](kraken/roughness.md)

## Output formats

- [Native files and result directories](guide/native-output.md): `.ray`,
  `.arr`, `.shd`, `.mod`, the manifest, and the overwrite rules
- [BELLHOP HDF5, schema v3](guide/bellhop-hdf5.md)
- [KRAKEN/KRAKENC HDF5, schema v1](guide/kraken-hdf5.md)

## Maintain Pelagic

Start with [CONTRIBUTING](../CONTRIBUTING.md): setup, checks, conventions and
where each kind of document lives.

| Topic | Page |
| --- | --- |
| Crates and how they depend on each other | [Architecture](development/architecture.md) |
| What is public, version rules, diagnostic codes | [Interface contract](development/interface-contract.md) |
| Daily checks, CI, adapter tests | [Testing](development/testing.md) |
| Cutting a release | [Releasing](development/releasing.md) |
| The pinned Fortran oracle and tolerances | [Numerical reference](development/reference.md) |
| Commands for acceptance and focused comparisons | [Reference workflows](development/reference-workflows.md) |
| Fortran-to-Rust source map | [Porting map](development/porting-map.md) |
| KRAKEN implementation layout | [KRAKEN internals](development/kraken-internals.md) |
| Reference scripts | [Tools catalogue](../tools/README.md) |
| Licence notices and distribution channels | [Packaging](../packaging/README.md) |
| Project vocabulary | [CONTEXT.md](../CONTEXT.md) |

## Numerical evidence

The current measured results, one record per model:

- [BELLHOP business coverage](development/bellhop-business-coverage.md)
- [KRAKEN business coverage](development/kraken-business-coverage.md)
- Golden provenance ledgers: [BELLHOP](../crates/bellhop/tests/fixtures/golden/README.md),
  [KRAKEN/KRAKENC](../crates/kraken/tests/fixtures/golden/README.md)

## History

[Past investigations](history/README.md) explain earlier decisions. They are
kept as written, and are not current support statements.

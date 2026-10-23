# Computing with Pelagic

Install from the repository with `cargo install --path crates/cli`, or replace
`pelagic` below with `cargo run --release -p cli --`. Use `pelagic --help` and
each subcommand's `--help` for the complete argument list.

## BELLHOP

```sh
pelagic bellhop validate path/to/case.env
pelagic bellhop export path/to/case.env > case.json
pelagic bellhop validate case.json
pelagic bellhop run case.json --output results/bellhop
pelagic bellhop run case.json --output results/bellhop --format both
```

`validate` checks inputs without solving. `export` resolves consumed legacy
resources and emits canonical self-contained JSON on stdout; warnings go to
stderr. JSON input never discovers neighboring auxiliary files.

See [input compatibility](../bellhop/compatibility.md),
[JSON schema](../bellhop/json.md) and [HDF5 v3](bellhop-hdf5.md).

## KRAKEN/KRAKENC

```sh
pelagic kraken run path/to/case.env --output results/kraken
pelagic kraken run path/to/case.env --solver krakenc --flp geometry.flp --format both --output results/complex
pelagic kraken export path/to/case.env --solver krakenc > case.json
pelagic kraken validate case.json
pelagic kraken run case.json --format hdf5 --output results/modern
```

Legacy input needs ENV plus FIELD geometry: `--flp` defaults to the same-stem
FLP. ENV owns boundary-table resources; FLP owns the source-pattern stem.
Legacy `--solver` defaults to `kraken`. JSON declares its engine; an explicit
`--solver` must match it, and `--flp` is invalid. Frequencies retain input order
and repetitions; solving/writing stops on the first failure.

See [fixed capabilities and exclusions](../kraken/compatibility.md),
[JSON schema](../kraken/json.md) and [HDF5 v1, quota and publication](kraken-hdf5.md).

## Machine interfaces

`pelagic version --json` reports the implementation, reference, case/HDF5 schema
versions and diagnostics protocol. Global `--diagnostics json` makes stderr NDJSON
records of `{"diagnostics":[...]}`; stdout is unchanged. Codes, severity and field
paths are machine-readable; message wording is not a stable API.

For either model, `validate`, `export` and `run` accept `-` to read self-contained
JSON from stdin (not ENV text). Example:

```sh
pelagic --diagnostics json bellhop run - --format hdf5 --output results/stdin < case.json
```

See the [binding/machine contract](bindings.md) for version policy and adapters.

## Output and failures

`--format legacy|hdf5|both` defaults to `legacy`; `both` never solves twice.
`--output` always names a result directory, default `<case-stem>` in the current
directory; missing parents are created. Existing verified owned products are
updated by default, stale owned products removed, and unrelated files retained.
Unrecognized conflicts, modified products, inputs and aliases are protected.
There is no `--overwrite` flag. See the [native layouts, manifest, quotas and
file-group failure/recovery contract](native-output.md); group installation is
not an atomic snapshot for concurrent readers or automatic crash recovery.
KRAKEN's default cumulative output quota remains 256 MiB, controlled by
`--max-output-bytes`, including all selected products and control metadata.
BELLHOP has no KRAKEN byte quota.

Exit codes are `0` success, `2` input/argument failure, `3` numerical failure,
and `4` output failure. Completion does not certify arbitrary-input oracle
parity. Failed rollback retains recoverable data and the result lock, with
explicit locations in the error; recover before removing those reserved paths.

The HTTP adapter is a separate `pelagic-server` executable; it supports only
BELLHOP and keeps its existing [HTTP protocol](http.md).

# Contributing to Pelagic

Thank you for helping. Bug reports, test cases from real studies, documentation
fixes and code are all welcome.

- **Found a problem?** [Open an issue](https://github.com/leihuang1/pelagic/issues/new/choose).
  [Troubleshooting](docs/guide/troubleshooting.md#reporting-a-problem) lists
  what to include.
- **Planning a larger change?** Open an issue first, so the approach can be
  agreed before you write it.

## Repository map

| Path | Contents |
| --- | --- |
| `crates/bellhop`, `crates/kraken` | The two independent solvers: input parsing, validation, numerics |
| `crates/native-output` | Toolbox-format writers and safe result-directory publication (no HDF5) |
| `crates/output` | HDF5 writers |
| `crates/cli` | The `pelagic` program |
| `crates/server` | The BELLHOP-only `pelagic-server` HTTP service |
| `crates/python`, `python/pelagic` | Python extension (Rust) and package (Python) |
| `matlab/` | MATLAB/Octave toolbox and its tests |
| `examples/` | User-facing cases and scripts, all run in CI ([catalogue](examples/README.md)) |
| `tools/reference/` | Fortran reference image and comparison scripts ([catalogue](tools/README.md)) |
| `crates/*/tests/fixtures/golden/` | Committed Fortran reference results, with provenance ledgers |
| `docs/` | User guides, model references and maintainer docs ([index](docs/README.md)) |
| `packaging/` | Licence notices, conda recipe ([packaging](packaging/README.md)) |

Read [architecture](docs/development/architecture.md) before changing more than
one crate, and [CONTEXT.md](CONTEXT.md) for the project's vocabulary.

## Set up

You need:

- **Rust**, installed with [rustup](https://rustup.rs). `rust-toolchain.toml`
  selects the exact version used by CI (currently 1.99.0). The declared minimum
  for users building from source is 1.88.
- **CMake** and a **C/C++ compiler**. HDF5 is built from source and linked
  statically.

```sh
git clone https://github.com/leihuang1/pelagic.git && cd pelagic
cargo build --release -p cli          # target/release/pelagic
cargo test --locked --workspace --all-features
```

Optional, depending on what you change:

| For | Also install | Then |
| --- | --- | --- |
| Python package | Python 3.9+, `pip install maturin` | `python -m venv .venv && . .venv/bin/activate && maturin develop --release --extras test` |
| MATLAB toolbox | MATLAB R2023a+ or Octave 7+ | See [adapter checks](docs/development/testing.md#language-adapter-checks) |
| Fresh Fortran comparisons | Docker | `tools/reference/build-image.sh`; see [reference workflows](docs/development/reference-workflows.md) |

## Before you open a PR

Run the [daily checks](docs/development/testing.md#daily-development-and-prs).
They take a few minutes and are exactly what CI runs:

```sh
cargo fmt --all -- --check
cargo clippy --locked --workspace --all-targets --all-features -- -D warnings
cargo test --locked --workspace --all-features
python3 tools/reference/verify-goldens.py
python3 tools/reference/test-tools.py
python3 tools/check-docs.py
RUSTDOCFLAGS='-D warnings' cargo doc --locked --workspace --all-features --no-deps
```

If you changed Python or MATLAB code, or anything under `examples/`, also run
the [adapter checks](docs/development/testing.md#language-adapter-checks).
[Adding an example](examples/README.md#adding-an-example) lists the steps for a
new case or script.

## Conventions

**Branches and commits.** Name branches `<type>/<short-topic>`, for example
`fix/kraken-rigid-bottom`. Commit subjects follow
`<type>(<scope>): <imperative summary>`. Types are `feat`, `fix`, `refactor`,
`test`, `docs`, `ci` and `release`; the scope, such as `kraken`, `bellhop`,
`cli`, `python` or `matlab`, is optional. Keep each PR to one concern. Split
large work into a sequence of PRs that each pass CI.

**PRs.** Fill in the template. Describe user-visible changes in
`CHANGELOG.md` under `## Unreleased`. A PR that changes a public interface must
follow the [interface contract](docs/development/interface-contract.md).

**After merge.** Delete the branch.

## Numerical changes

Pelagic's value is that its results match a fixed reference. These rules have
no exceptions:

- **Never regenerate a committed golden to make a test pass.** Goldens come
  from the pinned Fortran build. New ones need provenance in their ledger. See
  [reference](docs/development/reference.md#committed-evidence-and-run-artifacts).
- **Never relax a comparison tolerance, or widen a work limit, to hide a
  difference.** If the reference itself is wrong, keep the failing comparison
  and add an independent control, as described in
  [reference validity](docs/development/reference.md#reference-validity-and-independent-evidence).
- **Reject, don't approximate.** An input option that has not been compared
  against the reference must produce a validation error.
- A change to solver arithmetic needs the
  [full acceptance run](docs/development/reference-workflows.md#full-pre-release-acceptance)
  before release. State in the PR whether you ran it.

## Documentation

Each fact has one home. Link to it rather than copying it:

| Content | Home |
| --- | --- |
| How to install and use Pelagic | `README.md`, `docs/getting-started.md`, `docs/guide/` |
| What each model supports | `docs/bellhop/compatibility.md`, `docs/kraken/compatibility.md` |
| Input and output formats | `docs/{bellhop,kraken}/json.md`, `docs/guide/*-hdf5.md`, `docs/guide/native-output.md` |
| Stability promises, diagnostic codes | `docs/development/interface-contract.md` |
| Comparison rules and tolerances | `docs/development/reference.md` |
| Measured numerical results | `docs/development/*-business-coverage.md` |
| How to run checks and acceptance | `docs/development/testing.md`, `docs/development/reference-workflows.md` |
| How to release | `docs/development/releasing.md` |
| Runnable examples and their cases | `examples/README.md`, `examples/cases/catalog.json` |
| Exact case lists | The scripts in `tools/reference/`, never copied into docs |
| Past investigations | `docs/history/`, which is not updated |

User pages (`README.md`, `docs/getting-started.md`, `docs/guide/`) use plain
language and examples that you have actually run. Maintainer pages state
rules and commands exactly. The README is also the PyPI page, so its links are
absolute `https://github.com/leihuang1/pelagic/blob/main/...` URLs.
`tools/check-docs.py` checks those too.

## Releasing

Maintainers follow the [release checklist](docs/development/releasing.md).

## Licence

By contributing, you agree that your contribution is licensed under
GPL-3.0-or-later, the project's licence.

# Development tools

Executable helpers live in `tools/reference/`:

- `full-acceptance.sh`: complete configured fresh-reference acceptance on a clean
  commit; `--allow-dirty` is diagnostic only.
- `test-one.sh`: require exactly one passing named reference test.
- `verify-goldens.py` / `test-tools.py`: committed-reference integrity and tool guards.
- `build-image.sh`: build the pinned Acoustics Toolbox image.
- `run-case.sh`: run reference BELLHOP.
- `run-kraken-case.sh`: run reference KRAKEN/KRAKENC and FIELD.
- `compare-kraken.sh`: compare API modes and FIELD with fresh reference.
- `compare-kraken-hdf5.sh`: compare API and actual Pelagic CLI/HDF5 products.
- `run-cli-kraken.sh`: one actual CLI `both` solve and complete independent native readback.
- `compare-ray.sh` / `compare-ray-linux.sh`: semantic ray comparison.
- `compare-arrival.sh` / `compare-field.sh`: arrival/pressure comparison.
- `check-critical-rays.sh`: focused boundary/interface comparisons.
- `bellhop-business.py`: broad official/constructed/adjusted BELLHOP differential,
  ENV/JSON and CLI native/HDF5 readback; retains all failures and a per-case report.
- `bellhop-business-linux.sh` / `Dockerfile.business`: same-container Linux x86-64
  Rust 1.99.0 and unchanged Fortran oracle; requires a Linux Rust toolchain directory.
  See [reproduction](../docs/development/reference-workflows.md#broad-bellhop-business-case-diagnostic)
  and [findings](../docs/development/bellhop-business-coverage.md).
- `kraken-business.py`: pinned-Linux official/fixture KRAKEN/KRAKENC diagnostic,
  including script loops/shared FLPs, ENV/JSON APIs, actual CLI native/HDF5 and
  TL/phase metrics. Explicitly retains unsupported inputs and reference failures.
  Run inside `pelagic-business-linux:local`; see [reproduction](../docs/development/reference-workflows.md#broad-kraken-business-case-diagnostic)
  and [findings](../docs/development/kraken-business-coverage.md).
- `prepare-tabref.sh`: triplicate source-built BOUNCE table preparation.
- `probe-layered-refinement.py`: historical diagnostic-only seed perturbations,
  never an acceptance oracle.

See [complete reproduction commands](../docs/development/reference-workflows.md),
[pinned reference and tolerances](../docs/development/reference.md), and the
[documentation index](../docs/README.md). Configured fresh-reference case lists
and exclusions live in [`full-acceptance.sh`](reference/full-acceptance.sh);
[ordinary CI](../.github/workflows/ci.yml) runs the committed-golden suite.

The image defaults to `pelagic-reference:v2023.5-amd64`; set
`BELLHOP_REFERENCE_IMAGE` to an existing pinned image when reproducing locally.
The compiler, source, platform and flags must still match the documented oracle.
The business diagnostic intentionally uses the fixed image tag rather than this override.

# Distribution preparation

The binding workflow produces abi3 wheels, a source archive, matching CLI artifacts
and a MATLAB toolbox. On a published GitHub release, once every test job passes, it
uploads the wheels and sdist to PyPI and attaches `pelagic-vX.Y.Z-<platform>.tar.gz`
(macOS/Linux CLIs) and `pelagic-vX.Y.Z.mltbx`, each with a `.sha256`, to the
release; `release.yml` attaches the Windows ZIP. Both refuse a tag other than
`v<matlab/VERSION>`. It does not register the PyPI project, configure its trusted
publisher, submit a conda-forge feedstock, or create a MATLAB File Exchange listing.
Those are maintainer release actions. Keep the workspace version, `matlab/VERSION`,
`CITATION.cff` and release tag in sync. Run clean-commit numerical acceptance before
release; adapter parity alone is not independent Fortran acceptance.

## Dependency notices

`python/pelagic/THIRD-PARTY-LICENSES.json` is shipped in the Python package.
`cli-licenses.json` accompanies each staged CLI, including the toolbox's copies.
These include transitive Rust, target-specific and build dependency notices; a
license list is not a statement that every listed package is linked at runtime.
The Python dependency graph still excludes HDF5.

Regenerate/check with **cargo-bundle-licenses 4.2.0**, from each crate directory:

```sh
cargo install --locked cargo-bundle-licenses --version 4.2.0
(cd crates/python && cargo bundle-licenses --format json --output ../../target/python-licenses.json \
  --previous ../../python/pelagic/THIRD-PARTY-LICENSES.json --check-previous)
(cd crates/cli && cargo bundle-licenses --format json --output ../../target/cli-licenses.json \
  --previous ../../packaging/cli-licenses.json --check-previous)
```

For an intentional dependency update, omit `--check-previous`, review the generated
texts and warnings, fill missing upstream notices, then replace the committed
snapshot. Do not accept `NOT FOUND`, or assume a generated bundle is a legal audit.
MIT/Apache variant texts retain upstream copyrights and notices. HDF5's vendored C
license is included separately by its source crate.

The four HDF5 wrapper crate archives omit their workspace license files. Their
MIT and Apache-2.0 texts were obtained from `LICENSE-MIT` and `LICENSE-APACHE` at
the **exact upstream revision recorded in each crate's `.cargo_vcs_info.json`**:

| Crate | Version | `https://github.com/metno/hdf5-rust` revision |
|---|---|---|
| hdf5-metno | 0.14.1 | `25418ae50cdfee8463fd83602a448aadbe6e1efe` |
| hdf5-metno-derive | 0.10.2 | `f9041d021afb344ed21fa2129cfe8c06d07605c6` |
| hdf5-metno-sys | 0.12.2 | `10e8bb9df659288fe1f9fb1a2eebf6ce893a25d9` |
| hdf5-metno-types | 0.11.1 | `15683e2dda465c50304b6192acb01a645212548e` |

The `--previous` option retains those verified missing texts during checks. New
crate versions require reviewing their matching upstream revision again.

## conda-forge

`conda/meta.yaml` is a recipe starting point, not an accepted feedstock. Set
`PELAGIC_VERSION` and `PELAGIC_SDIST_SHA256` from the already published source
archive. Test the recipe with the feedstock's compiler/Python matrix, review its
maintainers, and submit through conda-forge's current staged-recipes process.

## MATLAB File Exchange

Stage all five checksum-verified CLI artifacts with `tools/stage-matlab.py`, then
run `matlab/build_toolbox.m` in MATLAB. Review File Exchange's current submission
and license requirements separately. Preserve GPL-3.0-or-later; do not select BSD
or invent a license exception just to satisfy an upload form.

# Releasing

Follow these steps in order. A release is one tag, `vX.Y.Z`, on a merged commit
of `main`. Publishing the GitHub release triggers everything else.

## What a release produces

| Artifact | Built by | Destination |
| --- | --- | --- |
| `pelagic-vX.Y.Z-windows-x64.zip` | `.github/workflows/release.yml` | GitHub release |
| `pelagic-vX.Y.Z-{linux-x64,linux-arm64,macos-arm64,macos-x64}.tar.gz` | `.github/workflows/bindings.yml` (`release-assets`) | GitHub release |
| `pelagic-vX.Y.Z.mltbx` (all five CLIs inside) | `bindings.yml` (`matlab`, `release-assets`) | GitHub release |
| A `.sha256` for each of the above | Same jobs | GitHub release |
| Five abi3 wheels and the sdist | `bindings.yml` (`wheels`, `sdist`, `publish-pypi`) | PyPI |

Both workflows refuse to publish unless the tag equals `v` + `matlab/VERSION`.
`publish-pypi` and `release-assets` run only after every test job passes.

## One-time setup

Already done for this repository. Repeat only if moving it.

- PyPI project `pelagic`, with a trusted publisher for this repository and
  workflow `bindings.yml`, environment `pypi`.
- GitHub environment `pypi`, with a deployment tag rule `v*`.

## 1. Prepare the version bump

On a branch `release/X.Y.Z` from up-to-date `main`, change exactly these:

| File | Change |
| --- | --- |
| `Cargo.toml` | `[workspace.package] version = "X.Y.Z"` |
| `Cargo.lock` | `cargo update --workspace` (changes only the workspace crates) |
| `matlab/VERSION` | `X.Y.Z` |
| `CITATION.cff` | `version: "X.Y.Z"` and `date-released: "YYYY-MM-DD"` (planned release date) |
| `python/pelagic/THIRD-PARTY-LICENSES.json` | `package_version` of the `bellhop`, `kraken` and `native-output` entries |
| `packaging/cli-licenses.json` | `package_version` of the `bellhop`, `kraken`, `native-output` and `output` entries |
| `CHANGELOG.md` | New section for `X.Y.Z`; move items out of "Unreleased" |

The two licence snapshots list the workspace crates by version, so CI's
`--check-previous` fails until they match. Update only those `package_version`
fields. Regenerate the whole snapshot only when dependencies changed; see
[packaging](../../packaging/README.md#dependency-notices).

Check that nothing still carries the old version:

```sh
git grep -n -F 'OLD.VERSION' -- ':!CHANGELOG.md' ':!docs/history'
```

## 2. Check and merge

Run the [daily checks](testing.md#daily-development-and-prs), plus:

```sh
python3 tools/check-docs.py
(cd crates/python && cargo bundle-licenses --format json --output ../../target/python-licenses.json \
  --previous ../../python/pelagic/THIRD-PARTY-LICENSES.json --check-previous)
(cd crates/cli && cargo bundle-licenses --format json --output ../../target/cli-licenses.json \
  --previous ../../packaging/cli-licenses.json --check-previous)
```

Open a PR titled `release: Pelagic X.Y.Z`. Merge it only when all three
workflows (CI, Language bindings, Windows release) are green. Note the merge
commit SHA.

## 3. Full numerical acceptance on the merge commit

In a clean checkout of exactly that commit:

```sh
git switch main && git pull --ff-only && git status --short   # must print nothing
tools/reference/full-acceptance.sh
```

It needs Docker and the pinned image `pelagic-reference:v2023.5-amd64`
(`tools/reference/build-image.sh` builds it). It takes about 8 minutes on an
Apple-silicon Mac. The report is `target/acceptance/<UTC>-<pid>/report.md`.

A release requires **all of**:

- the first line reads `PASS — complete acceptance of the recorded clean commit`;
- the initial and final commit both equal the merge commit;
- the dirty tree is empty at both ends.

A run with `--allow-dirty`, a partial run or a focused retry is not acceptance.
If it fails, fix the problem on a new PR and start again from step 2.

> **macOS host toolchain.** If the HDF5 build fails to link with a `.tbd` or
> `arm64e` error, Xcode's linker is older than the active macOS SDK. Run
> acceptance with `DEVELOPER_DIR=/Library/Developer/CommandLineTools`, or update
> Xcode. Note the toolchain used in the release notes.

## 4. Write the release notes

Use the previous release as a template (`gh release view vA.B.C`). The notes
have these sections:

1. **Highlights**: user-visible changes.
2. **Compatibility**: schema versions (case JSON, HDF5, diagnostics protocol),
   renamed fields or codes, changed defaults, Rust API changes. Say
   "unchanged" explicitly when nothing changed.
3. **Verification and known limits**: merge commit SHA; acceptance result,
   duration and host; test counts; whether the business inventories were re-run
   (see [coverage records](../README.md#numerical-evidence)).
4. **Downloads**: the install commands, and the macOS Gatekeeper note.

## 5. Publish

```sh
gh release create vX.Y.Z --target <merge-sha> --title "Pelagic vX.Y.Z" \
  --notes-file notes.md
```

Publishing starts the `Language bindings` and `Windows release` workflows for
the release. Follow them with `gh run list --event release` and
`gh run watch <id>`.

If a job fails for an infrastructure reason (runner setup, network, a
toolchain install conflict), use `gh run rerun <id> --failed`. If it fails
because of the code, **do not** move the tag:

- **Before PyPI upload** (`publish-pypi` has not run): delete the release
  and tag, fix the problem through a PR, and repeat from step 2 with the same
  version.
- **After PyPI upload:** the version number is used up, because PyPI never
  accepts the same file name twice. Release `X.Y.(Z+1)`.

## 6. Verify as a user

```sh
gh release view vX.Y.Z --json assets -q '.assets[].name'    # 12 files
python -m venv /tmp/pg && /tmp/pg/bin/pip install pelagic==X.Y.Z
/tmp/pg/bin/python -c "import pelagic; print(pelagic.__version__)"
```

- **GitHub release:** 6 archives (5 CLIs and the `.mltbx`) plus 6 `.sha256`
  files.
- **PyPI:** 5 wheels (manylinux x86-64 and aarch64, macOS x86-64 and arm64,
  win_amd64) plus the sdist.
- **One archive:** download it, check it with `shasum -a 256 -c`, then check
  that `pelagic version` reports `X.Y.Z` and that `BUILD-INFO.txt` names the
  merge commit.

## 7. After the release

- Open a PR adding an empty `## Unreleased` section to `CHANGELOG.md`, if the
  release PR did not.
- Optional distribution channels, each a maintainer decision:
  - **conda-forge:** start from `packaging/conda/meta.yaml` with the published
    sdist's SHA-256. See [packaging](../../packaging/README.md#conda-forge).
  - **Zenodo DOI:** enable the GitHub integration before the release you want
    archived.
  - **MATLAB File Exchange:** check its licence terms against GPL-3.0 first.

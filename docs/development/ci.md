# Continuous integration

The two workflows retain separate responsibilities:

- **CI**: formatting, Clippy, the complete Rust workspace test suite (including
  committed numerical references and doctests), reference-tool guards, links,
  and the Rust documentation build.
- **Language bindings**: wheels and matching CLIs, complete Python tests on every
  selected platform, Python 3.9/3.14 compatibility, MATLAB on Linux and Windows,
  Octave, license snapshots, source distribution, and release packaging.
  The same Windows producer also builds the static-CRT CLI distribution,
  dependency and Rust license notices, provenance, and ZIP/checksum. The
  release-only `publish-windows` job attaches that already validated artifact.
  There is no second standalone Windows compilation workflow.

## Cancellation and documentation changes

Only another run for the **same pull request in the same workflow** cancels an
older run. Every non-PR run has a unique concurrency group, including main,
manual, and release runs. Using a branch-wide group with cancellation disabled
is insufficient: GitHub can still replace an older pending run.

Workflow-level `paths`/`paths-ignore` filters are deliberately not used. They can
leave a required check pending indefinitely. Instead, `tools/ci/changes.py`
classifies a narrow set of prose/image paths and skips expensive jobs or steps.
Documentation links and the CI planner tests still run. Changes to examples,
notebooks, scripts, workflows, manifests, license files, or unknown paths always
run full validation. Renames consider both old and new paths. Missing history,
empty diffs, manual runs, and release events fail closed to full validation.

The stable required-check candidates are `build-test`, `bindings-check`, and
`windows-check`. Configure these in repository rules if required; this change
does not change repository protection settings. `bindings-check` verifies every
applicable producer/test job succeeded and rejects missing shard coverage. An
explicit documentation-only skip is allowed; an unexpected skip is a failure.

## Build once, test independently

The composite wheel builder restores the host Rust dependency cache **before**
compilation. Linux manylinux builds additionally use maturin's compiler-cache
support because a host target cache is not a substitute for the container's
build environment. macOS/Windows wheel and dynamic-CRT CLI builds use the same
explicit Rust target to avoid separate target trees. Windows static-CRT output
has a separate target directory and is never substituted for the exact
Python-versus-CLI numerical comparisons.

Linux-x64 consumers no longer wait for unrelated macOS/Windows wheel jobs.
MATLAB installation overlaps producer builds. A bounded, read-only same-run
artifact wait requires successful producers from the exact current attempt:
Windows checks its matching producer; Linux checks every selected platform
before all-platform toolbox packaging. Downloads are pinned by artifact ID,
fail on archive digest mismatch, and retain the existing CLI checksum checks.
Use **Re-run all jobs** for these jobs, not a partial failed-job rerun: an older
attempt cannot satisfy the current producer. Waiting is bounded to 30 minutes;
producer failures fail immediately. This trades some idle runner time (and
wasted installation if a build fails) for a shorter critical path.

Python tests are collected in full. The longer Windows and Intel macOS suites
use two runners with deterministic node-ID hashing; Linux and Apple Silicon
use one runner to avoid duplicate setup on noncritical paths. No test filters
or assertions were removed. The final gate requires the exact planned shard
count and indices for each platform, identical full collections across shards,
and selections forming an exact, disjoint union. Optional
arlpy dependencies are installed on the Linux x64 runner so those tests run
once with the rest of the suite rather than repeating a second invocation.

Platform coverage is unchanged: PRs build Linux x64/ARM64, macOS ARM64, and
Windows x64; main/manual/release runs also build Intel macOS. There are no
macOS formatting/lint/audit jobs. The test phase uses at most three macOS jobs
per workflow; account-wide runner limits still apply across workflows and PRs.

## Cache budget and tools

Use the setup actions' dependency caches, not archives of installed toolchains,
MATLAB, Docker images, or build artifacts. Python caches are keyed by Python and
the project dependency manifest; runtime-only test jobs never restore a Rust
cache. Rust caches omit installed Cargo tools. License verification caches only
registry sources, not target artifacts. These choices are intended to stay
within the repository's 10 GB budget; monitor actual cache usage and eviction
when adding platforms or dependency sets. No paid cache allowance is enabled.

The pinned license tool is downloaded from its upstream release and checked
against its SHA-256 digest, rather than compiled with `cargo install` on CI.
Release-only upload/publish jobs remain guarded by the release event.

## Reproducing measurements

With an authenticated GitHub CLI:

```sh
gh run list -R leihuang1/pelagic --limit 50 \
  --json databaseId,workflowName,headSha,headBranch,event,status,conclusion,createdAt,startedAt,updatedAt,url
gh api --paginate 'repos/leihuang1/pelagic/actions/runs/RUN_ID/jobs?per_page=100&filter=all'
gh run view -R leihuang1/pelagic RUN_ID --log
```

Use job `created_at` to `started_at` for runner queue time. Time before a job is
created includes dependency waiting, and must not be presented as runner queue
time. Step timestamps distinguish setup/build, test/validation, and cleanup,
but a combined `cargo test` or install-and-test step needs logs to separate its
parts. Report mixed/unknown time explicitly rather than inventing precision.
Compare the same event/platform shape and separate cold from warm caches.

Local planner regression tests and workflow syntax checks:

```sh
python3 tools/ci/test-ci.py
actionlint
```

To diagnose incremental invalidation, run the exact same Cargo build twice with
an unchanged target directory and environment, then inspect `cargo build -vv`.
Do not extrapolate a local no-op build to a fresh GitHub runner: dependency-only
caches intentionally do not retain every workspace output.

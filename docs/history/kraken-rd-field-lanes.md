# Range-dependent KRAKEN FIELD lanes

Measured 9 November 2026 against `7246c5a`, with Rust 1.99.0/LLVM 23.1.1,
on an Apple M4 Pro (14 CPUs, macOS arm64) and native AMD EPYC 7B13
(12 cores/24 logical CPUs, Linux, AVX2/FMA but no AVX-512).

**Status: coupled-only candidate, not performance-approved.** The Gulf coupled
gain is real in this experiment, and checked results are bit-for-bit unchanged.
However, small off-path v3 adverse comparisons reproduce. This change has not
passed the required no-regression gate and is not ready to merge. Adiabatic
lanes were measured but omitted.

## Change and scope

The implementation stays in `crates/kraken/src/solver/field.rs`. The coupled
march and projections remain sequential. Receiver rows share shape sampling by
profile and add each range's modes in the original order, using the original
f32 complex product/sum and f32/f64 spreading/conversion points. No fast-math,
FMA contraction, unsafe code, dependency, public selector or numerical-method
change was introduced.

`RangeValues` holds either a range-major scalar amplitude vector or mode-major
real/imaginary arrays using the existing 16-wide `RangePhaseLanes`. Amplitudes
are written **directly** into that representation during the march: unlike the
research patch, there is no full scalar block plus a duplicated transpose.

Validated `Case`/`FieldCase` values already guarantee finite, strictly increasing
receiver/profile ranges. `range_blocks` splits at profile midpoints; an exact
boundary still belongs to the left profile. A block may contain several profiles. Its range-slot capacity
is `max(1, min(table_values, PHASE_TABLE_VALUES) / max(largest_mode_count, 1))`.
A lane group spends 16 slots including padding; a scalar group spends its live
length. The default `TABLE_VALUES` is 65,536; finished receiver rows retain their
separate `fill_rows` budget. The existing one-scalar-range fallback remains for
an artificial test budget smaller than one mode vector. These are per-table
bounds, not a total-RSS guarantee.

Keep `MIN_LANE_RANGES = 8`: groups/tails shorter than eight, or groups without
room for 16 charged slots, stay scalar. The retained code reuses the existing
range-independent width/policy and lane type; it does not change that path.

The [implementation map](../development/kraken-internals.md#field-storage-and-range-lanes)
states the storage and precision invariants. This base does not contain the
later shooting/root-search optimization; numbers from that study are not
measurements of this change or of a stacked combination.

## Measurements

`tools/perf/run.py bench`, unchanged timing/check algorithms, one solver thread,
one warmup and three timed repetitions per process. The primary uses 11 seeded
randomized/interleaved blocks (schedule seed 20261031). Each target has a
byte-identical baseline **copy**, not a separately rebuilt control. Native cases
are pinned to logical CPU 0, with load checks and no concurrent builds.

Ratios are medians of paired process-block ratios, using each process's median
of three timings. Intervals are pointwise 95% block-bootstrap intervals, 4,000
resamples, seed 8675309. Pooled median milliseconds need not divide to the paired
ratio. Repetitions inside a process are not independent bootstrap units. The
separate rerun has 33 blocks, seed 20261110, and is **not pooled** with the primary.

A targeted gain requires the lower bound to exceed 1 against **both** baseline
copies; an upper bound below 1 against **either** flags an adverse comparison.
All eleven cases are screened at both native targets. This empirical screen is
not simultaneous non-inferiority proof. Only the coupled Gulf case is targeted
by the retained code; incidental off-path gains are not algorithmic claims.

These are **solver-only**, including modal solving and FIELD, not parsing,
serialization, CLI end-to-end time or a Fortran comparison. Native default has
no CPU override; v3 uses `-C target-cpu=x86-64-v3`, without fast-math. All eleven
cases were measured together, with both candidates and each target's control.

| Case | M4 speedup | EPYC default, 95% CI | EPYC v3, 95% CI |
|---|---:|---|---|
| broadband | 0.998× | 0.9983 [0.9952, 1.0032] | 0.9984 [0.9923, 1.0017] |
| elastic-sediment | 1.002× | 0.9985 [0.9969, 1.0010] | 0.9986 [0.9825, 1.0077] |
| gulf-adiabatic | 1.015× | 0.9976 [0.9938, 1.0089] | 0.9997 [0.9910, 1.0072] |
| gulf-coupled | 1.122× | 1.1813 [1.1754, 1.1908] | 1.1957 [1.1855, 1.2078] |
| kr-munk-big | 1.003× | 1.0015 [0.9954, 1.0079] | 0.9972 [0.9928, 1.0011] |
| krakenc-pekeris-rigid-loss | 1.003× | 0.9985 [0.9926, 1.0009] | 0.9974 [0.9936, 0.9990] |
| layered-seabed | 0.998× | 1.0052 [0.9711, 1.0116] | 1.0057 [0.9993, 1.0168] |
| munk-lossless | 0.998× | 1.0066 [0.9918, 1.0345] | 1.0007 [0.9971, 1.0108] |
| pekeris | 1.007× | 0.9993 [0.9938, 1.0091] | 0.9961 [0.9902, 1.0055] |
| sduct-trapped | 1.005× | 1.0005 [0.9986, 1.0039] | 1.0039 [1.0005, 1.0072] |
| surface-duct | 1.012× | 1.0002 [0.9974, 1.0050] | 1.0009 [0.9959, 1.0082] |

For Gulf coupled, the primary default pooled median changes
204.332 →
173.039 ms; v3 changes
138.103 →
115.382 ms. Primary paired
control/candidate ratios are
1.1795 [1.1718, 1.1845]× default and
1.1997 [1.1880, 1.2156]× v3. Independent
33-round reruns retain the Gulf gains, but do not clear the whole screen.

### Adverse comparisons: keep primary and rerun separate

| Case / target | Experiment | Base/coupled, 95% CI | Control/coupled, 95% CI |
|---|---|---|---|
| kr-munk-big / v3 | primary | 0.9972 [0.9928, 1.0011] | 0.9949 [0.9900, 0.9993] |
| kr-munk-big / v3 | rerun | 0.9946 [0.9925, 0.9965] | 0.9950 [0.9942, 0.9970] |
| krakenc-pekeris-rigid-loss / v3 | primary | 0.9974 [0.9936, 0.9990] | 0.9988 [0.9928, 1.0050] |
| krakenc-pekeris-rigid-loss / v3 | rerun | 0.9995 [0.9971, 1.0000] | 0.9981 [0.9960, 0.9999] |
| gulf-adiabatic / v3 | primary | 0.9997 [0.9910, 1.0072] | 1.0037 [0.9958, 1.0098] |
| gulf-adiabatic / v3 | rerun | 0.9963 [0.9936, 1.0012] | 0.9956 [0.9948, 0.9979] |
| munk-lossless / M4 | primary | 0.9983 [0.9852, 1.0181] | 0.9906 [0.9791, 0.9984] |
| munk-lossless / M4 | rerun | 0.9984 [0.9903, 1.0057] | 0.9954 [0.9888, 1.0014] |

Large Munk and this one-profile KRAKENC input take the range-independent path;
the retained change also leaves adiabatic arithmetic untouched. The actual
compiled solver-source audit finds only `field.rs` changed. These small signals
cannot therefore be attributed to added work in those algorithms. Codegen,
layout and VM variation are possible explanations, **not established causes**.
In particular, the repeated large-Munk deficit is not dismissed as noise.
No combined average is used to erase any primary or rerun witness.

### Threshold and omitted adiabatic prototype

Forced scalar-versus-lane row timings used 11 alternating blocks, ten warmups
and 1,000 row calls per block, with bitwise assertions before timing. The kernel
experiment excludes table preparation and is not a whole-solver measurement.
For 64 modes, coupled eight-range speedups are 1.597× M4, 2.036× EPYC default and
2.547× v3. Four ranges are only break-even on M4; a single padded range loses
badly. Two-mode eight-range rows also improve. The existing eight-range
threshold is therefore a conservative cross-platform choice, not a new tuning
option. Scalar tails avoid padding a lone seventeenth range into another full
lane group, and all padding is charged against the table.

The 16-lane adiabatic prototype needed a threshold of 12 on M4 and explicit
preservation of the empty incoherent sum's −0.0. Its identity checks passed,
but the estimated 1.21–1.31× Gulf adiabatic gain did not survive:

| Platform | Primary Gulf-ad base/both, 95% CI | Separate 33-round rerun |
|---|---|---|
| M4 | 1.1063 [1.1016, 1.1182] | 1.1003 [1.0985, 1.1052] |
| EPYC default | 1.0778 [1.0739, 1.0838] | 1.0751 [1.0738, 1.0780] |
| EPYC v3 | 1.1932 [1.1708, 1.1977] | 1.1876 [1.1853, 1.1931] |

The combined prototype also has additional off-path performance deficits.
Its f64 factor layout and row kernel were removed rather than bundled into the
coupled change. Other adiabatic lane widths were not investigated.

## Verification and acceptance

- `cargo test -p kraken --lib`: **67 passed** for coupled-only.
- `cargo test --workspace --all-features`: **391 passed, 12 ignored**, across
  54 reported suites, for both the combined prototype and coupled-only. The
  ignored tests were not claimed as passes. Existing FIELD parallelism,
  integration, CLI/output and doc tests are included in the workspace run.
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`:
  **passed** for both candidates. `DEVELOPER_DIR=/Library/Developer/CommandLineTools`
  and the rdfield worktree's own `target/` were used.
- New scalar/lane tests exercise **1–17 ranges per same-profile segment**,
  threshold boundaries, range zero, four profiles and several segments per
  block, point/line/scaled sources, zero/one/fixture mode counts, coherent and
  incoherent adiabatic addition, and tiny/boundary/large table budgets. The
  planner test asserts charged padding, contiguous coverage, midpoint ownership
  and scalar fallback. Comparisons use `to_bits`, not float equality.
- Extra macOS full-result checks: 11 timing inputs × baseline/coupled/both ×
  threads 1/4 = **66 successful, byte-identical snapshots**, zero mismatches.
- Native checks: 159 inputs (148 ENV/FLP workflows + 11 timing inputs) ×
  baseline/coupled/both × default/v3 × threads 1/4 = **1,908 configurations**.
  Each 318-configuration check has 274 successful snapshots, 42 unchanged
  expected load errors and two unchanged expected solve errors, with **zero
  mismatches or unexpected outcomes**. Cross-target and thread comparisons use
  default baseline/thread 1. Totals: **1,644 matching full-result hashes and 264
  unchanged diagnostic outcomes**, plus 1,908 locally verified diagnostic
  sidecars. The baseline/final-only subset is 1,272 configurations, 1,096
  successful snapshots and 176 expected diagnostic outcomes. Full-result
  identity includes ordered mode counts, modes and pressures; errors are not
  counted as numerical successes.
- Full native gzip snapshots total **895,899,180 bytes**, above the ~200 MB
  download threshold. They were read/hashed and compared on the VM. Every
  configuration's full-result/gzip hashes and outcome/diagnostic evidence was
  secured locally; the full native gzip files were not downloaded. Mac full
  snapshots are retained locally.
- Final macOS `DEVELOPER_DIR=/Library/Developer/CommandLineTools
  tools/reference/full-acceptance.sh` exited **2**: “Release acceptance needs a
  clean commit. --allow-dirty runs diagnostics, never release acceptance.”
  No Fortran acceptance stage ran, no tolerance/reference changed, and no
  `--allow-dirty` substitute was used. Claude still needs clean-commit acceptance.

## Evidence and resources

Native primary: 968 process records, 2,904 timed solves; separate rerun:
1,848 records, 5,544 timed solves. No timing row failed or was discarded.
Each driver's essential evidence was downloaded and SHA-verified before the
next driver. Twelve checkpoint archives and eleven completed phase boundaries
were independently audited, along with eight manifests and six solver builds.

Only the owned `pelagic-rdfield` session was used. It was stopped after a
conservative request-to-release interval of 34 min 30 s, inside the 45-minute
cap. The final account listing showed no active assignments. Observed account
debit was 1.73 compute units; the rate-duration estimate was 1.776 units, a
separate accounting view. The watchdog and controller exited. No replacement
allocation was made.

Raw timings, full local Mac snapshots, native per-configuration snapshot and
diagnostic hashes, source/binary hashes, controls, resource records and the
complete tables remain local in `pelagic-wt-kprof-scratch/rdfield/`.
`pelagic-wt-kprof-scratch/RDFIELD-REPORT.md` is the detailed handoff.
The final `field.rs` SHA-256 is
`2f922a8d25d7f60c82fff129e5579c65bddd00bd7a64bba86e5e639e3dd22608`;
the native identity aggregate is
`a92e407372eccca35b695aafe3f31b533f1dd08c662f999a3da52ecee1c76bf7`.

The persistent v3 performance cost and clean-commit acceptance remain open.
No end-to-end improvement, other-microarchitecture guarantee, or completed
Fortran release acceptance is claimed.

# Range-dependent KRAKEN FIELD lanes

Measured 9 November 2026 with Rust 1.99.0 on a native AMD EPYC 7B13 (12
cores/24 logical CPUs, Linux, AVX2/FMA but no AVX-512). The base is `57f55a3`,
which adds pull requests #82 to #86 to `7246c5a`: among them the faster fluid
root search and shooting kernel. The candidate is `0d294b7`. An
[earlier experiment](#earlier-experiment-on-7246c5a) measured the same change
against `7246c5a`, before the root-search and shooting changes; its numbers are
not measurements of the merged code.

**Status: retained.** Solver-only runs of the Gulf coupled example are 1.38
times faster with the default build and 1.30 times faster with the x86-64-v3
build. No
other example has a slowdown that a rerun reproduces. Full results are
bit-for-bit unchanged. Adiabatic lanes were tried in the earlier experiment and
not retained.

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
boundary still belongs to the left profile. A block may contain several
profiles. Its range-slot capacity is
`max(1, min(table_values, PHASE_TABLE_VALUES) / max(largest_mode_count, 1))`.
A lane group spends 16 slots including padding; a scalar group spends its live
length. The default `TABLE_VALUES` is 65,536; finished receiver rows retain their
separate `fill_rows` budget. The existing one-scalar-range fallback remains for
an artificial test budget smaller than one mode vector. These are per-table
bounds, not a total-RSS guarantee.

Keep `MIN_LANE_RANGES = 8`: groups/tails shorter than eight, or groups without
room for 16 charged slots, stay scalar. The retained code reuses the existing
range-independent width/policy and lane type; it does not change that path.
The [implementation map](../development/kraken-internals.md#field-storage-and-range-lanes)
states the storage and precision invariants.

## Decision rule

The change was retained because, against `57f55a3`, it met all three of these:

1. Gulf coupled is faster than both baseline copies at both EPYC targets.
2. Each of the other ten examples has no adverse comparison that a 33-round
   rerun reproduces, or runs the same machine code in base and candidate.
3. Full results are bit-for-bit identical for 159 inputs, at one and four
   threads, with both builds.

## Method

The `tools/perf` timing and check algorithms, one solver thread, one warm-up
and three timed repetitions per process, pinned to one logical CPU. Both runs
started with the one-minute load average below 1. Each build has a
byte-identical copy of its base binary, so base against copy shows what
identical code measures. The primary
run interleaves base, copy and candidate in 11 seeded random blocks (seed
20261111); a separate 33-block rerun (seed 20261112) repeats Gulf coupled and
every example with an adverse comparison. The two runs are not pooled. The
default build has no CPU flags; v3 uses `-C target-cpu=x86-64-v3`.

Ratios are base (or copy) time over candidate time, so above 1 means the
candidate is faster. Each is the median of paired per-block ratios of each
process's median of three timings, with a 95% block-bootstrap interval (4,000
resamples, seed 8675309). A gain needs the lower bound above 1 against both
copies; an upper bound below 1 against either is an adverse comparison. These
are solver-only times, including modal solving and FIELD, not parsing, output
or end-to-end CLI time.

## Results on `57f55a3`

Primary run, 11 blocks:

| Case | Default, vs base | Default, vs copy | v3, vs base | v3, vs copy |
|---|---|---|---|---|
| broadband | 1.0057 [0.9828, 1.0155] | 1.0039 [0.9984, 1.0139] | 0.9944 [0.9495, 1.0315] | 1.0301 [1.0012, 1.0638] |
| elastic-sediment | 0.9996 [0.9832, 1.0346] | 0.9993 [0.9967, 1.0204] | 1.0024 [0.9759, 1.0156] | 1.0015 [0.9500, 1.0462] |
| gulf-adiabatic | 1.0133 [0.9874, 1.0230] | 1.0039 [0.9922, 1.0121] | 1.0091 [0.9976, 1.0257] | 1.0032 [0.9850, 1.0229] |
| **gulf-coupled** | **1.3848 [1.3668, 1.4111]** | **1.3815 [1.3714, 1.4047]** | **1.2994 [1.2680, 1.3211]** | **1.2800 [1.2641, 1.3264]** |
| kr-munk-big | 0.9995 [0.9782, 1.0119] | 1.0031 [1.0001, 1.0141] | 1.0100 [0.9992, 1.0392] | 0.9968 [0.9929, 1.0149] |
| krakenc-pekeris-rigid-loss | 0.9976 [0.9508, 1.0143] | 0.9973 [0.9819, 1.0097] | 0.9990 [0.9955, 1.0004] | 0.9991 [0.9943, 1.0227] |
| layered-seabed | 1.0106 [0.9988, 1.0194] | 1.0170 [0.9438, 1.0524] | 1.0181 [0.9926, 1.0378] | 0.9980 [0.9549, 1.0315] |
| munk-lossless | 1.0038 [0.9984, 1.0151] | 1.0073 [0.9992, 1.0308] | 0.9960 [0.9826, 1.0147] | 1.0011 [0.9932, 1.0104] |
| pekeris | 0.9904 [0.9781, 1.0088] | 0.9910 [0.9741, 1.0169] | 0.9813 [0.9459, 1.0283] | 0.9806 [0.9555, 0.9992] |
| sduct-trapped | 1.0068 [0.9965, 1.0077] | 0.9983 [0.9831, 1.0108] | 0.9965 [0.9608, 1.0175] | 1.0004 [0.9923, 1.0282] |
| surface-duct | 1.0066 [1.0005, 1.0234] | 1.0027 [0.9924, 1.0131] | 0.9823 [0.9547, 1.0187] | 0.9895 [0.9727, 1.0183] |

Gulf coupled's pooled median falls from 105.671 to 76.121 ms with the default
build and from 86.646 to 66.441 ms with v3. That saves 29.6 and 20.2 ms, close
to the 31.3 and 22.7 ms the same change saved on `7246c5a`. The ratio is higher
now because the root-search and shooting changes made the rest of the solve
faster, so FIELD is a larger share of it.

The only adverse comparison is Pekeris with v3 against the copy. The 33-block
rerun did not reproduce it:

| Case / build | vs base | vs copy | Base vs copy |
|---|---|---|---|
| gulf-coupled / default | 1.3807 [1.3762, 1.3984] | 1.3797 [1.3712, 1.3874] | 1.0008 [0.9953, 1.0114] |
| gulf-coupled / v3 | 1.2994 [1.2875, 1.3124] | 1.3013 [1.2956, 1.3151] | 0.9974 [0.9899, 1.0047] |
| pekeris / default | 0.9989 [0.9879, 1.0077] | 0.9939 [0.9858, 1.0031] | 1.0029 [0.9981, 1.0080] |
| pekeris / v3 | 0.9855 [0.9632, 1.0059] | 0.9723 [0.9607, 1.0040] | 1.0150 [0.9967, 1.0489] |

Pekeris takes about 0.45 ms, and its v3 point estimates stay 1.5 to 2.8% below
1. The two identical copies differ by 1.5% in the same rerun, and in the
primary run the identical v3 broadband copies differ by 0.9871 [0.9245,
0.9973], an adverse comparison between identical binaries. The two examples flagged on
`7246c5a`, large Munk and the KRAKENC input, have no adverse comparison here.

### Machine code

Each timed binary was disassembled with `objdump`, and the base and candidate
were compared function by function, ignoring addresses. With the default build
3,198 of 3,285 base functions match; with v3, 3,190 of 3,271 match. The rest
are the changed coupled FIELD code and functions in unrelated modules, such as
`FieldCase::new` and layer construction.

Every hot function of large Munk and KRAKENC is identical with both builds:
fluid root search, shooting, Sturm counts, mode shapes, and KRAKENC's secant
search and mode functions. One hot function differs: `tabulate_field_query`,
range-independent FIELD, has different register assignments and stack
offsets, and with the default build a 16-byte larger stack frame. Its source
is unchanged, it calls the same functions, and its mode and range loops, which
compile as separate functions, are identical.

Cross-compiling the `kraken` library for `x86_64-unknown-linux-gnu` reproduces
this. The release profile splits the crate into 16 codegen units, and the new
coupled code moves `tabulate_field_query` from unit 12 to unit 05. With
`codegen-units = 1` it compiles identically in base and candidate, with both
builds. The difference comes from how the compiler splits the crate, not from
anything this change does on the range-independent path.

### Full-result identity

159 inputs (148 ENV/FLP workflows and the 11 timing inputs), at threads 1 and 4,
with both builds, give 636 base and 636 candidate results. All 636 pairs match:
548 complete results, including ordered mode counts, modes and pressures, and
88 identical expected errors (84 load errors and 4 solve errors, the same as on
`7246c5a`). Every result also matches the default base at one thread, so they
are identical across builds and thread counts.

### Tests

On `0d294b7`, `cargo test -p kraken` passed 219 tests with 6 ignored, and
workspace clippy and rustfmt were clean.

### Resources

One Colab TPU v5e1 runtime (EPYC 7B13), session `pelagic-rdfield-2`: 29.6
minutes from request to release, under a 45-minute cap, and 1.47 compute units.
No assignment was active afterwards. The raw timings, disassembly, snapshot
hashes and session records are kept outside the repository, in
`pelagic-wt-kprof-scratch/rdfield-2/`.

## Earlier experiment on `7246c5a`

Measured 8–9 November 2026 against `7246c5a`, on an Apple M4 Pro (14 CPUs,
macOS arm64) and the same EPYC 7B13. The method matches the one above, with
schedule seed 20261031 and rerun seed 20261110. This experiment did not pass:
small off-path v3 adverse comparisons reproduced, and it could not show why.

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

Gulf coupled's default pooled median fell from 204.332 to 173.039 ms, and v3's
from 138.103 to 115.382 ms. Against the copy, the gains were 1.1795 [1.1718,
1.1845] with the default build and 1.1997 [1.1880, 1.2156] with v3.

Adverse comparisons, primary and rerun kept separate:

| Case / target | Experiment | Base/coupled, 95% CI | Copy/coupled, 95% CI |
|---|---|---|---|
| kr-munk-big / v3 | primary | 0.9972 [0.9928, 1.0011] | 0.9949 [0.9900, 0.9993] |
| kr-munk-big / v3 | rerun | 0.9946 [0.9925, 0.9965] | 0.9950 [0.9942, 0.9970] |
| krakenc-pekeris-rigid-loss / v3 | primary | 0.9974 [0.9936, 0.9990] | 0.9988 [0.9928, 1.0050] |
| krakenc-pekeris-rigid-loss / v3 | rerun | 0.9995 [0.9971, 1.0000] | 0.9981 [0.9960, 0.9999] |
| gulf-adiabatic / v3 | primary | 0.9997 [0.9910, 1.0072] | 1.0037 [0.9958, 1.0098] |
| gulf-adiabatic / v3 | rerun | 0.9963 [0.9936, 1.0012] | 0.9956 [0.9948, 0.9979] |
| munk-lossless / M4 | primary | 0.9983 [0.9852, 1.0181] | 0.9906 [0.9791, 0.9984] |
| munk-lossless / M4 | rerun | 0.9984 [0.9903, 1.0057] | 0.9954 [0.9888, 1.0014] |

Large Munk and this one-profile KRAKENC input take the range-independent path,
whose source the change leaves alone. This experiment could not tell a
code-generation or layout effect from added work, so it was not approved. On `57f55a3` neither example has an adverse comparison, and their
hot functions compile identically.

### Threshold and adiabatic prototype

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
but its estimated 1.21–1.31× Gulf adiabatic gain was not borne out:

| Platform | Primary Gulf-ad base/both, 95% CI | Separate 33-round rerun |
|---|---|---|
| M4 | 1.1063 [1.1016, 1.1182] | 1.1003 [1.0985, 1.1052] |
| EPYC default | 1.0778 [1.0739, 1.0838] | 1.0751 [1.0738, 1.0780] |
| EPYC v3 | 1.1932 [1.1708, 1.1977] | 1.1876 [1.1853, 1.1931] |

The combined prototype also had additional off-path performance deficits.
Its f64 factor layout and row kernel were removed rather than bundled into the
coupled change. Other adiabatic lane widths were not investigated.

### Verification

- `cargo test --workspace --all-features`: 391 passed, 12 ignored, for both the
  combined prototype and coupled-only; `cargo clippy --workspace --all-targets
  --all-features -- -D warnings` passed for both.
- New scalar/lane tests exercise 1–17 ranges per same-profile segment,
  threshold boundaries, range zero, four profiles and several segments per
  block, point/line/scaled sources, zero/one/fixture mode counts, coherent and
  incoherent adiabatic addition, and tiny/boundary/large table budgets. The
  planner test asserts charged padding, contiguous coverage, midpoint ownership
  and scalar fallback. Comparisons use `to_bits`, not float equality.
- 11 timing inputs × baseline/coupled/both × threads 1/4 gave 66 byte-identical
  macOS snapshots. On the EPYC, 159 inputs × baseline/coupled/both × default/v3
  × threads 1/4 gave 1,908 configurations with 1,644 matching full-result hashes,
  264 unchanged expected errors, and no mismatches.
- One Colab session, `pelagic-rdfield`: 34 min 30 s from request to release and
  1.73 compute units. Its records are kept outside the repository, in
  `pelagic-wt-kprof-scratch/rdfield/`.

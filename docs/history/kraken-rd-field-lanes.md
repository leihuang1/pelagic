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
bit-for-bit unchanged. Adiabatic lanes were not retained in that coupled change;
a new eight-wide implementation is described next. The EPYC measurements below
remain measurements of coupled lanes, not of the new adiabatic kernel.

## Adiabatic lanes on `14fd4b1`

The new adiabatic kernel builds on the mode-shape scratch reuse and direct
one-thread receiver-row path in `14fd4b1`. It reuses `RangePhaseLanes`,
`RangeValues` and the range planner, with f64 factors and eight lanes instead of
the coupled kernel's f32 factors and sixteen lanes. Only full eight-range groups
use lanes; partial groups remain scalar. The coupled and range-independent lane
widths and thresholds are unchanged.

The phase march and its range-factor formulas are unchanged. Adiabatic groups
split at profile endpoints, retaining exact endpoints on the left, rather than
at coupled midpoints. Factors are produced directly in the scalar/lane layout,
not copied from a second factor table. Each table retains its existing bound,
including charged lane slots and the one-scalar-range fallback for a tiny test
budget. Integral snapshots are dropped before receiver rows. The one-thread row
iterator returns stack-buffered pressures directly to the result; it adds no
heap-allocated pressure row. The existing profile-sampling vectors remain.

Every lane retains the mode order, `double(single(base + w * slope))`, complex
product and final f32/f64 conversion. Incoherent norm-squared sums start at -0.0,
not the coherent sum's +0.0. No fast-math, FMA contraction, unsafe code,
dependency or public numerical selector was added.

### Width and threshold

Cross-compiling the actual `kraken` library with Rust 1.99.0 / LLVM 23.1.1 for
`x86_64-unknown-linux-gnu`, using `-C target-cpu=x86-64-v3`, gives four doubles
per AVX2 register. The eight-wide coherent loop keeps four accumulator vectors
in registers; neither addition mode references the stack in its modal loop.
The sixteen-wide coherent loop spills three accumulator vectors. Both retain
separate multiply/add instructions and f64-to-f32-to-f64 rounding.

**Indicative, macOS, shared machine:** current direct-row microbenchmarks on an
Apple M4 Pro used 11 alternating blocks, 2,000 row calls per block after ten
warmups, and bitwise assertions before timing. They include shape sampling and
iterator consumption, but exclude table construction; these are not solver
speedups. All lengths 1–17 were measured with 2, 64 and 128 modes and both
addition modes. For 64/128 modes, eight-wide groups with four live ranges lose
(0.79–0.86× scalar/lane), six are borderline (0.98–1.05×), seven gain only
1.07–1.13×, and eight gain 1.14–1.24×. Sixteen-wide groups still lose with eight
live ranges (0.75–0.81×); sixteen live ranges perform about like two eight-wide
groups. Hence full eight-wide groups, not the earlier prototype's width 16 and
threshold 12. Forced lanes with only two modes lose in this microbenchmark;
this is not a universal speedup claim for tiny fields.

### Solver measurements and identity

**Indicative, macOS, shared machine:** 11 seeded, interleaved rounds of
`tools/perf/run.py bench`, all 12 `tools/perf/cases.json` inputs, one solver
thread, one warmup and three timed repetitions per process. The third binary
is a byte-identical copy of the archived `14fd4b1` base. Ratios are medians of
paired process-block ratios, with pointwise 95% block-bootstrap intervals
(4,000 resamples). Gulf adiabatic improves 1.1541 [1.1509, 1.1798]× against the
base and 1.1700 [1.1496, 1.1796]× against the copy. The identical bases compare
at 1.0047 [0.9903, 1.0086]×. Pooled median times are 58.999 and 50.323 ms.
These are solver-only measurements, not CLI times or an x86 speedup claim.

**Indicative, macOS, shared machine:** a separate 33-round rerun of both Gulf
cases and the three initially adverse cases confirms Gulf adiabatic at
1.1689 [1.1670, 1.1702]× against the base and 1.1665 [1.1631, 1.1680]× against
the copy. Broadband and Sduct's initial adverse comparisons do not reproduce;
Gulf coupled is unchanged within the intervals. BELLHOP Munk retains a small
adverse comparison against the base, 0.9986 [0.9979, 0.9994]×, but not the copy,
0.9998 [0.9986, 1.0001]×. Its identical-base control is
0.9994 [0.9981, 1.0001]×, so this 0.14% point estimate is within the observed
A/A spread; the adverse interval is nevertheless retained, not erased by
pooling runs. All 28 disassembled BELLHOP solver functions have identical
instructions after normalizing addresses and constant references, with external
call targets resolved by symbol. That does not compare constant data or rule
out layout effects. The completed native x86 CLI measurement is recorded below.

The test surface now compares the new row directly against the base scalar row
with 1–17 ranges per segment, several profiles per block, complex shapes,
0/1/64 modes, both addition modes and direct/parallel row filling. Existing
whole-field scalar/lane checks cover point, line and scaled sources and tiny,
threshold and large table budgets. Planner checks pin contiguous coverage,
charged slots and exact profile ownership. All comparisons use floating-point
bits, including empty incoherent sums.

Full-result checks cover all 12 performance inputs and all seven multi-profile
fixture/example ENV/FLP workflows under both KRAKEN and KRAKENC: 26 inputs in
all. The candidate at threads 1, 2 and 4 matches both the archived `14fd4b1`
base and the supplied `737ca20` binary: 78/78 complete results against each,
zero mismatches or unexpected outcomes. Legacy CLI checks on all seven ENV/FLP
workflows, both engines and threads 1/4 also match every output file: 28
base/candidate pairs, 224 file hashes including manifests and run reports.

The macOS implementation and measurement evidence against `14fd4b1` are kept
outside the repository in `pelagic-wt-adiab-scratch/`. These measurements were
not rerun against the later, test-only `f340749` parent of PR #96.

### Native x86 CLI measurement

**Indicative results from one machine**: Intel Core i5-14600KF, WSL2 Ubuntu 24.04, Linux 6.18, rustc 1.99.0. Both CLIs were built
from source tarballs: `14fd4b1` as the base and the adiabatic-lane candidate
subsequently committed for PR #96. The build command was:

```sh
RUSTFLAGS='-C target-cpu=x86-64-v3' cargo build --locked --release -p cli
```

Each KRAKEN trial started a fresh process, pinned to logical CPU 2, with a fresh
output directory:

```sh
taskset -c 2 pelagic kraken run <env> -o <fresh-dir> \
  --format legacy --threads 1 --no-sync
```

Unlike the macOS solver measurements above, these are whole CLI times, including
parsing and legacy output. Ratios here are **new/base time**, so below 1 is
faster. A mixed run of 21 interleaved trials per case gave these medians:

| Case | Base ms | New ms | New/base |
|---|---:|---:|---:|
| Gulf adiabatic | 79.37 | 56.41 | 0.711 |
| BELLHOP Munk TL (BELLHOP CLI) | 375.08 | 374.92 | 1.000 |

The other KRAKEN cases ranged from 0.998 to 1.044 in that mixed run. Separate
single-case controls used 61 interleaved trials per binary, in random order,
with the base, a byte-identical copy under another path, and the candidate:

| Case | Copy/base | New/base |
|---|---:|---:|
| Munk | 1.001 | 1.005 |
| Elastic sediment | 1.000 | 0.998 |
| Broadband | 0.997 | 0.998 |
| Surface duct | 1.003 | 1.001 |

The mixed run's Munk +4.4% did not survive the single-case control: it was an
interleaving artefact, not a reproduced solver regression. Legacy CLI output
matched the base for all six KRAKEN timing examples at threads 1 and 4. Gulf's
minor page faults (4,703 → 4,691) and peak RSS (23.4 → 23.1 MB) were effectively
unchanged. These numbers support the changelog's 0.71 on this machine; they are
not a general x86 guarantee.

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
were compared function by function, ignoring addresses. The comparison covers
instructions, not constant data, and it does not trace which functions run.
With the default build 3,198 of 3,285 base functions match; with v3, 3,190 of
3,271 match. The rest are the changed coupled FIELD code and functions in
unrelated modules, such as `FieldCase::new` and layer construction.

The numerical kernels of large Munk and KRAKENC are identical with both builds:
fluid root search, shooting, Sturm counts, mode shapes, and KRAKENC's secant
search and mode functions. Nine other functions on these paths differ with
each build:

- Eight are the `parallelism::with_threads` and `cancellation::with_token`
  instances that run a parallel map's items on several threads, for root
  search, mode shapes, KRAKENC and range-independent FIELD. Each changes by 1
  to 28 instructions. A map on one thread takes a different instance, so the
  one-thread timings never run them.
- The ninth is `tabulate_field_query`, range-independent FIELD itself. It has
  different register assignments and stack offsets, and with the default build
  a 16-byte larger stack frame. Its source is unchanged, it calls the same
  functions, and its mode and range loops, which compile as separate
  functions, are identical.

Local cross-builds of the `kraken` library for `x86_64-unknown-linux-gnu`, with
the same toolchain and release profile, show where these differences come
from. The profile splits the crate into 16 codegen units, and the new coupled
code moves `tabulate_field_query` from unit 12 to unit 05. With
`codegen-units = 1`, at both builds, the only functions whose instructions
differ are coupled FIELD and its generic helpers. `tabulate_field_query` and
the row maps of range-independent and adiabatic FIELD keep identical
instructions; only the names of a few call targets change, to a coupled FIELD
closure's new number. The script and
its output are kept with the evidence, outside the repository, in
`pelagic-wt-kprof-scratch/rdfield-2/cgu/` (`run.sh`, `compare.py` and
`results.txt`).

### Full-result identity

159 inputs (148 ENV/FLP workflows and the 11 timing inputs), at threads 1 and 4,
with both builds, give 636 base and 636 candidate results. All 636 pairs match:
548 complete results, including ordered mode counts, modes and pressures, and
88 identical expected errors (84 load errors and 4 solve errors, the same as on
`7246c5a`). Every result also matches the default base at one thread, so they
are identical across builds and thread counts.

### Tests

`cargo test -p kraken` passes 220 tests with 6 ignored, and workspace clippy
and rustfmt are clean. The independent review found that the new tests only
compared the grouped march with itself: changing its `while` loop over profile
midpoints to an `if` still passed them all. A further test,
`coupled_field_matches_the_sequential_march_when_receivers_skip_profiles`,
compares coupled FIELD bit for bit with a test-only copy of the `57f55a3`
march, which takes one range at a time. Its receiver grids skip profiles: one
receiver beyond every midpoint, receivers exactly at the midpoints, and scalar
and lane groups with the middle profiles empty. It runs with all three source
geometries, with 0, 1 and all modes, and with four table budgets, and it fails
with that `while`-to-`if` change.

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
numerical kernels compile identically.

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
coupled change. Other adiabatic lane widths were not investigated in that
experiment; the [new eight-wide kernel](#adiabatic-lanes-on-14fd4b1) revisits this
decision on the later base.

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

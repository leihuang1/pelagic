# Comparison with Fortran, November 2026

Measured 19–20 November 2026. The record compares Pelagic `main` at `9dc947c`, which adds pull requests #102–#107 to
the v0.6.3 release (`a05d461`), and v0.6.3 itself against the pinned Fortran Acoustics Toolbox (v2023.5, commit
`4751085`). Times are end to end on four platforms. The headline is Pelagic's default build against Fortran built with
`-O3 -march=native -ffast-math`. Ratios are Pelagic's median time divided by Fortran's on the same machine, so lower is
faster for Pelagic.

## Result

- **With `--no-sync`, Pelagic is faster than Fortran in every single case on every platform**, on one thread. It takes
  0.71–0.95 of Fortran's time on Linux (EPYC), 0.62–0.91 under WSL, 0.30–0.89 on native Windows and 0.47–0.70 on
  macOS.
- **Durable runs, the default, are faster on native Windows in every case and on macOS in every case but Pekeris**
  (1.14).
  - On Linux, Pekeris (2.4–2.6), broadband (1.1–1.4) and elastic sediment (1.2–1.5) still take longer than Fortran.
  - Munk and the EPYC's Gulf are about even.
  - Surface duct and BELLHOP are faster, and so is Gulf under WSL.

  Syncing file contents explains much of the small cases' deficit. On the EPYC, separate solver-only and CLI medians
  for Pekeris were 0.41 and 6.73 ms, and `--no-sync` cut the CLI median by 4.63 ms. Fortran does not sync its
  files.
- **Sweeps.**
  - With `--no-sync`, sweeps beat Fortran on every platform and worker count (0.08–0.97; surface duct at 24 workers on
    the EPYC is close to even).
  - Durable sweeps beat Fortran on native Windows.
  - Some durable small-case sweeps lose on Linux and macOS. Pekeris takes 1.7–4.1 times as long as
    Fortran (on macOS only with 10 or more workers), and EPYC surface duct with 12 or more workers takes 2.5–3.8
    times as long.
- **Threads.** Fortran is single-threaded, and the CLI's default thread count is the number of available CPUs. With it,
  `--no-sync` surface duct, Gulf, broadband and BELLHOP take 0.07–0.35 of Fortran's time on the three platforms
  measured.
- **v0.6.3 against `main`.**
  - One-thread BELLHOP takes 0.89–0.90 of v0.6.3's time on x86-64 Linux and WSL (#104). No clear change was
    measured on native Windows or macOS.
  - At four threads, broadband takes 0.60–0.77 of v0.6.3's time and Gulf 0.85–0.93 (#105, #106).
  - With the default 14–24 threads, no clear change was measured for either, since v0.6.3 already solved independent
    frequencies and profiles side by side from eight threads.
  - No other repeatable change was established. Small differences, and the large, variable durable-sweep
    differences, would need repeat runs.
- **Identity.** Within each platform, every checked acoustic product (the MOD and SHD files) was byte-identical
  between v0.6.3 and `main`, between durable and `--no-sync`, and across build variants, thread counts and worker
  counts. The checks do not compare platforms, or Pelagic with Fortran.

## Setup

| Platform | Machine | Pelagic builds | Fortran compiler |
| --- | --- | --- | --- |
| Linux | Google Colab VM: AMD EPYC 7B13, 12 cores / 24 threads; Linux 6.6, overlay file system | Built by the x86 kit with `cargo build --release --locked` | gfortran 13.3 |
| WSL | Intel Core i5-14600KF (6 performance and 8 efficiency cores) under WSL2, Ubuntu 24.04, ext4 | As on Linux | gfortran 13.3 |
| Windows | The same PC, Windows 11, NTFS | v0.6.3 release archive; `main` from CI's release packaging (static C runtime) | WinLibs gfortran 16.2 |
| macOS | Apple M4 Pro (10 performance and 4 efficiency cores), APFS | v0.6.3 release archive; `main` from CI's release packaging | Homebrew gfortran 16.2 |

**Workloads.** Every trial times one run from start to exit: one Pelagic process, Fortran's KRAKEN then FIELD, or a
whole sweep batch.
- **Single cases:** the seven cases of the x86 kit (`tools/perf/x86`): Pekeris, Munk, surface duct, Gulf (adiabatic),
  broadband (101 frequencies), elastic sediment and the BELLHOP Munk transmission-loss example.
- **Sweeps:** Pekeris ×200 and surface duct ×50, through `run-many` and through parallel Fortran processes.

**Procedure.**
- Output is `--format legacy`, with a fresh output directory for each trial, and both programs read the same input
  bytes.
- Variants are interleaved in a seeded random order after one warmup.
- Medians are of 15 trials for single cases and 3 batches for sweeps.

**Platform harnesses.**
- **Linux and WSL:** the x86 kit (`tools/perf/x86/run.py`).
- **EPYC default threads:** an extra pass on the same VM, of 7 trials per variant.
- **Windows and macOS:** harnesses of the same design.
- **Pinning:**
  - The kit and the Windows harness pin one-thread runs to one CPU.
  - On Windows, four-thread runs may use one logical CPU of each of the six performance cores, and default-thread runs
    use all 20 CPUs.
  - The EPYC default-thread pass runs every variant, its Fortran included, on all 24 CPUs.
  - macOS does not pin.

**Fortran flags.**
- **O3:** `-O3 -march=native -ffast-math -funroll-all-loops -fomit-frame-pointer -std=gnu`. macOS uses `-mcpu=native`
  for `-march=native` and adds `-g`; Windows adds `-static`.
- **O1:** the same at `-O1`, without `-march=native` or `-mcpu=native`. It was timed too.
- **Comparing with O1.** O3 was faster in every case but Pekeris, where O1 took up to 10.7% less time.
  - The `--no-sync` results hold against O1 as well.
  - Some durable ones change: on the EPYC, durable Munk and broadband beat O1 (0.79, 0.96), and elastic sediment is
    about even with it (1.01).

**Builds and evidence.**
- The Linux and WSL builds were made by the kit, which records commits, compilers and the hashes of binaries and
  inputs.
- The Windows and macOS binaries are release-style artifacts, and their runs record less provenance.
- Raw records and harnesses are kept outside the repository, under `pelagic-wt-xplat-scratch/` in the maintainer's
  report archive.

## One thread

`main` / Fortran O3, durable / `--no-sync`:

| Case | Linux (EPYC) | WSL | Windows | macOS |
| --- | ---: | ---: | ---: | ---: |
| Pekeris | 2.43 / 0.76 | 2.59 / 0.68 | 0.48 / 0.30 | 1.14 / 0.51 |
| Munk | 1.04 / 0.71 | 0.99 / 0.62 | 0.48 / 0.32 | 0.76 / 0.47 |
| Surface duct | 0.90 / 0.76 | 0.83 / 0.66 | 0.53 / 0.48 | 0.59 / 0.49 |
| Gulf | 0.98 / 0.86 | 0.93 / 0.81 | 0.70 / 0.63 | 0.77 / 0.70 |
| Broadband | 1.15 / 0.95 | 1.41 / 0.91 | 0.51 / 0.40 | 0.84 / 0.64 |
| Elastic sediment | 1.17 / 0.71 | 1.46 / 0.77 | 0.56 / 0.40 | 0.92 / 0.61 |
| BELLHOP Munk TL | 0.82 / 0.81 | 0.89 / 0.87 | 0.91 / 0.89 | 0.72 / 0.70 |

Durable small cases vary most:
- under WSL, `main`'s Pekeris ranged from 6.5 to 24 ms and v0.6.3's from 5.7 to 10.6 ms;
- the EPYC Gulf ranges overlap Fortran's.

## Threads

`main` / Fortran O3, `--no-sync`. Fortran is on one thread; brackets show v0.6.3 for the four-thread Gulf and
broadband runs.
WSL was not measured with threads.

| Case | Linux, 24 (default) | Windows, 4 | Windows, 20 (default) | macOS, 4 | macOS, 14 (default) |
| --- | ---: | ---: | ---: | ---: | ---: |
| Pekeris | 0.83 | 0.29 | 0.28 | 0.53 | 0.53 |
| Munk | 0.52 | 0.28 | 0.31 | 0.35 | 0.35 |
| Surface duct | 0.35 | 0.28 | 0.30 | 0.24 | 0.19 |
| Gulf | 0.34 | 0.35 [0.42] | 0.35 | 0.34 [0.37] | 0.26 |
| Broadband | 0.28 | 0.28 [0.40] | 0.26 | 0.38 [0.64] | 0.32 |
| Elastic sediment | 0.71 | 0.38 | 0.39 | 0.57 | 0.58 |
| BELLHOP Munk TL | 0.07 | 0.27 | 0.13 | 0.21 | 0.09 |

- **Gains with threads.** Gulf, broadband, surface duct and BELLHOP gain most from threads.
- **Little gain.** Elastic sediment's root search is sequential, and Pekeris is too small to split.
- **Where v0.6.3 differs.** It solved independent frequencies and profiles one at a time below eight threads, so
  broadband's small frequencies barely gained from four threads.

## Sweeps

Time per case, `main` / Fortran O3, durable / `--no-sync`, by number of workers (W):

| Platform | Pekeris ×200 | Surface duct ×50 |
| --- | --- | --- |
| Linux (EPYC) | W1 1.65 / 0.33; W12 1.97 / 0.23; W24 3.54 / 0.24 | W1 0.90 / 0.73; W12 2.47 / 0.72; W24 3.77 / 0.97 |
| WSL | W1 4.10 / 0.33; W10 3.00 / 0.49; W20 2.07 / 0.46 | W1 0.88 / 0.66; W10 0.88 / 0.69; W20 0.88 / 0.76 |
| Windows | W1 0.29 / 0.08; W12 0.49 / 0.12; W20 0.46 / 0.11 | W1 0.45 / 0.37; W12 0.54 / 0.43; W20 0.56 / 0.42 |
| macOS | W1 0.98 / 0.11; W10 2.57 / 0.24; W14 2.47 / 0.26 | W1 0.52 / 0.42; W10 0.65 / 0.44; W14 0.62 / 0.42 |

Durable runs sync each case's file contents before installing them; Apple platforms also flush the drive cache. On
native Windows, even durable Pelagic sweeps had lower medians than Fortran's runs of two processes per KRAKEN case.

## v0.6.3 against `main`

- **Single cases at one thread:** `main` / v0.6.3 is within 0.96–1.03 everywhere, except two cases:
  - BELLHOP takes 0.89 on the EPYC and 0.90 under WSL. #104 builds the BELLHOP crate as one codegen unit; this
    native-Windows build shows no change.
  - Durable Pekeris under WSL is 1.28, a ratio of the medians of two widely spread distributions; its `--no-sync`
    ratio is 1.03.
- **Four threads:**
  - broadband takes 0.71–0.77 of v0.6.3's time on Windows and 0.60–0.69 on macOS;
  - Gulf takes 0.85–0.86 and 0.92–0.93.

## Notes

- **Noise.** These are single runs on shared machines: a Colab VM, the PC its owner was using, and a desktop Mac with a
  load average around 5. As a rule of thumb, treat differences under about 5% with caution; that is not a measured
  bound.
- **The macOS FIELD binary.** On macOS, `field.exe` from a full Acoustics Toolbox build (`misc`, `Bellhop`, `Kraken`,
  then `KrakenField`) crashed with a segmentation fault on Gulf. FIELD built from `misc` and `KrakenField` alone has
  byte-identical code and data sections and completed every trial, and the macOS Fortran timings use that build.
  Linux and Windows were not affected.
- **What durable means.** Pelagic's default mode syncs file contents before installing them; neither mode guarantees
  directory-entry durability (see the [CLI guide](../guide/cli.md)). Fortran makes no durability promise, so `--no-sync`
  is the like-for-like speed comparison.

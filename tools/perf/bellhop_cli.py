#!/usr/bin/env python3
"""Interleaved BELLHOP CLI trials, including publication; no third-party packages.

Uses macOS /usr/bin/time -l for per-process peak RSS. The Python wall clock
includes process startup and the time wrapper, not output hashing or cleanup.
Every artifact must match the baseline's warm-up output byte for byte.
"""
import argparse
import contextlib
import json
import os
import pathlib
import random
import re
import signal
import statistics
import subprocess
import sys
import tempfile
import time

import run as perf


def rss_bytes(text):
    match = re.search(r"^\s*(\d+)\s+maximum resident set size$", text, re.MULTILINE)
    if not match:
        raise ValueError("missing macOS peak RSS measurement")
    return int(match[1])


def invoke(binary, case, threads):
    with tempfile.TemporaryDirectory(prefix="bellhop-cli-") as directory:
        root = pathlib.Path(directory)
        output = root / "output"
        timing = root / "time.txt"
        command = [str(pathlib.Path(binary).resolve()), "--threads", str(threads),
                   "bellhop", "run", case["path"], "--format", "legacy", "-o", str(output)]
        start = time.monotonic_ns()
        process = subprocess.Popen(["/usr/bin/time", "-l", "-o", str(timing), *command],
                                   cwd=perf.ROOT, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                                   text=True, start_new_session=True)
        try:
            stdout, stderr = process.communicate(timeout=120)
        except BaseException as error:
            # A new session hides the solver from Ctrl-C, and the wrapper does not
            # forward a kill, so a solver left running would disturb later timings.
            with contextlib.suppress(PermissionError, ProcessLookupError):
                os.killpg(process.pid, signal.SIGKILL)
            process.communicate()
            if isinstance(error, subprocess.TimeoutExpired):
                raise RuntimeError(f"CLI timed out: {command}") from None
            raise
        elapsed = time.monotonic_ns() - start
        if process.returncode:
            raise RuntimeError(f"CLI failed ({process.returncode}): {command}\n{stderr}")
        artifacts = {str(p.relative_to(output)): perf.sha(p.read_bytes())
                     for p in sorted(output.rglob("*")) if p.is_file()}
        if not artifacts:
            raise RuntimeError("CLI produced no output files")
        return dict(status="ok", command=command, samples_ns=[elapsed],
                    max_rss_bytes=rss_bytes(timing.read_text()), artifacts=artifacts,
                    stderr=stderr, stdout=stdout)


def check_artifacts(row, expected):
    if row["artifacts"] != expected:
        raise RuntimeError(f"CLI artifacts differ from baseline: {row['command']}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", action="append", required=True, help="label=/absolute/pelagic")
    parser.add_argument("--cases", default=str(perf.ROOT / "tools/perf/bellhop-cases.json"))
    parser.add_argument("--output", required=True)
    parser.add_argument("--rounds", type=int, default=11)
    parser.add_argument("--threads", type=int, nargs="+", default=[1, 14])
    parser.add_argument("--seed", type=int, default=20261103)
    parser.add_argument("--build-provenance")
    args = parser.parse_args()
    if sys.platform != "darwin":
        parser.error("this runner uses macOS /usr/bin/time -l; use run.py for portable solver timing")
    if any("=" not in value for value in args.binary):
        parser.error("--binary takes label=/absolute/pelagic")
    variants = dict(value.split("=", 1) for value in args.binary)
    if not all(pathlib.Path(binary).is_file() for binary in variants.values()):
        parser.error("every --binary path must be a file")
    if len(variants) != len(args.binary) or "baseline" not in variants:
        parser.error("unique labels and a baseline binary are required")
    if args.rounds < 5 or not args.threads or min(args.threads) < 1 or len(set(args.threads)) != len(args.threads):
        parser.error("at least five rounds and distinct positive thread counts are required")
    cases = perf.selected_cases(args)
    if any(case["model"] != "bellhop" for case in cases):
        parser.error("only BELLHOP cases are supported")
    path = pathlib.Path(args.output).resolve()
    with path.open("x") as log:
        manifest, binaries = perf.run_manifest(args, variants, cases, path.with_name(path.name + ".manifest.json"))
        expected = {}
        for case in cases:
            for threads in args.threads:
                expected[case["key"], threads] = invoke(variants["baseline"], case, threads)["artifacts"]
                for label, binary in variants.items():
                    if label != "baseline":
                        check_artifacts(invoke(binary, case, threads), expected[case["key"], threads])
        rng = random.Random(args.seed)
        trials = [(block, threads, case) for block in range(args.rounds)
                  for threads in args.threads for case in cases]
        rng.shuffle(trials)
        for block, threads, case in trials:
            labels = list(variants)
            rng.shuffle(labels)
            for label in labels:
                row = dict(case, variant=label, threads=threads, block=block, seed=args.seed,
                           utc=time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
                           binary_sha256=binaries[label]["sha256"], **manifest)
                try:
                    row.update(invoke(variants[label], case, threads))
                except (RuntimeError, ValueError, OSError) as error:
                    row.update(status="error", error=str(error))
                    perf.write_row(log, row)
                    raise
                row["baseline_identical"] = row["artifacts"] == expected[case["key"], threads]
                perf.write_row(log, row)
                check_artifacts(row, expected[case["key"], threads])
    summary = perf.summarize(path)
    rows = perf.read_rows(path)
    for item in summary:
        memory = [r["max_rss_bytes"] for r in rows if
                  (r["key"], r["threads"], r["variant"]) == (item["case"], item["threads"], item["variant"])]
        item.update(median_rss_bytes=statistics.median(memory), max_rss_bytes=max(memory))
    path.with_suffix(".summary.json").write_bytes(perf.json_bytes(summary))


def self_test():
    assert rss_bytes("  12345  maximum resident set size\n") == 12345
    try:
        rss_bytes("no measurement")
    except ValueError:
        pass
    else:
        raise AssertionError("missing RSS accepted")
    check_artifacts({"artifacts": {"a": "hash"}}, {"a": "hash"})
    try:
        check_artifacts({"artifacts": {}, "command": ["fake"]}, {"a": "hash"})
    except RuntimeError:
        pass
    else:
        raise AssertionError("different artifacts accepted")
    if sys.platform != "darwin":
        print("bellhop_cli self-test passed (parsing only: invocation needs macOS)")
        return
    with tempfile.TemporaryDirectory() as directory:
        binary = pathlib.Path(directory) / "fake-cli"
        binary.write_text("#!/bin/sh\nwhile [ \"$1\" != '-o' ]; do shift; done\n"
                          "mkdir \"$2\"\nprintf deterministic > \"$2/result\"\n")
        binary.chmod(0o755)
        case = {"path": "unused"}
        first = invoke(binary, case, 1)
        assert first["samples_ns"][0] > 0 and first["max_rss_bytes"] > 0
        check_artifacts(invoke(binary, case, 2), first["artifacts"])
        binary.write_text("#!/bin/sh\nexit 3\n")
        try:
            invoke(binary, case, 1)
        except RuntimeError:
            pass
        else:
            raise AssertionError("failed CLI accepted")
    print("bellhop_cli self-test passed")


if __name__ == "__main__":
    if sys.argv[1:] == ["--self-test"]:
        self_test()
    else:
        main()

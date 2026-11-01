#!/usr/bin/env python3
"""Serial real-solver snapshots and seeded, interleaved solver-only trials.

No third-party packages. JSONL retains failures, commands, timestamps and timings.
Snapshot text follows the existing parallelism tests' round-trip float contract.
"""
import argparse
import collections
import gzip
import hashlib
import json
import math
import os
import pathlib
import platform
import random
import statistics
import subprocess
import sys
import tempfile
import time

ROOT = pathlib.Path(__file__).resolve().parents[2]


def sha(data):
    return hashlib.sha256(data).hexdigest()


PAIRINGS = ROOT / "tools/perf/pairings.json"
# These are results, which must stay identical. Any other failure, such as a crash or
# a timeout, leaves nothing to compare, so it fails the check whatever other runs did.
COMPARED_DIAGNOSTICS = {"load-error", "solve-error"}


def file_record(path, role):
    path = pathlib.Path(path)
    try:
        data = (ROOT / path).read_bytes()
    except FileNotFoundError:
        return dict(path=str(path), role=role, status="missing", sha256=None)
    return dict(path=str(path), role=role, status="present", sha256=sha(data))


def case_inputs(item):
    path = pathlib.Path(item["path"])
    primary = file_record(path, "primary")
    inputs = [primary]
    if path.suffix == ".env":
        if item["model"] in ("kraken", "modal"):
            flp = pathlib.Path(item.get("flp", path.with_suffix(".flp")))
            inputs.append(file_record(flp, "flp"))
            candidates = [(path.with_suffix("." + ext), ext) for ext in ("trc", "brc", "irc")]
            candidates.append((flp.with_suffix(".sbp"), "sbp"))
        else:
            candidates = [(path.with_suffix("." + ext), ext)
                          for ext in ("ssp", "ati", "bty", "brc", "trc", "irc", "sbp")]
        for candidate, role in candidates:
            inputs.append(dict(file_record(candidate, role), consumption="unknown", kind="auxiliary-candidate"))
    # JSON inputs use the self-contained document contract, not neighboring files.
    return dict(item, input_sha256=primary["sha256"], inputs=inputs)


def validate_cases(items):
    if not isinstance(items, list) or not items:
        raise ValueError("cases must be a nonempty JSON array")
    keys = set()
    for item in items:
        if not isinstance(item, dict) or any(not isinstance(item.get(k), str) or not item[k]
                                             for k in ("key", "model", "path")):
            raise ValueError("each case requires nonempty key, model and path strings")
        if item["key"] in keys:
            raise ValueError(f"duplicate case key: {item['key']}")
        keys.add(item["key"])
        if item["model"] not in ("kraken", "modal", "bellhop") or item.get("engine", "kraken") not in ("kraken", "krakenc"):
            raise ValueError("unknown model or engine")
        if "flp" in item and (not isinstance(item["flp"], str) or not item["flp"]
                              or item["model"] == "bellhop" or pathlib.Path(item["path"]).suffix != ".env"):
            raise ValueError("explicit FLP requires a nonempty path and KRAKEN legacy ENV input")
    return items


def inventory(selection="all"):
    items = []
    for module in ("kraken", "bellhop"):
        fixtures = ROOT / "crates" / module / "tests" / "fixtures"
        paths = set(fixtures.rglob("*.env"))
        paths.update((ROOT / "examples" / "cases" / module).rglob("*.json"))
        paths.update((ROOT / "tools/perf/bench").glob("kr-*.json" if module == "kraken" else "bh-*.json"))
        for path in sorted(paths):
            # KRAKEN goldens contain outputs, not source ENV files; BELLHOP retains ENV there.
            engines = ("kraken", "krakenc") if module == "kraken" and path.suffix == ".env" else ("kraken",)
            for engine in engines:
                models = ("kraken", "modal") if module == "kraken" and path.suffix == ".env" else (module,)
                for model in models:
                    relative = str(path.relative_to(ROOT))
                    key = f"{model}:{engine}:{relative}"
                    items.append(dict(key=key, model=model, engine=engine, path=relative))
    original = items
    supplementary = []
    for pair in json.loads(PAIRINGS.read_text())["pairs"]:
        for engine in ("kraken", "krakenc"):
            for model in ("kraken", "modal"):
                key = f"{model}:{engine}:{pair['path']}:flp={pair['flp']}"
                supplementary.append(dict(pair, key=key, model=model, engine=engine, supplementary=True))
    items = {"original": original, "supplementary": supplementary, "all": original + supplementary}[selection]
    return [case_inputs(item) for item in validate_cases(items)]


def selected_cases(args):
    if args.cases:
        return [case_inputs(item) for item in validate_cases(json.loads(pathlib.Path(args.cases).read_text()))]
    return inventory(args.selection)


def invocation_command(binary, item, threads, warmup, reps, snapshot=None):
    command = [str(pathlib.Path(binary).resolve()), item["model"], item["path"], str(threads),
               str(warmup), str(reps), str(snapshot) if snapshot else "-", item.get("engine", "kraken")]
    # No placeholder: binaries built before the optional FLP accept only these arguments.
    if "flp" in item:
        command.append(item["flp"])
    return command


def invoke(binary, item, threads, warmup, reps, snapshot=None, timeout=120):
    command = invocation_command(binary, item, threads, warmup, reps, snapshot)
    start = time.monotonic_ns()
    row = dict(command=command, utc=time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()))
    try:
        process = subprocess.run(command, cwd=ROOT, text=True, capture_output=True, timeout=timeout)
        row.update(exit_code=process.returncode, stderr=process.stderr)
        if process.returncode:
            row.update(status="process-error", stdout=process.stdout)
        else:
            row.update(json.loads(process.stdout))
    except subprocess.TimeoutExpired:
        row.update(status="timeout", timeout=timeout)
    row["process_ns"] = time.monotonic_ns() - start
    return row


def write_row(handle, row):
    handle.write(json.dumps(row, sort_keys=True) + "\n")
    handle.flush()


def json_bytes(value):
    return (json.dumps(value, sort_keys=True, indent=2) + "\n").encode()


def build_identity(digest, supplied):
    record = supplied.get(digest)
    if record is not None and not isinstance(record, dict):
        raise ValueError("build provenance entries must be objects keyed by binary SHA-256")
    identity = dict.fromkeys(("source_commit", "source_dirty", "build_command", "toolchain", "flags"), "unknown")
    if record is not None:
        identity.update(record)
    return dict(identity, provenance="caller-supplied (not independently attested)" if record is not None else "unknown")


def run_manifest(args, variants, items, path):
    supplied = {}
    if args.build_provenance:
        supplied = json.loads(pathlib.Path(args.build_provenance).read_text())
        if not isinstance(supplied, dict) or any(len(k) != 64 or any(c not in "0123456789abcdef" for c in k) for k in supplied):
            raise ValueError("build provenance must be an object keyed by lowercase binary SHA-256")
    binaries = {}
    for label, binary in variants.items():
        digest = sha(pathlib.Path(binary).read_bytes())
        binaries[label] = dict(path=str(pathlib.Path(binary).resolve()), sha256=digest,
                               build=build_identity(digest, supplied))
    def git(*command):
        return subprocess.check_output(["git", *command], cwd=ROOT)
    files = {"tools/perf/run.py", "tools/perf/src/main.rs", "tools/perf/Cargo.toml",
             "tools/perf/Cargo.lock", "tools/perf/cases.json", "tools/perf/pairings.json",
             "tools/perf/README.md", "Cargo.lock"}
    if args.cases:
        files.add(str(pathlib.Path(args.cases).resolve()))
    evidence = json.loads(PAIRINGS.read_text())
    for entry in evidence["pairs"] + evidence["unavailable_or_ambiguous"]:
        files.update(source["path"] for source in entry["evidence"])
    if args.build_provenance:
        files.add(str(pathlib.Path(args.build_provenance).resolve()))
    baseline_files = []
    if getattr(args, "baseline", None):
        baseline_files = [file_record(pathlib.Path(args.baseline).resolve() / name, "baseline-evidence")
                          for name in ("results.jsonl", "inventory.json", "summary.json")]
    environment = dict(os.environ)
    visible_environment = {key: value for key, value in environment.items()
                           if key in ("PATH", "HOME", "TMPDIR", "TMP", "TEMP", "LANG", "SDKROOT", "MACOSX_DEPLOYMENT_TARGET",
                                      "RUSTFLAGS", "RUSTDOCFLAGS", "RUSTUP_TOOLCHAIN", "RUSTUP_HOME", "CARGO_HOME",
                                      "CARGO_TARGET_DIR", "CARGO_ENCODED_RUSTFLAGS", "RUSTC", "RUSTC_WRAPPER")
                           or key.startswith(("LC_", "DYLD_", "LD_", "OMP_"))}
    status = git("status", "--porcelain=v1", "--untracked-files=all").decode()
    manifest = dict(schema_version=1, utc=time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
                    command=[sys.executable, *sys.argv], invocation_cwd=os.getcwd(), solver_cwd=str(ROOT),
                    arguments=vars(args), platform=platform.platform(), python=sys.version,
                    environment=visible_environment, environment_sha256=sha(json_bytes(environment)),
                    environment_policy="listed runtime/build-related variables only; full environment hashed, not disclosed",
                    binaries=binaries,
                    harness_tree=dict(root=str(ROOT), head=git("rev-parse", "HEAD").decode().strip(),
                                      head_tree=git("rev-parse", "HEAD^{tree}").decode().strip(),
                                      dirty=bool(status), status=status,
                                      tracked_diff_sha256=sha(git("diff", "HEAD", "--binary")),
                                      meaning="current harness checkout, NOT executable build identity"),
                    files=[file_record(name, "harness-or-evidence") for name in sorted(files)],
                    cases=items, selected_cases_sha256=sha(json_bytes(items)), baseline_files=baseline_files,
                    input_hash_policy="pre-run reads; auxiliary candidates have unknown consumption; no concurrent input/binary edits permitted")
    data = json_bytes(manifest)
    with path.open("xb") as handle:
        handle.write(data)
    path.chmod(0o444)
    return dict(run_manifest=str(path.resolve()), run_manifest_sha256=sha(data)), binaries


def same_case_inputs(previous, item):
    defaults = {"engine": "kraken"}
    return (all(previous.get(key, defaults.get(key)) == item.get(key, defaults.get(key))
                for key in ("model", "engine", "path", "flp", "input_sha256"))
            and ("inputs" not in previous or previous["inputs"] == item["inputs"]))


def check(args):
    items = selected_cases(args)
    out = pathlib.Path(args.output).resolve()
    out.mkdir(parents=True, exist_ok=False)
    baseline = {}
    if args.baseline:
        baseline = {r["key"]: r for r in read_rows(pathlib.Path(args.baseline) / "results.jsonl") if r["threads"] == 1}
    manifest_ref, binaries = run_manifest(args, {"check": args.binary}, items, out / "run-manifest.json")
    (out / "inventory.json").write_text(json.dumps(items, indent=2) + "\n")
    failures = 0
    unexpected = 0
    totals = collections.Counter()
    with (out / "results.jsonl").open("x") as log:
        for item in items:
            single = None
            single_outcome = None
            for threads in args.threads:
                filename = sha(item["key"].encode())[:24] + f"-t{threads}.txt"
                snapshot = out / filename
                row = dict(item, threads=threads, binary_sha256=binaries["check"]["sha256"], **manifest_ref)
                row.update(invoke(args.binary, item, threads, 0, 1, snapshot, args.timeout))
                totals[row["status"]] += 1
                if args.baseline and (previous := baseline.get(item["key"])) is not None:
                    if not same_case_inputs(previous, item):
                        row["baseline_input_mismatch"] = True
                        failures += 1
                outcome = (row["status"], row.get("error"))
                if threads == 1:
                    single_outcome = outcome
                elif outcome != single_outcome:
                    row["thread_diagnostic_mismatch"] = True
                    failures += 1
                if row["status"] == "ok":
                    data = snapshot.read_bytes()
                    row["snapshot_sha256"] = sha(data)
                    row["snapshot_bytes"] = len(data)
                    row["snapshot"] = filename + ".gz"
                    with gzip.open(out / row["snapshot"], "wb", compresslevel=1) as zipped:
                        zipped.write(data)
                    snapshot.unlink()
                    if single is not None and data != single:
                        row["thread_mismatch"] = True
                        failures += 1
                    single = data if threads == 1 else single
                    if args.baseline:
                        previous = baseline.get(item["key"])
                        if previous and previous["status"] == "ok":
                            with gzip.open(pathlib.Path(args.baseline) / previous["snapshot"], "rb") as source:
                                same = data == source.read()
                            row["baseline_identical"] = same
                            failures += not same
                        else:
                            row["baseline_status_changed"] = True
                            failures += 1
                elif row["status"] not in COMPARED_DIAGNOSTICS:
                    row["unexpected_outcome"] = True
                    unexpected += 1
                elif args.baseline:
                    previous = baseline.get(item["key"])
                    same = previous is not None and row["status"] == previous["status"] and row.get("error") == previous.get("error")
                    row["baseline_diagnostic_identical"] = same
                    failures += not same
                write_row(log, row)
    summary = dict(status_counts=totals, mismatches=failures, unexpected_outcomes=unexpected,
                   configurations=len(items) * len(args.threads))
    (out / "summary.json").write_text(json.dumps(summary, indent=2) + "\n")
    print(json.dumps(summary))
    if failures or unexpected:
        raise SystemExit(1)


def bench(args):
    variants = dict(v.split("=", 1) for v in args.binary)
    if len(variants) != len(args.binary):
        raise ValueError("variant labels must be unique")
    cases = selected_cases(args)
    rng = random.Random(args.seed)
    trials = [(r, t, case) for r in range(args.rounds) for t in args.threads for case in cases]
    # Each round interleaves all variants for the same case/thread; order is seeded.
    rng.shuffle(trials)
    output = pathlib.Path(args.output).resolve()
    with output.open("x") as log:
        manifest_ref, binaries = run_manifest(args, variants, cases, output.with_name(output.name + ".manifest.json"))
        for block, threads, case in trials:
            labels = list(variants)
            rng.shuffle(labels)
            for label in labels:
                row = dict(case, variant=label, threads=threads, block=block, seed=args.seed,
                           warmup=args.warmup, reps=args.reps, binary_sha256=binaries[label]["sha256"], **manifest_ref)
                row.update(invoke(variants[label], case, threads, args.warmup, args.reps, timeout=args.timeout))
                write_row(log, row)
                if row["status"] != "ok":
                    raise RuntimeError(f"benchmark failed: {row}")
    summarize(args.output)


def read_rows(path):
    return [json.loads(line) for line in pathlib.Path(path).read_text().splitlines() if line]


def percentile(values, fraction):
    ordered = sorted(values)
    position = (len(ordered) - 1) * fraction
    lo = math.floor(position)
    hi = math.ceil(position)
    return ordered[lo] + (ordered[hi] - ordered[lo]) * (position - lo)


def summarize(path):
    groups = collections.defaultdict(list)
    blocks = collections.defaultdict(dict)
    for row in read_rows(path):
        if row["status"] != "ok":
            continue
        key = (row["key"], row["threads"], row["variant"])
        groups[key].extend(ns / 1e6 for ns in row["samples_ns"])
        blocks[key][row["block"]] = statistics.median(row["samples_ns"])
    summary = []
    rng = random.Random(8675309)
    for (case, threads, variant), values in sorted(groups.items()):
        row = dict(case=case, threads=threads, variant=variant, n=len(values), median_ms=statistics.median(values),
                   p10_ms=percentile(values, .1), p90_ms=percentile(values, .9), min_ms=min(values), max_ms=max(values))
        base = blocks.get((case, threads, "baseline"))
        current = blocks[(case, threads, variant)]
        if base and set(base) == set(current):
            # Bootstrap independent randomized blocks, not correlated repetitions inside a process.
            ratios = [base[b] / current[b] for b in sorted(base)]
            boot = [statistics.median(rng.choices(ratios, k=len(ratios))) for _ in range(4000)]
            row.update(paired_speedup=statistics.median(ratios), speedup_ci95=[percentile(boot, .025), percentile(boot, .975)], blocks=len(ratios))
        summary.append(row)
    destination = pathlib.Path(path).with_suffix(".summary.json")
    destination.write_text(json.dumps(summary, indent=2) + "\n")
    print(destination)
    return summary


def self_test():
    assert percentile([1, 2, 3], .5) == 2
    assert percentile([1, 3], .25) == 1.5
    items = inventory()
    original = inventory("original")
    supplementary = inventory("supplementary")
    pairings = json.loads(PAIRINGS.read_text())
    assert original
    assert items == original + supplementary
    assert len(supplementary) == 4 * len(pairings["pairs"])
    assert len({i["key"] for i in items}) == len(items)
    assert {i["model"] for i in items} == {"kraken", "modal", "bellhop"}
    assert all("flp" not in item for item in original)
    for item in supplementary:
        assert all(source["status"] == "present" for source in item["inputs"][:2])
        assert all((ROOT / evidence["path"]).is_file() for evidence in item["evidence"])
    old = original[0]
    paired = supplementary[0]
    assert same_case_inputs({k: v for k, v in old.items() if k != "inputs"}, old)
    assert same_case_inputs(paired, paired)
    assert not same_case_inputs(paired, dict(paired, flp="other.flp"))
    assert not same_case_inputs(paired, dict(paired, inputs=[]))
    assert len(invocation_command("old-binary", old, 1, 0, 1)) == 8
    assert invocation_command("new-binary", paired, 4, 0, 1)[-1] == paired["flp"]
    assert build_identity("unused", {})["source_commit"] == "unknown"
    assert build_identity("known", {"known": {"source_commit": "abc"}})["source_commit"] == "abc"
    assert parse_args(["check", "--binary", "x", "--output", "y"]).threads[0] == 1
    assert parse_args(["check", "--binary", "x", "--output", "y", "--supplementary-only"]).selection == "supplementary"
    for invalid in ([dict(paired, model="bellhop")], [dict(paired, path="case.json")],
                    [dict(paired, flp="")], [paired, paired], []):
        try:
            validate_cases(invalid)
        except ValueError:
            pass
        else:
            raise AssertionError(f"invalid case accepted: {invalid}")
    import contextlib
    import io
    for tail in (["--threads", "4"], ["--threads", "1", "0"], ["--threads", "1", "1"],
                 ["--timeout", "0"], ["--cases", "x", "--supplementary-only"]):
        with contextlib.redirect_stderr(io.StringIO()):
            try:
                parse_args(["check", "--binary", "x", "--output", "y", *tail])
            except SystemExit as error:
                assert error.code == 2
            else:
                raise AssertionError(f"invalid arguments accepted: {tail}")
    with tempfile.TemporaryDirectory() as directory:
        directory = pathlib.Path(directory)
        env = directory / "environment.env"
        flp = directory / "geometry.flp"
        env.write_bytes(b"test primary")
        flp.write_bytes(b"test field")
        env.with_suffix(".brc").write_bytes(b"ENV owns BRC")
        flp.with_suffix(".sbp").write_bytes(b"FLP owns SBP")
        env.with_suffix(".sbp").write_bytes(b"not a candidate")
        material = case_inputs(dict(paired, path=str(env), flp=str(flp)))
        resources = {r["role"]: r for r in material["inputs"]}
        assert resources["primary"]["sha256"] == sha(b"test primary")
        assert resources["flp"]["sha256"] == sha(b"test field")
        assert resources["brc"]["sha256"] == sha(b"ENV owns BRC")
        assert resources["sbp"]["sha256"] == sha(b"FLP owns SBP")
        assert resources["trc"]["status"] == "missing" and resources["trc"]["consumption"] == "unknown"
        binary = directory / "binary"
        binary.write_bytes(b"not executed")
        args = parse_args(["check", "--binary", str(binary), "--output", str(directory / "out")])
        manifest_path = directory / "manifest.json"
        ref, _ = run_manifest(args, {"check": str(binary)}, [material], manifest_path)
        manifest = json.loads(manifest_path.read_text())
        assert ref["run_manifest_sha256"] == sha(manifest_path.read_bytes())
        assert manifest["binaries"]["check"]["build"]["source_commit"] == "unknown"
        assert manifest["harness_tree"]["head"]
        try:
            run_manifest(args, {"check": str(binary)}, [material], manifest_path)
        except FileExistsError:
            pass
        else:
            raise AssertionError("manifest was overwritten")
        path = directory / "sample.jsonl"
        rows = [dict(key="x", threads=1, variant=v, block=b, status="ok", samples_ns=[n, n])
                for b in range(3) for v, n in [("baseline", 2000000), ("candidate", 1000000)]]
        path.write_text("".join(json.dumps(r) + "\n" for r in rows))
        result = summarize(path)
        candidate = next(r for r in result if r["variant"] == "candidate")
        assert candidate["paired_speedup"] == 2 and candidate["speedup_ci95"] == [2, 2]
        cases = directory / "cases.json"
        cases.write_text(json.dumps([dict(key="case", model="kraken", path="examples/cases/kraken/pekeris/pekeris.json")]))

        def check_with(name, script, *options):
            fake = directory / name
            fake.write_text(f"#!/bin/sh\n{script}\n")
            fake.chmod(0o755)
            output = directory / f"{name}-output"
            arguments = parse_args(["check", "--binary", str(fake), "--output", str(output), "--cases", str(cases),
                                    "--threads", "1", "2", *options])
            code = 0
            with contextlib.redirect_stdout(io.StringIO()):
                try:
                    check(arguments)
                except SystemExit as error:
                    code = error.code
            return code, json.loads((output / "summary.json").read_text()), read_rows(output / "results.jsonl")

        # $3 is the thread count and $6 the snapshot path.
        crash = 'echo "failed on $3 threads" >&2; exit "$3"'
        code, summary, rows = check_with("crash", crash)
        assert code == 1 and summary["unexpected_outcomes"] == 2 and summary["mismatches"] == 0
        assert [(r["exit_code"], r["stderr"]) for r in rows] == [(1, "failed on 1 threads\n"), (2, "failed on 2 threads\n")]
        code, summary, rows = check_with("crash-again", crash, "--baseline", str(directory / "crash-output"))
        assert code == 1 and summary["unexpected_outcomes"] == 2
        assert not any("baseline_diagnostic_identical" in r for r in rows)
        code, summary, _ = check_with("hang", "exec sleep 30", "--threads", "1", "--timeout", "1")
        assert code == 1 and summary["unexpected_outcomes"] == 1 and summary["status_counts"] == {"timeout": 1}
        # Diagnostics and results are still compared across threads and with the baseline.
        diagnostic = """printf '{"status":"solve-error","error":"%s"}\\n' """
        code, summary, _ = check_with("same-error", diagnostic + '"same"')
        assert code == 0 and summary["unexpected_outcomes"] == 0 and summary["mismatches"] == 0
        code, summary, _ = check_with("thread-error", diagnostic + '"$3"')
        assert code == 1 and summary["unexpected_outcomes"] == 0 and summary["mismatches"] == 1
        code, summary, rows = check_with("changed-error", diagnostic + '"other"', "--baseline", str(directory / "same-error-output"))
        assert code == 1 and summary["mismatches"] == 2 and not any(r["baseline_diagnostic_identical"] for r in rows)
        snapshot = """printf '{"status":"ok"}\\n'; printf %s """
        code, summary, _ = check_with("same-bytes", snapshot + 'same > "$6"')
        assert code == 0 and summary["status_counts"] == {"ok": 2} and summary["mismatches"] == 0
        code, summary, _ = check_with("thread-bytes", snapshot + '"$3" > "$6"')
        assert code == 1 and summary["mismatches"] == 1
        code, summary, _ = check_with("changed-bytes", snapshot + 'other > "$6"', "--baseline", str(directory / "same-bytes-output"))
        assert code == 1 and summary["mismatches"] == 2
    print(f"self-test passed; {len(items)} input configurations")


def thread_counts(*counts):
    """The given counts below this machine's CPU count, then the CPU count itself."""
    cpus = os.cpu_count() or 1
    return [count for count in counts if count < cpus] + [cpus]


def add_selection(parser):
    selection = parser.add_mutually_exclusive_group()
    selection.add_argument("--cases", help="explicit JSON case list; same schema as benchmark cases")
    selection.add_argument("--original-only", dest="selection", action="store_const", const="original")
    selection.add_argument("--supplementary-only", dest="selection", action="store_const", const="supplementary")
    parser.set_defaults(selection="all")


def parse_args(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="action", required=True)
    commands.add_parser("self-test")
    add_selection(commands.add_parser("inventory"))
    checks = commands.add_parser("check")
    checks.add_argument("--binary", required=True)
    checks.add_argument("--output", required=True)
    checks.add_argument("--baseline")
    checks.add_argument("--threads", nargs="+", type=int, default=thread_counts(1, 2, 4))
    checks.add_argument("--timeout", type=int, default=120)
    checks.add_argument("--build-provenance", help="caller-supplied build records keyed by binary SHA-256")
    add_selection(checks)
    benchmarks = commands.add_parser("bench")
    benchmarks.add_argument("--binary", action="append", required=True, help="label=/absolute/binary")
    benchmarks.add_argument("--cases", required=True)
    benchmarks.add_argument("--output", required=True)
    benchmarks.add_argument("--threads", nargs="+", type=int, default=thread_counts(1, 4))
    benchmarks.add_argument("--rounds", type=int, default=11)
    benchmarks.add_argument("--reps", type=int, default=3)
    benchmarks.add_argument("--warmup", type=int, default=1)
    benchmarks.add_argument("--seed", type=int, default=20261031)
    benchmarks.add_argument("--timeout", type=int, default=120)
    benchmarks.add_argument("--build-provenance", help="caller-supplied build records keyed by binary SHA-256")
    summary = commands.add_parser("summarize")
    summary.add_argument("path")
    args = parser.parse_args(argv)
    if args.action in ("check", "bench"):
        if not args.threads or args.threads[0] != 1 or any(t < 1 for t in args.threads) or len(set(args.threads)) != len(args.threads):
            parser.error("thread list must start with 1 and contain distinct positive values")
        if args.timeout < 1:
            parser.error("timeout must be positive")
    if args.action == "bench" and (args.rounds < 1 or args.reps < 1 or args.warmup < 0):
        parser.error("rounds/reps must be positive, warmup nonnegative")
    return args


def main():
    args = parse_args()
    if args.action == "self-test":
        self_test()
    elif args.action == "inventory":
        print(json.dumps(selected_cases(args), indent=2))
    elif args.action == "check":
        check(args)
    elif args.action == "bench":
        bench(args)
    else:
        summarize(args.path)


if __name__ == "__main__":
    main()

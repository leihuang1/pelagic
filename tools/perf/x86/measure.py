"""Measurement blocks, output digests and on-machine summaries."""
import collections
import functools
import io
import json
import math
import os
from pathlib import Path
import random
import re
import shutil
import signal
import statistics
import struct
import subprocess
import tarfile
import time
import types

from run import append, digest, load, network_names, perf_driver, save, tail

E2E = ("pekeris", "munk-lossless", "surface-duct", "gulf-adiabatic", "broadband", "elastic-sediment", "bellhop-munk-tl")
PRODUCTS = {".mod", ".shd", ".arr", ".ray"}
MAX_ARCHIVE_BYTES = 10_000_000
# Archive normalization replaces a home or output path only as a whole token. A token starts
# unless a longer path, URL host or expansion could continue into it; file:// or a joined option
# such as -I or -isystem may precede it (captured, since lookbehinds must be fixed-width). Letters,
# digits and the other characters in NAME continue a file name, `.`, `!` and `?` too unless they
# end a sentence; anything else ends the token. So siblings such as `user.backup` stay unchanged.
BOUNDARY = r"(?<![\w.~/\-\])}])"
PATH_START = r"(?:" + BOUNDARY + r"|(?<=file://)|" + BOUNDARY + r"(?P<option>--?[A-Za-z][\w-]*))"
NAME = r"\w.~\-+@%=#$^*(\[{!?"
PATH_END = r"(?=$|[^" + NAME + r"]|[.!?]+(?:$|[^" + NAME + r"/\\]))"


class Deadline(Exception):
    pass


class WorkloadError(RuntimeError):
    pass


def cases(tree):
    selected = {c["key"]: c for c in load(tree / "tools/perf/cases.json")}
    result = []
    for key in E2E:
        item = dict(selected[key])
        env = (tree / item["path"]).with_suffix(".env")
        files = [env.with_suffix("." + ext) for ext in ("env", "flp", "brc", "ssp", "trc", "irc", "sbp", "bty", "ati")]
        if not env.is_file() or (item["model"] != "bellhop" and not env.with_suffix(".flp").is_file()):
            raise RuntimeError("missing end-to-end input: " + str(env))
        item.update(env=env, files=[f for f in files if f.is_file()])
        result.append(item)
    return result


def interleave(blocks, labels, seed):
    rng = random.Random(seed)
    blocks = list(blocks)
    rng.shuffle(blocks)
    for block in blocks:
        order = sorted(labels)
        rng.shuffle(order)
        yield block, order


def deadline(kit):
    if kit.deadline is not None and time.monotonic() >= kit.deadline:
        raise Deadline("measurement deadline reached; rerun the same command to continue")


def quiet(kit, block):
    start = time.monotonic()
    limit = kit.a.max_load or max(2, len(kit.p["topology"]["logical"]) / 10)
    while True:
        deadline(kit)
        loadavg = os.getloadavg()
        row = dict(block=block, utc=time.time(), load=list(loadavg), ceiling=limit, waited_seconds=time.monotonic() - start)
        append(kit.out / "load.jsonl", row)
        if loadavg[0] <= limit:
            return row
        if time.monotonic() - start >= 600:
            raise RuntimeError("load stayed above ceiling for 10 minutes; no contaminated timing recorded")
        kit.state(f"waiting for load <= {limit:g} ({loadavg[0]:.2f})")
        time.sleep(5)


def hashes(directory):
    return {str(p.relative_to(directory)): dict(sha256=digest(p), bytes=p.stat().st_size)
            for p in sorted(directory.rglob("*")) if p.is_file()}


def finish_raw(kit, directory):
    if kit.a.keep_raw:
        directory.rename(directory.with_name(f"{directory.name}-{time.time_ns()}-hashed"))
    else:
        shutil.rmtree(directory)


def retire(kit, directory, name, retain=False, failed=False):
    if directory.exists():
        record = dict(block=name, files=hashes(directory))
        if failed:
            record["diagnostic_tails"] = {str(p.relative_to(directory)): tail(p) for p in directory.rglob("*")
                                          if p.is_file() and (p.suffix == ".log" or p.name == "results.jsonl")}
        append(kit.out / "output-hashes.jsonl", record)
        if not retain:
            finish_raw(kit, directory)


def shd_coverage(path, *, field=False):
    """Flatten supported rectilinear SHD shapes, ignoring FIELD's unset freq0."""
    try:
        data = path.read_bytes()
        record = 4 * struct.unpack_from("<i", data)[0]
        if record < 100 or len(data) < 10 * record or len(data) % record:
            raise ValueError("invalid record length or truncated file")
        plot = data[record:record + 10]
        # Pinned FIELD initializes its tag to ten spaces but writes only rectilinear pressure rows.
        if plot.rstrip(b" \0") != b"rectilin" and not (field and plot == b" " * 10):
            raise ValueError("unsupported SHD plot type (expected rectilin): " + repr(plot))
        dims = struct.unpack_from("<7i", data, 2 * record)
        if any(n <= 0 for n in dims) or dims[1:4] != (1, 1, 1):
            raise ValueError("invalid or unsupported geometry dimensions")
        if max(8 * dims[0], 4 * dims[4], 4 * dims[5], 8 * dims[6]) > record or len(data) // record != 10 + dims[0] * dims[4] * dims[5]:
            raise ValueError("frequency/geometry/pressure records do not cover the declared shape")
        frequencies = struct.unpack_from("<" + str(dims[0]) + "d", data, 3 * record)
        if not all(math.isfinite(f) for f in frequencies):
            raise ValueError("non-finite frequency vector")
        return [[f, *dims[1:]] for f in frequencies]
    except (OSError, ValueError, struct.error) as exc:
        raise WorkloadError(f"cannot parse SHD {path}: {exc}") from exc


def workload_products(root, products, model, stems=None, status_file=None, *, field=False):
    groups = {stem: {} for stem in stems} if stems is not None else {"": products}
    if stems is not None:
        for path, value in products.items():
            stem, sep, relative = path.partition("/")
            if not sep or stem not in groups:
                raise WorkloadError("unexpected sweep product: " + path)
            groups[stem][relative] = value
    coverage = {}
    for stem, files in groups.items():
        required = (".shd",) if model == "bellhop" else (".shd", ".mod")
        for suffix in required:
            matching = [v for p, v in files.items() if Path(p).suffix.lower() == suffix]
            if not matching or any(v["bytes"] == 0 for v in matching):
                raise WorkloadError(f"{stem or 'single case'}: missing/empty {suffix} products")
        coverage[stem] = sorted(entry for p in files if Path(p).suffix.lower() == ".shd"
                                for entry in shd_coverage(root / stem / p, field=field))
    if status_file is not None:
        try:
            statuses = [json.loads(line) for line in status_file.read_text().splitlines()]
            if (len(statuses) != len(stems) or {s["stem"] for s in statuses} != set(stems)
                    or any(s["outcome"] != "ok" or s["exit_code"] != 0 for s in statuses)):
                raise ValueError("expected exactly one ok status for every case")
        except (OSError, ValueError, KeyError, TypeError) as exc:
            raise WorkloadError(f"invalid run-many status file {status_file}: {exc}") from exc
    return groups, coverage


def copy_case(item, directory, stem):
    directory.mkdir(parents=True)
    for path in item["files"]:
        shutil.copyfile(path, directory / (stem + path.suffix))
    return directory / (stem + ".env")


def timed(kit, commands, cwd, log, stdin=None):
    start = time.perf_counter_ns()
    codes, logs = [], []
    try:
        for cmd in commands:
            process = subprocess.Popen(cmd, cwd=cwd, env=kit.env, stdin=stdin, stdout=subprocess.PIPE,
                                       stderr=subprocess.STDOUT, start_new_session=True, pass_fds=(kit.lock_fd,))
            try:
                # Pipes avoid wait(timeout)'s polling delay contaminating millisecond cases.
                logs.append(process.communicate(timeout=600)[0])
                codes.append(process.returncode)
            except BaseException:
                # Only our newly created process group (including xargs children).
                os.killpg(process.pid, signal.SIGKILL)
                logs.append(process.communicate()[0])
                raise
            if codes[-1]:
                break
        elapsed = time.perf_counter_ns() - start
    finally:
        log.write_bytes(b"".join(logs))
    return dict(wall_ns=elapsed, commands=commands, exit_codes=codes,
                status="ok" if len(codes) == len(commands) and not any(codes) else "process-error")


def cli_trial(kit, directory, item, label, build, workers, count, warmup, block):
    run = directory / label
    run.mkdir()
    fortran = build["kind"] == "fortran"
    sweep = workers is not None
    if fortran or sweep:
        paths = [copy_case(item, run / "inputs" / f"case-{i:04d}", f"case-{i:04d}") for i in range(count)]
    output = run / "out"
    commands = []
    stdin = None
    cwd = run
    if fortran:
        tree = Path(build["tree"])
        exe = tree / ("Bellhop/bellhop.exe" if item["model"] == "bellhop" else "Kraken/kraken.exe")
        field = tree / "KrakenField/field.exe"
        if sweep:
            listing = run / "directories.bin"
            listing.write_bytes(b"".join(os.fsencode(p.parent) + b"\0" for p in paths))
            stdin = listing.open("rb")
            script = 'set -eu; cd "$3"; stem=${3##*/}; "$1" "$stem"; "$2" "$stem"; test -s "$stem.mod"; test -s "$stem.shd"'
            commands = [["xargs", "-0", "-n", "1", "-P", str(workers), "sh", "-c", script, "sh", str(exe), str(field)]]
        else:
            cwd = paths[0].parent
            commands = [[str(exe), paths[0].stem]]
            if item["model"] != "bellhop":
                commands.append([str(field), paths[0].stem])
    else:
        action = "run-many" if sweep else "run"
        cmd = [build["binary"], item["model"], action]
        if sweep:
            listing = run / "cases.txt"
            listing.write_text("".join(str(p) + "\n" for p in paths))
            cmd += ["--list", str(listing), "--workers", str(workers)]
        else:
            cmd.append(str(item["env"]))
        cmd += ["-o", str(output), "--format", "legacy", "--threads", "1"]
        if label.endswith("-nosync"):
            cmd.append("--no-sync")
        commands = [cmd]
    deadline(kit)
    row = dict(kind="sweep" if sweep else "e2e", case=item["key"], variant=label, block=block,
               workers=workers or 1, threads=1, count=count, warmup=warmup, utc=time.time(),
               load_before=list(os.getloadavg()), affinity=sorted(os.sched_getaffinity(0)))
    try:
        row.update(timed(kit, commands, cwd, run / "process.log", stdin))
    finally:
        if stdin:
            stdin.close()
    row["load_after"] = list(os.getloadavg())
    root = (run / "inputs" if sweep else paths[0].parent) if fortran else output
    row["products"] = {p: v for p, v in hashes(root).items() if Path(p).suffix.lower() in PRODUCTS}
    row["process_status"] = row["status"]
    try:
        stems = [f"case-{i:04d}" for i in range(count)] if sweep else None
        groups, coverage = workload_products(root, row["products"], item["model"], stems,
                                             output / "status.ndjson" if sweep and not fortran else None,
                                             field=fortran and item["model"] == "kraken")
        row["coverage_by_case"] = coverage
        if sweep:
            row["case_products"] = groups
        expected = kit.coverage.get(item["key"])
        if sweep and expected is None:
            raise WorkloadError("single-case coverage must be checked before a sweep")
        if expected is not None and any(c != expected for c in coverage.values()):
            raise WorkloadError("frequency/geometry coverage differs from the single-case baseline")
        if not sweep and row["status"] == "ok":
            kit.coverage.setdefault(item["key"], coverage[""])
    except WorkloadError as exc:
        row.update(status="workload-error", workload_error=str(exc))
    if row["status"] != "ok":
        row["diagnostic_tail"] = tail(run / "process.log")
    return row


def driver_for(kit):
    tree = Path(kit.p["sources"][next(iter(kit.a.ref))]["tree"])
    driver = perf_driver(tree)
    # A surviving child retains the run lock if the supervisor is SIGKILLed.
    driver.subprocess = types.SimpleNamespace(**vars(subprocess))
    driver.subprocess.run = functools.partial(subprocess.run, pass_fds=(kit.lock_fd,))
    return driver


def solver_block(kit, directory, rows, round_number):
    driver = driver_for(kit)
    original = driver.invoke

    def invoke(*args, **kwargs):
        before = quiet(kit, "solver")
        row = original(*args, **kwargs)
        row.update(load_before=before, load_after=list(os.getloadavg()), affinity=sorted(os.sched_getaffinity(0)))
        return row

    driver.invoke = invoke
    output = directory / "solver.jsonl"
    args = ["bench", "--cases", str(kit.out / "solver-cases.json"), "--threads", "1", "--rounds", "1",
            "--reps", "1" if kit.a.quick else "3", "--warmup", "1", "--seed", str(kit.a.seed + round_number),
            "--output", str(output)]
    for label, build in sorted(kit.p["builds"].items()):
        if build["kind"] == "solver":
            args += ["--binary", label + "=" + build["binary"]]
    try:
        driver.bench(driver.parse_args(args))
    finally:
        if output.exists():
            for line in output.read_text().splitlines():
                row = json.loads(line)
                row.update(kind="solver", case=row["key"], workers=1, count=1, block=round_number)
                rows.append(row)


def identity_block(kit, directory, key):
    driver = driver_for(kit)
    original = driver.invoke

    def invoke(*args, **kwargs):
        deadline(kit)
        return original(*args, **kwargs)

    driver.invoke = invoke
    build = kit.p["builds"][key]
    ref, variant = build["ref"], build["variant"]
    output = directory / "check"
    args = ["check", "--binary", build["binary"], "--threads", "1", "--output", str(output)]
    if kit.a.quick:
        args += ["--cases", str(kit.out / "solver-cases.json")]
    if variant != "default":
        args += ["--baseline", str(directory.parent / f"{ref}-solver-default" / "check")]
    try:
        driver.check(driver.parse_args(args))
    except SystemExit as exc:
        if exc.code != 1 or not (output / "summary.json").exists():
            raise
    results = [json.loads(line) for line in (output / "results.jsonl").read_text().splitlines()]
    return [dict(ref=ref, variant=variant, summary=load(output / "summary.json"),
                 results=[{k: r[k] for k in ("key", "status", "snapshot_sha256", "error") if k in r} for r in results],
                 diagnostics=[{k: r[k] for k in ("key", "stderr", "stdout", "exit_code") if k in r}
                              for r in results if r.get("stderr") or r.get("stdout")])]


def cleanup_identity(kit):
    for directory in (kit.work / "identity").glob("*"):
        if directory.name.endswith("-hashed"):
            continue
        marker = kit.out / "records" / ("identity-" + directory.name + ".json")
        if marker.exists() and load(marker)["complete"]:
            partner = kit.out / "records" / ("identity-" + directory.name.removesuffix("default") + "v3.json")
            if (directory.name.endswith("-default") and directory.name.removesuffix("default") + "v3" in kit.p["builds"]
                    and not (partner.exists() and load(partner)["complete"])
                    and not identity_summary([], load(marker)["identity"])["failed"]):
                continue  # The completed default snapshots are still needed by v3 on resume.
            # Completed identity artifacts were hashed before their checkpoint was published.
            finish_raw(kit, directory)
        else:
            retire(kit, directory, "interrupted-identity:" + directory.name, failed=True)


def measure(kit):
    kit.deadline = time.monotonic() + kit.a.deadline_minutes * 60 if kit.a.deadline_minutes else None
    kit.tree = Path(kit.p["sources"][next(iter(kit.a.ref))]["tree"])
    items = cases(kit.tree)
    topo = kit.p["topology"]
    original_affinity = os.sched_getaffinity(0)
    original_environment = dict(os.environ)
    os.environ.clear()
    os.environ.update(kit.env)
    records = kit.out / "records"
    records.mkdir(exist_ok=True)
    raw = kit.work / "raw"
    raw.mkdir(exist_ok=True)
    tasks = [("identity-" + key, "identity", key, None, 1, False) for ref in kit.a.ref
             for variant in ("default", "v3") for key in [f"{ref}-solver-{variant}"] if key in kit.p["builds"]]
    tasks += [(f"solver-{r}", "solver", r, None, 1, False) for r in range(1 if kit.a.quick else 7)]
    blocks = [(r, item, None, 1, r == -1) for r in range(-1, 1 if kit.a.quick else 15) for item in items]
    for r in range(-1, 1 if kit.a.quick else 3):
        for item in items:
            if item["key"] in ("pekeris", "surface-duct"):
                for workers in sorted({1, len(topo["physical"]), len(topo["logical"])}):
                    blocks.append((r, item, workers, 4 if kit.a.quick else 200 if item["key"] == "pekeris" else 50, r == -1))
    # Establish single-case coverage before sweep warmups; then mix measured blocks.
    shuffled = []
    for offset, group in enumerate(([b for b in blocks if b[4] and b[2] is None],
                                    [b for b in blocks if b[4] and b[2] is not None], [b for b in blocks if not b[4]])):
        shuffled += list(interleave(group, [], kit.a.seed + offset))
    for (r, item, workers, count, warmup), _ in shuffled:
        name = f"{'sweep' if workers else 'e2e'}-{item['key']}-w{workers or 1}-r{r}"
        tasks.append((name, item, r, workers, count, warmup))
    save(kit.out / "plan.json", [t[0] for t in tasks])
    previous = [record for p in sorted(records.glob("*.json")) for record in [load(p)] if record["complete"]]
    rows = [row for record in previous for row in record["rows"]]
    identities = [s for record in previous for s in record["identity"]]
    kit.coverage = {row["case"]: row["coverage_by_case"][""] for row in rows
                    if row["kind"] == "e2e" and row["status"] == "ok" and "coverage_by_case" in row}
    try:
        if identity_summary(rows, identities)["failed"]:
            raise RuntimeError("a completed checkpoint contains a within-ref identity failure")
        cleanup_identity(kit)
        for index, (name, item, r, workers, count, warmup) in enumerate(tasks):
            record_path = records / (name + ".json")
            if record_path.exists() and load(record_path)["complete"]:
                continue
            deadline(kit)
            kit.state(f"block {index + 1}/{len(tasks)} {name}")
            cpus = topo["logical"] if workers == len(topo["logical"]) else topo["physical"] if workers and workers > 1 else topo["physical"][:1]
            os.sched_setaffinity(0, cpus)
            quiet(kit, name)
            # Unique attempts preserve --keep-raw and make interrupted output unambiguous.
            directory = kit.work / "identity" / r if item == "identity" else raw / (name + "-" + str(time.time_ns()))
            directory.mkdir(parents=True)
            record = dict(complete=False, rows=[], identity=[])
            try:
                if item == "solver":
                    solver_block(kit, directory, record["rows"], r)
                elif item == "identity":
                    record["identity"] = identity_block(kit, directory, r)
                else:
                    variants = {}
                    for label, build in kit.p["builds"].items():
                        if build["kind"] in ("cli", "fortran"):
                            variants[label] = build
                            action = item["model"] + ("/run-many" if workers else "/run")
                            if build.get("no_sync", {}).get(action):
                                variants[label + "-nosync"] = build
                    order = next(interleave([None], variants, kit.a.seed + index))[1]
                    for label in order:
                        row = cli_trial(kit, directory, item, label, variants[label], workers, count, warmup, r)
                        record["rows"].append(row)
                        if row["status"] != "ok":
                            raise RuntimeError(f"{name}/{label}: {row['status']}: {row.get('workload_error', 'see diagnostic tails')}")
                record["complete"] = True
            finally:
                retire(kit, directory, name, retain=item == "identity" and record["complete"], failed=not record["complete"])
                save(record_path, record)
                if item == "identity":
                    cleanup_identity(kit)
            rows.extend(record["rows"])
            identities.extend(record["identity"])
            if identity_summary(rows, identities)["failed"]:
                raise RuntimeError("within-ref identity mismatch or unexpected solver outcome; no further blocks run")
    finally:
        os.sched_setaffinity(0, original_affinity)
        os.environ.clear()
        os.environ.update(original_environment)


def aggregate(rows):
    groups = collections.defaultdict(list)
    for row in rows:
        if row["status"] != "ok" or row.get("warmup") is True:
            continue
        key = (row["kind"], row["case"], row["workers"], row["variant"])
        groups[key].extend(n / 1e6 / row["count"] for n in row.get("samples_ns", [row.get("wall_ns")]))
    medians = {key: statistics.median(values) for key, values in groups.items()}
    return [dict(kind=k[0], case=k[1], workers=k[2], variant=k[3], n=len(v), median_ms=medians[k],
                 min_ms=min(v), max_ms=max(v), ratio_fortran_O3=medians[k] / medians[k[:3] + ("fortran-O3",)]
                 if k[:3] + ("fortran-O3",) in medians else None) for k, v in sorted(groups.items())]


def identity_summary(rows, solver):
    results, products = [], {}
    for row in rows:
        if row["kind"] in ("e2e", "sweep") and row["status"] == "ok" and "-cli-" in row["variant"]:
            groups = row["case_products"] if row["kind"] == "sweep" else {"": row["products"]}
            for stem, files in groups.items():
                key = (row["kind"], row["case"], row["workers"], stem, row["variant"])
                products.setdefault(key, []).append(files)
    refs = list(dict.fromkeys([s["ref"] for s in solver] + [k[-1].rsplit("-cli-", 1)[0] for k in products]))
    for key, samples in sorted(products.items()):
        kind, case, workers, stem, variant = key
        ref = variant.rsplit("-cli-", 1)[0]
        baseline = products.get(key[:-1] + (ref + "-cli-default",))
        result = dict(kind=kind, case=case, workers=workers, sweep_case=stem or None, variant=variant)
        results.append(dict(result, scope="within-ref-cli", identical=all(p == baseline[0] for p in samples) if baseline else None))
        if variant.endswith("-cli-default") and ref != refs[0]:
            base = products.get(key[:-1] + (refs[0] + "-cli-default",))
            results.append(dict(result, scope="between-ref-cli", identical=samples[0] == base[0] if base else None))
    defaults = [s for s in solver if s["variant"] == "default"]
    for s in defaults[1:]:
        results.append(dict(scope="between-ref-solver", ref=s["ref"], identical=s["results"] == defaults[0]["results"]))
    failed = any(r["scope"].startswith("within") and r["identical"] is False for r in results)
    failed |= any(s["summary"]["mismatches"] or s["summary"]["unexpected_outcomes"] for s in solver)
    return dict(failed=bool(failed), comparisons=results, solver=solver)


def workload_summary(rows):
    results = []
    for case in E2E:
        trials = [r for r in rows if r["case"] == case and r["kind"] in ("e2e", "sweep")]
        valid = [r for r in trials if r["status"] == "ok"]
        single = [r for r in valid if r["kind"] == "e2e"]
        sweeps = [r for r in valid if r["kind"] == "sweep"]
        failures = [{k: r[k] for k in ("kind", "variant", "workers", "workload_error")} for r in trials if r["status"] == "workload-error"]
        results.append(dict(case=case, status="failed" if failures else "checked" if valid else "not-run",
                            coverage=single[0]["coverage_by_case"][""] if single else None,
                            single_trials=len(single), sweep_trials=len(sweeps), sweep_cases=sum(r["count"] for r in sweeps),
                            variants=sorted({r["variant"] for r in valid}), failures=failures))
    return results


def archive_value(kit, value, diagnostic=False):
    if isinstance(value, dict):
        normalized = {}
        for key, item in value.items():
            name = archive_value(kit, key)
            if name in normalized:
                raise ValueError("archive path normalization would merge dictionary keys: " + repr(name))
            normalized[name] = archive_value(kit, item, diagnostic or key in
                    ("error", "stderr", "stdout", "diagnostic_tail", "diagnostic_tails", "workload_error"))
        return normalized
    if isinstance(value, list):
        return [archive_value(kit, v, diagnostic) for v in value]
    if isinstance(value, str):
        roots = {str(p): marker for p, marker in ((Path.home(), "~"), (kit.out, "<out>"))
                 if p.is_absolute() and p != Path(p.anchor)}
        if roots:
            # Match whole path prefixes once; never rewrite URLs, sibling names or replacement text.
            prefixes = "|".join(re.escape(p) for p in sorted(roots, key=len, reverse=True))
            value = re.sub(PATH_START + "(?P<root>" + prefixes + ")" + PATH_END,
                           lambda m: (m["option"] or "") + roots[m["root"]], value)
        if diagnostic:
            for key in network_names(kit.env):
                if kit.env[key]:
                    value = value.replace(kit.env[key], "<" + key + ">")
            value = value.encode()[-4096:].decode(errors="replace")
    return value


def archive_content(kit, path):
    if path.suffix == ".json":
        value = load(path)
        if path.name == "machine.json":
            value = {k: v for k, v in value.items() if k not in ("mounts", "boot_id", "hostname")}
            words = value.get("kernel", "").split()
            value["kernel"] = " ".join(words[:1] + words[2:])  # uname -a's second word is the hostname.
        return (json.dumps(archive_value(kit, value), indent=2, sort_keys=True) + "\n").encode()
    if path.suffix == ".jsonl":
        return "".join(json.dumps(archive_value(kit, json.loads(line)), sort_keys=True) + "\n"
                       for line in path.read_text().splitlines()).encode()
    text = path.read_text()
    if path.name == "summary.md":
        error = load(kit.out / "summary.json").get("error")
        if error:
            text = text.replace(error, archive_value(kit, error, diagnostic=True))
    return archive_value(kit, text).encode()


def summarize(kit, outcome, error):
    # Also hash leftovers from a killed process before deleting them on resume.
    for directory in (kit.work / "raw").glob("*"):
        if not directory.name.endswith("-hashed"):
            retire(kit, directory, "interrupted-attempt:" + directory.name, failed=True)
    cleanup_identity(kit)
    records = [load(p) for p in sorted((kit.out / "records").glob("*.json"))]
    rows = [row for record in records for row in record["rows"]]
    identities = identity_summary(rows, [s for record in records for s in record["identity"]])
    if identities["failed"]:
        outcome = "failed"
        error = (error + "; " if error else "") + "within-ref identity mismatch or unexpected solver outcome"
    workloads = workload_summary(rows)
    if any(w["failures"] for w in workloads):
        outcome = "failed"
        error = (error + "; " if error else "") + "workload validation failed; see workloads.json"
    plan = load(kit.out / "plan.json") if (kit.out / "plan.json").exists() else []
    completed = sum(r["complete"] for r in records)
    if outcome == "complete" and (not plan or completed != len(plan) or not (kit.out / "measure.done.json").exists()):
        outcome = "partial"
    summary = dict(status=outcome, error=error, completed_blocks=completed, planned_blocks=len(plan),
                   seed=kit.a.seed, quick=kit.a.quick, topology=kit.p.get("topology"), timings=aggregate(rows),
                   note="ms per case for sweeps; solver-only has no Fortran end-to-end denominator; ratios are ratios of medians")
    save(kit.out / "summary.json", summary)
    save(kit.out / "identity.json", identities)
    save(kit.out / "workloads.json", workloads)
    with (kit.out / "timings.jsonl").open("w") as f:
        for row in rows:
            f.write(json.dumps(row, sort_keys=True) + "\n")
    text = ["# Pelagic x86 measurements", "", "**Status: " + outcome + "**", "", error or "All planned blocks finished.", "",
            f"Blocks: {completed}/{len(plan)}. Seed: {kit.a.seed}. Quick: {kit.a.quick}.", "",
            "Times are milliseconds (sweeps: per case). P/F is median / Fortran O3 median at the same worker count; lower is better.",
            "Solver-only times exclude I/O and have no comparable Fortran denominator. Ranges are observed min–max, not confidence intervals.", "",
            "| Scope | Case | W | Variant | n | Median [min–max] ms | P/F |", "|---|---|---:|---|---:|---:|---:|"]
    for row in summary["timings"]:
        ratio = "—" if row["ratio_fortran_O3"] is None else f"{row['ratio_fortran_O3']:.3f}"
        text.append(f"| {row['kind']} | {row['case']} | {row['workers']} | {row['variant']} | {row['n']} | {row['median_ms']:.3f} [{row['min_ms']:.3f}–{row['max_ms']:.3f}] | {ratio} |")
    text += ["", "## Workload shapes", "", "Frequency vectors and geometry dimensions, not nominal freq0 or file counts. See workloads.json.",
             "", "| Case | Result | Single trials | Sweep trials / cases |", "|---|---|---:|---:|"]
    text += [f"| {w['case']} | {w['status']} | {w['single_trials']} | {w['sweep_trials']} / {w['sweep_cases']} |" for w in workloads]
    text += ["", "## Identity", "", f"Within-ref failure: {identities['failed']}. See identity.json for per-case comparisons and solver diagnostics.",
             "Cross-ref differences are data, not a failure: acceptance, not identity, gates numerical changes.",
             "Operational metadata is hashed but not used as a numerical identity test. Fortran whole-file hashes may vary in unset headers.", ""]
    (kit.out / "summary.md").write_text("\n".join(text))
    archive = kit.out / "results.tar.gz"
    temporary = kit.out / "results.tar.gz.tmp"
    names = ("summary.md", "summary.json", "timings.jsonl", "identity.json", "workloads.json", "provenance.json", "machine.json", "config.json",
             "solver-cases.json", "plan.json", "commands.jsonl", "load.jsonl", "output-hashes.jsonl")
    with tarfile.open(temporary, "w:gz") as tar:
        for name in names:
            path = kit.out / name
            if path.exists():
                content = archive_content(kit, path)
                entry = tarfile.TarInfo(name)
                entry.size, entry.mode = len(content), 0o644
                entry.uid = entry.gid = 0
                entry.uname = entry.gname = ""
                tar.addfile(entry, io.BytesIO(content))
    if temporary.stat().st_size > MAX_ARCHIVE_BYTES:
        temporary.unlink()
        for path in (archive, kit.out / "archive.json"):
            if path.exists():
                path.unlink()
        summary.update(status="failed", error="summary archive exceeds 10 MB; no oversized archive published")
        save(kit.out / "summary.json", summary)
        (kit.out / "summary.md").write_text("# Failed packaging\n\n" + summary["error"] + "\n")
        raise RuntimeError(summary["error"])
    temporary.replace(archive)
    save(kit.out / "archive.json", dict(bytes=archive.stat().st_size, sha256=digest(archive), status=outcome))
    return outcome

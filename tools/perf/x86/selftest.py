"""Synthetic regression tests: no compiler, network or external process required."""
import contextlib
import io
import json
import os
from pathlib import Path
import struct
import tarfile
import tempfile
from types import SimpleNamespace
from unittest.mock import patch

from run import (FLAGS, Kit, append, check_cargo_config, check_cmake, command, compiler_inventory, load, parse_args, save,
                 topology, unpack)
from measure import (Deadline, E2E, WorkloadError, aggregate, archive_value, cases, cli_trial, identity_summary, interleave,
                     measure, shd_coverage, summarize)

SHA = "7ac64e127ab103ea6123037cc3945e53eae376cc"
PRODUCTS = {"result.shd": {"sha256": "same", "bytes": 4}}
ROW = dict(kind="e2e", case="pekeris", workers=1, count=1, status="ok", warmup=False, wall_ns=1_000_000,
           products=PRODUCTS, coverage_by_case={"": [[50.0, 1, 1, 1, 1, 1, 1]]})


def kit_at(out):
    out.mkdir(parents=True, exist_ok=True)
    kit = Kit(parse_args(["--ref", "base=" + SHA, "--out", str(out), "--quick"]), -1)
    kit.state = lambda *args: None
    kit.p.update(sources={"base": dict(tree=str(out), toolchain="1.99.0", commit=SHA)},
                 topology=dict(physical=[0], logical=[0, 1], v3=True))
    return kit


def shd(frequencies, depths=1, sources=1, plot=b"rectilin  "):
    record = max(128, 8 * len(frequencies))
    data = bytearray(record * (10 + len(frequencies) * sources * depths))
    struct.pack_into("<i", data, 0, record // 4)
    data[record:record + 10] = plot.ljust(10, b" ")
    struct.pack_into("<7i", data, 2 * record, len(frequencies), 1, 1, 1, sources, depths, 1)
    struct.pack_into("<d", data, 2 * record + 28, float("nan"))  # Unset freq0 is not coverage.
    struct.pack_into("<" + str(len(frequencies)) + "d", data, 3 * record, *frequencies)
    return bytes(data)


def test_core(out):
    args = parse_args(["--ref", "base=" + SHA, "--ref", "probe=pr:123", "--out", "results"])
    assert args.ref == {"base": SHA, "probe": "pr:123"}
    assert parse_args(["status", "--out", "results"]).action == "status"
    for invalid in (["--ref", "x=pr:0"], ["--ref", "../x=main"], ["--ref", "x=main", "--ref", "x=main"],
                    ["--ref", "x=main", "--deadline-minutes", "nan"], ["--ref", "x=main", "--max-load", "0"],
                    ["--quick", "--ref", "x=main", "--ref", "y=main"], []):
        with contextlib.redirect_stderr(io.StringIO()):
            try:
                parse_args(["--out", "results", *invalid])
            except SystemExit as exc:
                assert exc.code == 2
            else:
                raise AssertionError("invalid arguments accepted")
    check_cmake("cmake version 3.26.0")
    check_cmake("cmake version 4.1.0")
    try:
        check_cmake("cmake version 3.25.1")
    except RuntimeError as exc:
        assert "3.26" in str(exc)
    else:
        raise AssertionError("old CMake accepted")
    order = list(interleave(range(12), ["base", "v3", "fortran"], 42))
    assert order == list(interleave(range(12), ["v3", "fortran", "base"], 42))
    assert order != list(interleave(range(12), ["base", "v3", "fortran"], 43))
    assert sorted(b for b, _ in order) == list(range(12))
    assert all(sorted(labels) == ["base", "fortran", "v3"] for _, labels in order)
    rows = [dict(ROW, variant=v, wall_ns=n * 1_000_000) for v, samples in
            (("fortran-O3", [1, 3, 5]), ("base-cli-default", [2, 6, 10])) for n in samples]
    rows += [dict(rows[-1], warmup=True, wall_ns=999_000_000), dict(rows[-1], status="workload-error")]
    candidate = next(r for r in aggregate(rows) if r["variant"] == "base-cli-default")
    assert (candidate["n"], candidate["median_ms"], candidate["min_ms"], candidate["max_ms"], candidate["ratio_fortran_O3"]) == (3, 6, 2, 10, 2)
    assert next(r for r in aggregate([dict(r, kind="sweep", count=2) for r in rows]) if r["variant"] == "base-cli-default")["median_ms"] == 3
    solver = aggregate([dict(ROW, kind="solver", variant="base", samples_ns=[1_000_000, 3_000_000], warmup=1)])
    assert solver[0]["median_ms"] == 2 and solver[0]["ratio_fortran_O3"] is None
    assert FLAGS == {"default": "", "v3": "-C target-cpu=x86-64-v3", "cgu1": "-C codegen-units=1"}


def test_safety(out):
    flags = "avx avx2 bmi1 bmi2 f16c fma movbe xsave pni ssse3 sse4_1 sse4_2 popcnt cx16 lahf_lm abm"
    def cpu_file(path):
        return "flags : " + flags if str(path) == "/proc/cpuinfo" else "0"
    with patch("run.Path.read_text", cpu_file), patch("run.platform.machine", return_value="x86_64"), \
            patch("os.sched_getaffinity", return_value={1, 3}, create=True):
        topo = topology()
        assert topo["v3"] and topo["physical"] == [1] and topo["logical"] == [1, 3]
        for missing in ("avx2", "pni"):
            saved = flags
            flags = flags.replace(missing + " ", "")
            assert missing in topology()["v3_skip_reason"]
            flags = saved
    save(out / "atomic.json", {"value": 1})
    save(out / "atomic.json", {"value": 2})
    assert load(out / "atomic.json") == {"value": 2}
    for name, kind in (("top/../../escape", None), ("/absolute", None), ("top/link", tarfile.SYMTYPE), ("top/device", tarfile.CHRTYPE)):
        archive = out / "unsafe.tar"
        with tarfile.open(archive, "w") as tar:
            entry = tarfile.TarInfo(name)
            if kind:
                entry.type = kind
            tar.addfile(entry)
        try:
            unpack(archive, out / "extract")
        except ValueError:
            pass
        else:
            raise AssertionError("unsafe tar accepted")


def test_workloads(out):
    kit = kit_at(out)
    env = out / "broadband.env"
    env.write_text("synthetic input")
    item = dict(key="broadband", model="kraken", env=env, files=[env])
    def trial(label, files, kind="cli", workers=None, count=1, status="ok", model="kraken"):
        def fake_timed(kit, commands, cwd, log, stdin=None):
            root = log.parent / ("inputs/case-0000" if kind == "fortran" else "out")
            root.mkdir(parents=True, exist_ok=True)
            for name, payload in files.items():
                path = root / name
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_bytes(payload)
            if workers is not None:
                (root / "status.ndjson").write_text("".join(json.dumps(dict(stem=f"case-{i:04d}", outcome=status, exit_code=0)) + "\n" for i in range(count)))
            log.write_text("synthetic successful process")
            return dict(status="ok", wall_ns=1_000_000, exit_codes=[0] * len(commands), commands=commands)
        with patch("measure.timed", fake_timed), patch("os.sched_getaffinity", return_value={0}, create=True):
            return cli_trial(kit, out, dict(item, model=model), label, dict(kind=kind, binary="pelagic", tree="/reference"), workers, count, False, 0)
    flattened = {f"f{f}.{suffix}": shd([f]) if suffix == "shd" else b"mod" for f in range(50, 151) for suffix in ("shd", "mod")}
    grouped = {"all.shd": shd(list(range(50, 151))), "all.mod": b"mod"}
    assert trial("old-layout", flattened)["status"] == "ok"
    assert trial("new-layout", grouped)["status"] == "ok"
    assert trial("fortran", grouped, "fortran")["status"] == "ok"
    blank = dict(grouped, **{"all.shd": shd(list(range(50, 151)), plot=b" " * 10)})
    assert trial("blank-field", blank, "fortran")["status"] == "ok"
    assert trial("blank-cli", blank)["status"] == "workload-error"
    assert trial("blank-bellhop", blank, "fortran", model="bellhop")["status"] == "workload-error"
    assert trial("nul-field", dict(grouped, **{"all.shd": shd([50], plot=b"\0" * 10)}), "fortran")["status"] == "workload-error"
    for label, payload in (("one-frequency", shd([50])), ("wrong-geometry", shd(list(range(50, 151)), depths=2)), ("truncated", b"x")):
        row = trial(label, {"r.shd": payload, "r.mod": b"mod"}, "fortran")
        assert row["status"] == "workload-error" and row["process_status"] == "ok"
        assert row["workload_error"] and aggregate([row]) == []
    valid_sweep = {f"case-{i:04d}/{name}": data for i in range(4) for name, data in grouped.items()}
    good = trial("good-sweep", valid_sweep, workers=4, count=4)
    assert good["status"] == "ok" and len(good["coverage_by_case"]) == 4
    missing = {p: b for p, b in valid_sweep.items() if not p.startswith("case-0003/")}
    assert trial("missing-case", missing, workers=4, count=4)["status"] == "workload-error"
    concentrated = {f"case-0000/{i}-{name}": data for i in range(4) for name, data in grouped.items()}
    assert trial("concentrated", concentrated, workers=4, count=4)["status"] == "workload-error"
    assert trial("bad-status", valid_sweep, workers=4, count=4, status="failed")["status"] == "workload-error"
    partial = dict(valid_sweep, **{"case-0003/all.shd": shd([50])})
    assert trial("incomplete-case", partial, workers=4, count=4)["status"] == "workload-error"
    extra = dict(grouped, **{"broken.shd": b"x"})
    bad = trial("extra-malformed", extra)
    assert bad["status"] == "workload-error"
    save(out / "records/probe.json", dict(complete=True, rows=[bad], identity=[]))
    save(out / "plan.json", ["probe"])
    save(out / "measure.done.json", {})
    assert summarize(kit, "complete", None) == "failed"
    assert next(w for w in load(out / "workloads.json") if w["case"] == "broadband")["status"] == "failed"


def test_shd_headers(out):
    path = out / "test.shd"
    for sources, depths in ((32, 1), (1, 32)):
        path.write_bytes(shd([50], sources=sources, depths=depths))
        assert shd_coverage(path) == [[50.0, 1, 1, 1, sources, depths, 1]]
    irregular = bytearray(shd([50], plot=b"irregular"))
    struct.pack_into("<7i", irregular, 256, 1, 1, 1, 1, 1, 3, 3)  # Irregular has one pressure row, not three.
    for payload, message in ((shd([50], sources=33), "declared shape"),
                             (shd([50], depths=33), "declared shape"),
                             (shd([50], plot=b"garbage"), "plot type"), (irregular, "plot type"),
                             (shd([50], plot=b"\0" * 10), "plot type")):
        path.write_bytes(payload)
        for field in (False, True):
            try:
                shd_coverage(path, field=field)
            except WorkloadError as exc:
                assert message in str(exc), str(exc)
            else:
                raise AssertionError("invalid SHD header accepted")


def test_identity(out):
    same = [dict(ROW, variant="base-cli-" + v) for v in ("default", "v3", "cgu1", "default-nosync")]
    assert not identity_summary(same, [])["failed"]
    assert identity_summary(same + [dict(same[-1], products={})], [])["failed"]
    sweeps = [dict(r, kind="sweep", workers=2, count=2, case_products={"case-0000": PRODUCTS, "case-0001": PRODUCTS}) for r in same]
    assert len(identity_summary(sweeps, [])["comparisons"]) == 8
    bad = dict(sweeps[-1], case_products={"case-0000": PRODUCTS, "case-0001": {"r.shd": {"sha256": "different", "bytes": 4}}})
    assert identity_summary(sweeps + [bad], [])["failed"]
    other_w = [dict(r, workers=4, case_products=bad["case_products"]) for r in sweeps]
    assert not identity_summary(sweeps + other_w, [])["failed"], "compared different W"
    cross = identity_summary(sweeps + [dict(bad, variant="other-cli-default"), dict(bad, variant="fortran-O3")], [])
    assert not cross["failed"] and any(r["scope"] == "between-ref-cli" and r["identical"] is False for r in cross["comparisons"])
    kit = kit_at(out)
    save(out / "records/probe.json", dict(complete=True, rows=sweeps + [bad], identity=[]))
    save(out / "plan.json", ["probe"])
    save(out / "measure.done.json", {})
    assert summarize(kit, "complete", None) == "failed", "sweep mismatch did not fail the run"


def test_environment(out):
    controls = ("RUSTFLAGS", "RUSTDOCFLAGS", "RUSTC", "RUSTC_WRAPPER", "RUSTC_WORKSPACE_WRAPPER", "CARGO_ENCODED_RUSTFLAGS",
                "CARGO_BUILD_RUSTFLAGS", "CARGO_BUILD_RUSTC", "CARGO_BUILD_RUSTC_WRAPPER", "CARGO_BUILD_TARGET",
                "CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUSTFLAGS", "CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER", "CARGO_TARGET_DIR", "CARGO_PROFILE_RELEASE_OPT_LEVEL", "CARGO_PROFILE_DEV_DEBUG",
                "CFLAGS", "CXXFLAGS", "CPPFLAGS", "FFLAGS", "FCFLAGS", "LDFLAGS", "CMAKE_TOOLCHAIN_FILE", "FC", "AR")
    network = {k: "secret-" + k for k in ("https_proxy", "NO_PROXY", "CARGO_HTTP_PROXY", "CARGO_NET_GIT_FETCH_WITH_CLI", "CARGO_REGISTRIES_PRIVATE_TOKEN")}
    ambient = dict.fromkeys(controls, "injected")
    ambient.update(network, PATH="/module/bin", LD_LIBRARY_PATH="/module/lib", RUSTUP_HOME="/shared/rustup", CARGO_HOME="/shared/cargo", CARGO_INCREMENTAL="1", CC="gcc", CXX="g++")
    with patch.dict(os.environ, ambient, clear=True):
        kit = kit_at(out)
    assert all(k not in kit.env for k in controls)
    assert kit.env["CC"] == "gcc" and kit.env["CXX"] == "g++"
    assert all(kit.env[k] == v for k, v in network.items())
    assert kit.env["PATH"].endswith("/module/bin") and kit.env["LD_LIBRARY_PATH"] == "/module/lib"
    assert kit.env["CARGO_INCREMENTAL"] == "0" and kit.env["RUSTUP_HOME"] == "/shared/rustup"
    assert all(Path(kit.env[k]).is_relative_to(out) for k in ("CARGO_HOME", "TMPDIR"))
    with patch("run.subprocess.check_output", return_value="metadata\n") as child:
        assert command(["probe"], env=kit.env) == "metadata"
        assert child.call_args.kwargs["env"]["TMPDIR"] == str(kit.work / "tmp")
        try:
            command(["probe"])
        except TypeError:
            pass
        else:
            raise AssertionError("metadata command accepted an implicit ambient environment")
    cargo = Path(kit.env["CARGO_HOME"])
    tree = out / "source/nested"
    tree.mkdir(parents=True)
    exists, symlink = Path.exists, Path.is_symlink
    with patch("run.Path.exists", lambda p: exists(p) if p.is_relative_to(out) else False), \
            patch("run.Path.is_symlink", lambda p: symlink(p) if p.is_relative_to(out) else False):
        check_cargo_config(tree, cargo)
        for base in (tree / ".cargo", out / ".cargo", cargo):
            base.mkdir(exist_ok=True)
            for name in ("config", "config.toml"):
                config = base / name
                config.write_text('[build]\nrustflags = ["-C", "target-cpu=native"]\n')
                try:
                    check_cargo_config(tree, cargo)
                except RuntimeError as exc:
                    assert str(config) in str(exc)
                    if base == tree / ".cargo":
                        assert "refs with their own Cargo config" in str(exc) and "not supported" in str(exc)
                        assert "move" not in str(exc) and "pinned ref" in str(exc)
                    else:
                        assert "move --out" in str(exc) and "move the config" in str(exc)
                else:
                    raise AssertionError("Cargo config accepted")
                config.unlink()
        captured = []
        kit.p["builds"] = {"fortran-" + opt: dict(binaries={}) for opt in ("O3", "O1")}
        def fake_run(cmd, cwd=None, env=None):
            captured.append(env)
            binary = Path(env["CARGO_TARGET_DIR"]) / "release" / ("pelagic" if "cli" in cmd else "pelagic-perf")
            binary.parent.mkdir(parents=True)
            binary.write_bytes(b"synthetic binary")
            assert cmd[:4] == ["cargo", "build", "--release", "--locked"]
        def fake_command(cmd, **kwargs):
            return "--no-sync" if cmd[-1] == "--help" else "cmake version 3.31.10" if cmd[0] == "cmake" else "cargo 1.99.0" if cmd[0] == "cargo" else "rustc 1.99.0 (synthetic)\nhost: x86_64-unknown-linux-gnu"
        with patch("run.command", fake_command), patch("run.shutil.which", return_value="/module/bin/tool"), patch.object(kit, "run", fake_run):
            kit.build()
    assert len(captured) == 5 and len({e["CARGO_TARGET_DIR"] for e in captured}) == 5
    assert all(e["CARGO_HOME"] == str(cargo) and e["CARGO_INCREMENTAL"] == "0" for e in captured)
    assert all(e["CC"] == "gcc" and e["CXX"] == "g++" and e["CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER"] == "/module/bin/tool" for e in captured)
    assert all(b["linker"] == "/module/bin/tool" for b in kit.p["builds"].values() if b.get("kind") in ("cli", "solver"))
    assert kit.p["environment"]["network_names"] == sorted(network)
    assert all(v not in json.dumps(kit.p) for v in network.values())


def test_compilers(out):
    class PreflightComplete(Exception):
        pass
    for index, selectors in enumerate(({}, {"CC": "gcc", "CXX": "g++"}, {"CC": "/module/bin/gcc", "CXX": "/module/bin/g++"})):
        with patch.dict(os.environ, dict(PATH="/module/bin", **selectors), clear=True):
            kit = kit_at(out / str(index))
        selected = [selectors.get("CC", "cc"), selectors.get("CXX", "c++")]
        checked, queried = [], []
        def which(name, path=None):
            assert path == kit.env["PATH"]
            checked.append(name)
            return None if selectors and name in ("cc", "c++") else "/module/bin/" + Path(name).name
        def metadata(cmd, **kwargs):
            queried.append(cmd)
            return "cmake version 3.31.10" if cmd[0] == "cmake" else cmd[0] + " version synthetic"
        def save_machine(path, value):
            assert path.name == "machine.json"
            assert value["cc"] == selected[0] + " version synthetic" and value["c++"] == selected[1] + " version synthetic"
            assert value["executables"]["cc"] == "/module/bin/" + Path(selected[0]).name
            assert value["executables"]["c++"] == "/module/bin/" + Path(selected[1]).name
            raise PreflightComplete()
        with patch("run.shutil.which", which), patch("run.command", metadata), \
                patch("run.topology", return_value={}), patch("run.save", save_machine):
            try:
                kit.setup()
            except PreflightComplete:
                pass
            else:
                raise AssertionError("setup did not reach machine provenance")
        assert all(name in checked and [name, "--version"] in queried for name in selected)
        assert not selectors or not any(name in checked for name in ("cc", "c++"))
    # A compiler that answered --version but has since vanished fails loudly instead of recording None.
    with patch("run.command", return_value="synthetic version"), \
            patch("run.shutil.which", lambda name, path=None: None if name == "gcc" else "/module/bin/" + name):
        try:
            compiler_inventory(dict(PATH="/module/bin", CC="gcc"))
        except RuntimeError as exc:
            assert "no longer executable" in str(exc) and str(exc).endswith(": cc"), str(exc)
        else:
            raise AssertionError("vanished compiler accepted")


def test_make_environment(out):
    poison = out / "site.mk"
    poison.write_text("override FC := synthetic-broken-compiler\noverride FFLAGS := -O0\n")
    controls = dict(MAKEFILES=str(poison), GNUMAKEFLAGS="-e", MAKEFLAGS="-e", MFLAGS="-e", FC="wrong-compiler", FFLAGS="-O0")
    with patch.dict(os.environ, controls):
        kit = kit_at(out)
    kit.p.update(sources={}, fortran=dict(archive=str(out / "unused.tar")))
    captured = []
    def fake_unpack(archive, tree):
        for directory in ("misc", "Kraken", "KrakenField", "Bellhop"):
            (tree / directory).mkdir(parents=True)
    def fake_run(cmd, cwd=None, env=None):
        env = env or kit.env
        assert not set(controls).intersection(env), "inherited makefile/flags reached Fortran make"
        assert cmd[:2] == ["make", "-C"] and "FC=gfortran" in cmd
        optimization = "-O3 -march=native" if cwd.name == "fortran-O3" else "-O1"
        expected = optimization + " -ffast-math -funroll-all-loops -fomit-frame-pointer -std=gnu"
        assert cmd[-1] == "FFLAGS=" + expected + (" -I../misc" if cmd[2] != "misc" else "")
        if cmd[2] != "misc":
            (cwd / cmd[2] / cmd[3]).write_bytes(b"synthetic Fortran binary")
        captured.append(cmd)
    with patch.object(kit, "prepare_rust"), patch.object(kit, "run", fake_run), patch("run.unpack", fake_unpack), \
            patch("run.command", return_value="cmake version 3.31.10"), patch("run.shutil.which", return_value="/module/bin/compiler"):
        kit.build()
    assert len(captured) == 8 and len(kit.p["builds"]) == 2
    assert set(controls).issubset(kit.p["environment"]["removed_names"])
    assert poison.read_text().startswith("override FC := synthetic-broken-compiler")


def test_checkpoints(out, keep_raw=False):
    kit = kit_at(out)
    kit.a.keep_raw = keep_raw
    kit.p["builds"] = {"base-solver-" + v: dict(kind="solver", ref="base", variant=v, binary=v) for v in ("default", "v3")}
    calls, interruptions = [], [KeyboardInterrupt("synthetic signal"), Deadline("synthetic deadline")]
    def driver_for(kit):
        driver = SimpleNamespace(invoke=lambda *a, **kw: None)
        def parse(argv):
            return SimpleNamespace(binary=argv[argv.index("--binary") + 1], output=Path(argv[argv.index("--output") + 1]),
                                   baseline=Path(argv[argv.index("--baseline") + 1]) if "--baseline" in argv else None)
        def check(args):
            calls.append(args.binary)
            driver.invoke()
            args.output.mkdir()
            if args.binary == "v3":
                assert (args.baseline / "snapshot.bin").read_bytes() == b"baseline"
                if interruptions:
                    raise interruptions.pop(0)
            (args.output / "snapshot.bin").write_bytes(b"baseline")
            mismatch = args.binary == "v3" and kit.a.seed == 999
            save(args.output / "summary.json", dict(mismatches=int(mismatch), unexpected_outcomes=0))
            append(args.output / "results.jsonl", dict(key="probe", status="ok", snapshot_sha256="same"))
            if mismatch:
                raise SystemExit(1)
        driver.parse_args, driver.check = parse, check
        return driver
    def solver(kit, directory, rows, number):
        calls.append("solver")
        rows.append(dict(ROW, kind="solver", variant="base-solver-default", samples_ns=[1_000_000]))
        (directory / "raw.txt").write_text("hash then delete")
    with patch("measure.cases", return_value=[]), patch("measure.quiet"), patch("measure.driver_for", driver_for), \
            patch("measure.solver_block", solver), patch("os.sched_getaffinity", return_value={0, 1}, create=True), patch("os.sched_setaffinity", create=True):
        for exception, status in ((KeyboardInterrupt, "failed"), (Deadline, "partial")):
            try:
                measure(kit)
            except exception:
                pass
            else:
                raise AssertionError("interruption not propagated")
            assert load(out / "records/identity-base-solver-default.json")["complete"]
            assert not load(out / "records/identity-base-solver-v3.json")["complete"]
            assert "solver" not in calls and calls.count("default") == 1
            assert (kit.work / "identity/base-solver-default/check/snapshot.bin").exists()
            assert summarize(kit, status, "synthetic interruption") == status
        measure(kit)
        save(out / "measure.done.json", {})
        assert summarize(kit, "complete", None) == "complete"
        measure(kit)
    assert calls == ["default", "v3", "v3", "v3", "solver"]
    assert load(out / "summary.json")["completed_blocks"] == 3
    for folder, expected in (("identity", 4), ("raw", 1)):
        remaining = list((kit.work / folder).iterdir())
        assert len(remaining) == (expected if keep_raw else 0)
        assert all(p.name.endswith("-hashed") for p in remaining)
    assert "raw.txt" in (out / "output-hashes.jsonl").read_text()
    bad = kit_at(out / "mismatch")
    bad.a.seed = 999
    bad.p["builds"] = kit.p["builds"]
    calls.clear()
    with patch("measure.cases", return_value=[]), patch("measure.quiet"), patch("measure.driver_for", driver_for), \
            patch("measure.solver_block", solver), patch("os.sched_getaffinity", return_value={0, 1}, create=True), patch("os.sched_setaffinity", create=True):
        for attempt in range(2):
            try:
                measure(bad)
            except RuntimeError as exc:
                assert "identity" in str(exc)
            else:
                raise AssertionError("solver identity mismatch did not stop timing")
    assert calls == ["default", "v3"] and not list((bad.work / "identity").iterdir())


def test_keep_raw(out):
    test_checkpoints(out, keep_raw=True)


def test_quick_scope(out):
    kit = kit_at(out)
    inventory = []
    for key in E2E:
        model = "bellhop" if key.startswith("bellhop") else "kraken"
        env = out / "fixtures" / (key + ".env")
        env.parent.mkdir(exist_ok=True)
        env.write_text(key)
        if model == "kraken":
            env.with_suffix(".flp").write_text("field")
        inventory.append(dict(key=key, model=model, path="fixtures/" + key + ".json"))
    save(out / "tools/perf/cases.json", inventory)
    assert [c["key"] for c in cases(out)] == list(E2E)
    kit.p["builds"] = {"base-cli-default": dict(kind="cli")}
    seen = []
    def trial(kit, directory, item, label, build, workers, count, warmup, block):
        seen.append((item["key"], workers, count, warmup))
        return dict(ROW, kind="sweep" if workers else "e2e", case=item["key"], workers=workers or 1, count=count,
                    warmup=warmup, variant=label, case_products={f"case-{i:04d}": PRODUCTS for i in range(count)})
    with patch("measure.quiet"), patch("measure.cli_trial", trial), patch("measure.solver_block"), \
            patch("os.sched_getaffinity", return_value={0, 1}, create=True), patch("os.sched_setaffinity", create=True):
        measure(kit)
    assert len(load(out / "plan.json")) == 23  # No solver identity builds in this scheduling-only mock.
    assert len(seen) == 22
    assert {(c, warmup) for c, w, n, warmup in seen if w is None} == {(c, b) for c in E2E for b in (True, False)}
    assert {(c, w, n, warmup) for c, w, n, warmup in seen if w} == {(c, w, 4, b) for c in ("pekeris", "surface-duct") for w in (1, 2) for b in (True, False)}
    assert max(i for i, (_, w, _, b) in enumerate(seen) if b and w is None) < min(i for i, (_, w, _, b) in enumerate(seen) if b and w)


def test_normalization(out):
    kit = kit_at(out)
    unchanged = {"/a/b": "first", "/a~b": "second", "url": "https://github.com/a/b", "shape": [1, 2, 3]}
    with patch.dict(os.environ, HOME="/"):
        assert archive_value(kit, unchanged) == unchanged
        assert archive_value(kit, str(out / "file")) == "<out>/file"
        with patch.object(kit, "out", Path("/")):
            assert archive_value(kit, unchanged) == unchanged
    home = out.parent / "user"
    with patch.dict(os.environ, HOME=str(home)):
        original = {str(home / "file"): str(out / "file"), "text": f"'{home}' --out={out}/file\n{home}/log",
                    "prefixes": [str(home) + "land/file", str(out) + "side/file", "/prefix" + str(home), "https://example.org" + str(home)]}
        normalized = archive_value(kit, original)
        assert len(normalized) == len(original) and normalized["~/file"] == "<out>/file"
        assert normalized["text"] == "'~' --out=<out>/file\n~/log"
        assert normalized["prefixes"] == original["prefixes"]
        for root, marker in ((home, "~"), (out, "<out>")):
            # Operators, response files, joined options and file URLs can start a path token.
            for lead in ("@", ">", "2>", "<", "|", "-I", "-L", "-MF", "-isystem", "-iquote", "-include", "-idirafter",
                         "cc -isystem", "-Wl,-rpath,", "--sysroot=", "file://", "PATH=/bin:"):
                assert archive_value(kit, lead + str(root / "f")) == lead + marker + "/f", lead
                assert archive_value(kit, lead + str(root)) == lead + marker, lead
            assert archive_value(kit, f"See {root}. Then {root}.\n") == f"See {marker}. Then {marker}.\n"
            assert archive_value(kit, f"Is it {root}? Yes, {root}!") == f"Is it {marker}? Yes, {marker}!"
            assert archive_value(kit, f"\"{root}...\" ‘{root}’ {root}\\f") == f"\"{marker}...\" ‘{marker}’ {marker}\\f"
            # Siblings, URL hosts, relative paths and expansions are other paths.
            siblings = (".backup/f", ".tar.gz", "-old/f", "2", "~", "+old/f", "@2/f", "%20old/f", "=old/f", "#old/f",
                        "$X/f", "^x", "*", "(1)/f", "[0-9]/f", "{,.bak}", "!old/f", "?x=1", ".d/f", "!!x")
            for text in tuple(str(root) + s for s in siblings) + (
                    "https://[::1]" + str(root), "https://example.org" + str(root), "." + str(root), "~" + str(root),
                    "a-I" + str(root), "x-isystem" + str(root), "/-I" + str(root), "$(pwd)" + str(root), "${X}" + str(root)):
                assert archive_value(kit, text) == text, text
        for collision in ({str(home / "x"): 1, "~/x": 2}, {str(out / "x"): 1, "<out>/x": 2}):
            try:
                archive_value(kit, collision)
            except ValueError as exc:
                assert "dictionary keys" in str(exc)
            else:
                raise AssertionError("normalization silently merged dictionary keys")
            assert len(collision) == 2
    with patch.dict(os.environ, HOME="/work"):
        assert archive_value(kit, str(out / "work/file")) == "<out>/work/file", "normalized a replacement twice"
    with patch.dict(os.environ, HOME=str(out.parent)):
        assert archive_value(kit, str(out / "file")) == "<out>/file", "home swallowed the more specific output path"


def test_archive(out):
    kit = kit_at(out)
    home = out.parent / "private-home"
    machine = dict(mounts="PRIVATE_MOUNTS", boot_id="PRIVATE_BOOT", hostname="PRIVATE_HOST",
                   kernel="Linux PRIVATE_HOST 6.1 x86_64 GNU/Linux", output_filesystem=f"{out} /dev/test ext4 rw")
    save(out / "machine.json", machine)
    kit.p["sources"]["base"]["home_path"] = str(home / "private")
    kit.persist()
    save(out / "records/test.json", dict(complete=True, rows=[dict(ROW, variant="base-cli-default")], identity=[]))
    save(out / "plan.json", ["test"])
    save(out / "measure.done.json", {})
    kit.env["CARGO_REGISTRIES_PRIVATE_TOKEN"] = "SYNTHETIC_NETWORK_SECRET"
    # The review's leaks, checked in the packaged archive rather than the helper alone.
    leaks = f"@{out}/work/tmp/args 2>{home}/err |{out}/pipe <{home}/in -I{out}/include -isystem{home}/include See {home}."
    append(out / "commands.jsonl", dict(diagnostic_tail="x" * 8000 + f"\n{out}/log {home}/private SYNTHETIC_NETWORK_SECRET\n" + leaks))
    with patch("measure.Path.home", return_value=home):
        assert summarize(kit, "complete", None) == "complete"
        with tarfile.open(out / "results.tar.gz") as tar:
            payload = b"\n".join(tar.extractfile(m).read() for m in tar)
            assert str(out).encode() not in payload and str(home).encode() not in payload
            assert b"@<out>/work/tmp/args 2>~/err |<out>/pipe <~/in -I<out>/include -isystem~/include See ~." in payload
            assert b"<out>" in payload and b"~/private" in payload
            assert all(s not in payload for s in (b"PRIVATE_MOUNTS", b"PRIVATE_BOOT", b"PRIVATE_HOST", b"SYNTHETIC_NETWORK_SECRET"))
            assert all(m.uid == m.gid == 0 and m.uname == m.gname == "" and not m.pax_headers for m in tar.getmembers())
            archived = json.load(tar.extractfile("machine.json"))
            assert archived["output_filesystem"] == "<out> /dev/test ext4 rw"
            diagnostic = json.loads(tar.extractfile("commands.jsonl").read())["diagnostic_tail"]
            assert len(diagnostic.encode()) <= 4096 and "~/private" in diagnostic
            assert not any("work/" in name or "raw" in name for name in tar.getnames())
        assert load(out / "machine.json") == machine, "transfer normalization changed resume evidence"
        with patch("measure.MAX_ARCHIVE_BYTES", 1):
            try:
                summarize(kit, "complete", None)
            except RuntimeError:
                pass
            else:
                raise AssertionError("archive cap not enforced")
    assert load(out / "summary.json")["status"] == "failed" and not (out / "results.tar.gz").exists()


def self_test():
    tests = (test_core, test_safety, test_workloads, test_shd_headers, test_identity, test_environment, test_compilers,
             test_make_environment, test_checkpoints, test_keep_raw, test_quick_scope, test_normalization, test_archive)
    with tempfile.TemporaryDirectory() as tmp:
        for test in tests:
            # Resolve symlinks (macOS's /var -> /private/var), as the kit resolves --out.
            out = Path(tmp).resolve() / test.__name__
            out.mkdir()
            test(out)
            print(test.__name__ + ": PASS")
    print(f"self-test: PASS ({len(tests)} groups; shapes, per-case sweeps/identity, build isolation, identity-first resume, quick scope, archive privacy/cap)")

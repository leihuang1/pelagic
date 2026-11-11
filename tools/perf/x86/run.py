#!/usr/bin/env python3
"""Pull, build and measure Pelagic on Linux; no third-party Python packages."""
import argparse
import fcntl
import hashlib
import importlib.util
import json
import math
import os
from pathlib import Path
import platform
import re
import shutil
import signal
import subprocess
import sys
import tarfile
import time
import urllib.request

REPOSITORY = "https://github.com/leihuang1/pelagic.git"
PHASES = ("setup", "build", "measure", "summarize")
FLAGS = {"default": "", "v3": "-C target-cpu=x86-64-v3", "cgu1": "-C codegen-units=1"}


def digest(path):
    h = hashlib.sha256()
    with Path(path).open("rb") as f:
        for chunk in iter(lambda: f.read(1024 * 1024), b""):
            h.update(chunk)
    return h.hexdigest()


def save(path, value):
    path = Path(path)
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_name(path.name + ".tmp")
    with temporary.open("w") as f:
        json.dump(value, f, indent=2, sort_keys=True)
        f.write("\n")
        f.flush()
        os.fsync(f.fileno())
    temporary.replace(path)


def load(path):
    return json.loads(Path(path).read_text())


def append(path, value):
    with Path(path).open("a") as f:
        f.write(json.dumps(value, sort_keys=True) + "\n")
        f.flush()


def tail(path):
    with Path(path).open("rb") as f:
        f.seek(max(0, f.seek(0, os.SEEK_END) - 4096))
        return f.read().decode(errors="replace")


def network_names(env):
    return sorted(k for k in env if k.lower().endswith("_proxy") or k.startswith(("CARGO_HTTP_", "CARGO_NET_", "CARGO_REGISTRIES_")))


def check_cargo_config(tree, cargo_home):
    tree = tree.resolve()
    paths = [parent / ".cargo" / name for parent in (tree, *tree.parents)
             for name in ("config", "config.toml")]
    paths += [cargo_home / name for name in ("config", "config.toml")]
    for path in paths:
        if path.exists() or path.is_symlink():
            if path.parent == tree / ".cargo":
                raise RuntimeError(f"Cargo config {path}: refs with their own Cargo config are not supported yet; choose a ref without one instead of editing the pinned ref")
            raise RuntimeError(f"Cargo config {path} could override the recorded build settings; move --out outside its ancestry, or move the config before building")


def parse_args(argv=None):
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("action", nargs="?", choices=("run", "status", "self-test"), default="run")
    p.add_argument("--ref", action="append", default=[], metavar="LABEL=COMMIT_OR_PR:N")
    p.add_argument("--out", type=Path)
    p.add_argument("--quick", action="store_true")
    p.add_argument("--keep-raw", action="store_true")
    p.add_argument("--install-system", action="store_true", help="explicitly allow apt-get (root/sudo)")
    p.add_argument("--seed", type=int, default=20261109)
    p.add_argument("--max-load", type=float, help="1-minute load ceiling; default max(2, allowed CPUs / 10)")
    p.add_argument("--deadline-minutes", type=float, help="measurement budget per invocation, excluding builds")
    a = p.parse_args(argv)
    for key in ("max_load", "deadline_minutes"):
        value = getattr(a, key)
        if value is not None and (not math.isfinite(value) or value <= 0):
            p.error(key.replace("_", "-") + " must be finite and positive")
    refs = {}
    for ref in a.ref:
        label, sep, revision = ref.partition("=")
        if (not sep or not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9_-]*", label)
                or label in refs or not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9_./:-]*", revision)
                or (revision.startswith("pr:") and not re.fullmatch(r"pr:[1-9][0-9]*", revision))):
            p.error("refs need unique safe LABEL=REF values (PR syntax: pr:N, positive N)")
        refs[label] = revision
    a.ref = refs
    if a.action != "self-test" and a.out is None:
        p.error("--out is required")
    if a.action == "run" and (not refs or (a.quick and len(refs) != 1)):
        p.error("supply --ref; --quick requires exactly one ref")
    if a.out and any(c in str(a.out) for c in "\n\r\0"):
        p.error("--out cannot contain line breaks or NUL (case lists are line-based)")
    return a


def command(cmd, cwd=None, *, env):
    return subprocess.check_output(cmd, cwd=cwd, env=env, text=True, stderr=subprocess.STDOUT).strip()


def native_tools(env):
    return {"cmake": "cmake", "gfortran": "gfortran", "cc": env.get("CC") or "cc", "c++": env.get("CXX") or "c++"}


def compiler_inventory(env):
    tools = native_tools(env)
    versions = {name: command([executable, "--version"], env=env) for name, executable in tools.items()}
    versions["executables"] = {name: shutil.which(executable, path=env["PATH"]) for name, executable in tools.items()}
    return versions


def check_cmake(version):
    if tuple(map(int, re.search(r"cmake version (\d+)\.(\d+)", version).groups())) < (3, 26):
        raise RuntimeError("CMake >=3.26 is required by the pinned HDF5 dependency; load a newer CMake/module (Bookworm's 3.25 is too old)")


def perf_driver(tree):
    spec = importlib.util.spec_from_file_location("pelagic_solver_driver", tree / "tools/perf/run.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def topology():
    cpus = sorted(os.sched_getaffinity(0))
    cores = {}
    for cpu in cpus:
        base = Path(f"/sys/devices/system/cpu/cpu{cpu}/topology")
        key = tuple((base / name).read_text().strip() for name in ("physical_package_id", "core_id"))
        cores.setdefault(key, []).append(cpu)
    flags = [set(line.split(":", 1)[1].split()) for line in Path("/proc/cpuinfo").read_text().splitlines()
             if line.startswith("flags")]
    common = set.intersection(*flags) if flags else set()
    required = set("avx avx2 bmi1 bmi2 f16c fma movbe xsave pni ssse3 sse4_1 sse4_2 popcnt cx16 lahf_lm".split())
    missing = required - common
    if not common.intersection({"abm", "lzcnt"}):
        missing.add("lzcnt/abm")
    reason = ("non-x86 machine: " + platform.machine() if platform.machine() != "x86_64"
              else "missing CPU/OS features: " + ", ".join(sorted(missing)) if missing else None)
    return dict(logical=cpus, physical=[v[0] for v in cores.values()], siblings=list(cores.values()),
                v3=reason is None, v3_skip_reason=reason)


def download(url, path):
    temporary = path.with_suffix(".download")
    for attempt in range(3):
        try:
            with urllib.request.urlopen(url, timeout=60) as response, temporary.open("wb") as f:
                shutil.copyfileobj(response, f)
            temporary.replace(path)
            return
        except OSError:
            if attempt == 2:
                raise
            time.sleep(2 ** attempt)


def unpack(archive, destination):
    # Reject links and traversal even though this archive is hash-pinned.
    with tarfile.open(archive) as tar:
        for member in tar:
            parts = Path(member.name).parts
            if not parts or Path(member.name).is_absolute() or ".." in parts or not (member.isdir() or member.isfile()):
                raise ValueError("unsafe archive member: " + member.name)
            target = destination.joinpath(*parts[1:])
            if member.isdir():
                target.mkdir(parents=True, exist_ok=True)
            else:
                target.parent.mkdir(parents=True, exist_ok=True)
                with tar.extractfile(member) as source, target.open("wb") as output:
                    shutil.copyfileobj(source, output)


class Kit:
    def __init__(self, args, lock_fd):
        self.a = args
        self.lock_fd = lock_fd
        self.out = args.out.resolve()
        self.work = self.out / "work"
        self.work.mkdir(exist_ok=True)
        self.env = dict(os.environ)
        for key in list(self.env):
            if key.startswith(("PELAGIC_", "RUSTC", "CARGO_BUILD_", "CARGO_TARGET_", "CARGO_PROFILE_", "CARGO_ENCODED_", "CMAKE_")) or key in (
                    "RUSTFLAGS", "RUSTDOCFLAGS", "RUSTUP_TOOLCHAIN", "FC", "AR", "CFLAGS", "CXXFLAGS",
                    "CPPFLAGS", "FFLAGS", "FCFLAGS", "LDFLAGS", "MAKEFLAGS", "MFLAGS", "MAKEFILES", "GNUMAKEFLAGS"):
                del self.env[key]
        removed = sorted(set(os.environ) - set(self.env))
        for name, directory in (("CARGO_HOME", "cargo"), ("TMPDIR", "tmp")):
            path = self.work / directory
            path.mkdir(exist_ok=True)
            self.env[name] = str(path)
        self.env["PATH"] = str(Path(self.env["CARGO_HOME"]) / "bin") + os.pathsep + self.env.get("PATH", os.defpath)
        if (self.work / "rustup").exists():
            self.env["RUSTUP_HOME"] = str(self.work / "rustup")
        self.env.update(CARGO_INCREMENTAL="0", OMP_NUM_THREADS="1", OPENBLAS_NUM_THREADS="1", MKL_NUM_THREADS="1", DEBIAN_FRONTEND="noninteractive")
        self.p = load(self.out / "provenance.json") if (self.out / "provenance.json").exists() else dict(sources={}, builds={})
        self.p["environment"] = dict(cargo_home=self.env["CARGO_HOME"], tmpdir=self.env["TMPDIR"], cargo_incremental="0",
                                     removed_names=removed, network_names=network_names(self.env),
                                     rustup_home=self.env.get("RUSTUP_HOME", str(Path.home() / ".rustup")))
        self.coverage = {}
        self.deadline = None
        self.phase = "setup"

    def state(self, detail, status="running"):
        save(self.out / "state.json", dict(status=status, phase=self.phase, detail=detail, pid=os.getpid(), utc=time.time()))
        print(self.phase + ": " + detail, flush=True)

    def persist(self):
        save(self.out / "provenance.json", self.p)

    def run(self, cmd, cwd=None, env=None):
        logs = self.work / "logs"
        logs.mkdir(exist_ok=True)
        log = logs / (str(time.time_ns()) + ".log")
        row = dict(command=list(map(str, cmd)), cwd=str(cwd or self.work), utc=time.time())
        with log.open("w") as f:
            process = subprocess.Popen(row["command"], cwd=cwd or self.work, env=env or self.env, stdout=f, stderr=f,
                                       start_new_session=True, pass_fds=(self.lock_fd,))
            try:
                process.wait()
            except BaseException:
                os.killpg(process.pid, signal.SIGKILL)
                process.wait()
                raise
            finally:
                row.update(exit_code=process.returncode, log=str(log), log_sha256=digest(log))
                if process.returncode:
                    row["diagnostic_tail"] = tail(log)
                append(self.out / "commands.jsonl", row)
        if process.returncode:
            raise RuntimeError(f"command failed ({process.returncode}): {row['command']}; see {log} and commands.jsonl's diagnostic_tail")

    def setup(self):
        missing = [name for name in ("git", "make", "curl", "xargs", "lscpu", "findmnt", *native_tools(self.env).values())
                   if not shutil.which(name, path=self.env["PATH"])]
        if missing and self.a.install_system:
            if not shutil.which("apt-get"):
                raise RuntimeError("--install-system supports apt-get only; load/install the missing tools: " + ", ".join(missing))
            prefix = [] if os.geteuid() == 0 else ["sudo", "-n"]
            self.run(prefix + ["apt-get", "update"])
            self.run(prefix + ["apt-get", "install", "-y", "gfortran", "cmake", "build-essential", "git", "curl",
                               "ca-certificates", "util-linux", "findutils"])
        elif missing:
            raise RuntimeError("Missing tools: " + ", ".join(missing) + "; load modules or explicitly allow --install-system. No system packages installed.")
        compilers = compiler_inventory(self.env)
        check_cmake(compilers["cmake"])
        self.p["topology"] = topology()
        machine = {name: command(cmd, env=self.env) for name, cmd in {
            "lscpu": ["lscpu"], "lscpu_e": ["lscpu", "-e"], "kernel": ["uname", "-a"],
            "output_filesystem": ["findmnt", "-T", str(self.out), "-o", "TARGET,SOURCE,FSTYPE,OPTIONS"]}.items()}
        machine.update(compilers)
        for name, path in {"memory": "/proc/meminfo", "mounts": "/proc/mounts", "cpuinfo": "/proc/cpuinfo",
                           "boot_id": "/proc/sys/kernel/random/boot_id", "cpu_quota": "/sys/fs/cgroup/cpu.max"}.items():
            machine[name] = Path(path).read_text() if Path(path).exists() else "unavailable"
        machine.update(topology=self.p["topology"], python=sys.version, hostname=platform.node())
        save(self.out / "machine.json", machine)
        trees = self.work / "trees"
        trees.mkdir(exist_ok=True)
        for label, ref in self.a.ref.items():
            tree = trees / label
            self.state("fetch " + label)
            if label not in self.p["sources"]:
                if tree.exists():
                    shutil.rmtree(tree)
                self.run(["git", "clone", "--filter=blob:none", "--no-checkout", REPOSITORY, tree])
                if ref.startswith("pr:"):
                    self.run(["git", "fetch", "--depth=1", "origin", "refs/pull/" + ref[3:] + "/head"], tree)
                    resolved = command(["git", "rev-parse", "--verify", "FETCH_HEAD^{commit}"], tree, env=self.env)
                else:
                    revision = ref if re.fullmatch(r"[0-9a-fA-F]{7,40}", ref) else "origin/" + ref.removeprefix("refs/heads/")
                    try:
                        resolved = command(["git", "rev-parse", "--verify", "--end-of-options", revision + "^{commit}"], tree, env=self.env)
                    except subprocess.CalledProcessError:
                        self.run(["git", "fetch", "origin", ref], tree)
                        resolved = command(["git", "rev-parse", "--verify", "FETCH_HEAD^{commit}"], tree, env=self.env)
                    if re.fullmatch(r"[0-9a-fA-F]{7,40}", ref) and not resolved.startswith(ref.lower()):
                        raise RuntimeError("resolved commit does not match requested SHA")
                self.p["sources"][label] = dict(requested=ref, commit=resolved, tree=str(tree), repository=REPOSITORY)
                self.persist()
            resolved = self.p["sources"][label]["commit"]
            self.run(["git", "checkout", "--detach", resolved], tree)
            if command(["git", "rev-parse", "HEAD"], tree, env=self.env) != resolved:
                raise RuntimeError("checkout verification failed: " + label)
            channel = re.search(r'^channel\s*=\s*"([^"]+)"', (tree / "rust-toolchain.toml").read_text(), re.M).group(1)
            self.p["sources"][label]["toolchain"] = channel
        self.tree = Path(self.p["sources"][next(iter(self.a.ref))]["tree"])
        script = (self.tree / "tools/reference/build-image.sh").read_text()
        dockerfile = (self.tree / "tools/reference/Dockerfile").read_text()
        commit = re.search(r"^commit=([0-9a-f]{40})$", script, re.M).group(1)
        expected = re.search(r"^expected=([0-9a-f]{64})$", script, re.M).group(1)
        if f"AT_ARCHIVE_SHA256={expected}" not in dockerfile or f"Acoustics-Toolbox-{commit}.tar.gz" not in dockerfile:
            raise RuntimeError("Fortran reference pins disagree")
        url = f"https://codeload.github.com/oalib-acoustics/Acoustics-Toolbox/tar.gz/{commit}"
        archive = self.work / "reference.tar.gz"
        if not archive.exists() or digest(archive) != expected:
            self.state("download pinned Fortran")
            download(url, archive)
        if digest(archive) != expected:
            raise RuntimeError("Fortran archive SHA-256 mismatch; not extracting")
        self.p["fortran"] = dict(commit=commit, sha256=expected, url=url, archive=str(archive))
        from measure import cases
        driver = perf_driver(self.tree)
        selected = load(self.tree / "tools/perf/cases.json")
        if self.a.quick:
            selected = [c for c in selected if c["key"] in ("pekeris", "bellhop-munk-tl")]
        save(self.out / "solver-cases.json", selected)
        inputs = driver.inventory() + [driver.case_inputs(c) for c in selected]
        self.p["inputs"] = {str(self.tree / f["path"]): f["sha256"] for c in inputs for f in c["inputs"] if f["status"] == "present"}
        for case in cases(self.tree):
            self.p["inputs"].update({str(f): digest(f) for f in case["files"]})
        self.p["kit"] = {f.name: digest(f) for f in Path(__file__).parent.glob("*.py")}
        self.persist()
        self.prepare_rust()

    def prepare_rust(self):
        private = self.work / "rustup"
        if not shutil.which("rustup", path=self.env["PATH"]) and not shutil.which("rustc", path=self.env["PATH"]):
            private.mkdir(exist_ok=True)
        if private.exists():
            self.env["RUSTUP_HOME"] = str(private)
        self.p["environment"]["rustup_home"] = self.env.get("RUSTUP_HOME", str(Path.home() / ".rustup"))
        if private.exists() and not shutil.which("rustup", path=self.env["PATH"]):
            installer = private / "rustup-init.sh"
            download("https://sh.rustup.rs", installer)
            self.run(["sh", installer, "-y", "--no-modify-path", "--profile", "minimal", "--default-toolchain", "none"])
        for source in self.p["sources"].values():
            channel = source["toolchain"]
            env = dict(self.env, RUSTUP_TOOLCHAIN=channel)
            if shutil.which("rustup", path=self.env["PATH"]):
                try:
                    command(["rustup", "run", channel, "rustc", "--version"], env=self.env)
                except subprocess.CalledProcessError:
                    self.state("install Rust " + channel)
                    self.run(["rustup", "toolchain", "install", channel, "--profile", "minimal", "--no-self-update"])
            version = command(["rustc", "-vV"], env=env)
            if not version.startswith("rustc " + channel + " "):
                raise RuntimeError(f"Rust {channel} required, found {version}; load the pinned module or rustup")
            source["rustc"] = version
            source["host"] = re.search(r"^host: (\S+)$", version, re.M).group(1)
            source["cargo"] = command(["cargo", "--version"], env=env)
        self.persist()

    def build(self):
        for source in self.p["sources"].values():
            check_cargo_config(Path(source["tree"]), Path(self.env["CARGO_HOME"]))
        self.prepare_rust()
        compilers = compiler_inventory(self.env)
        check_cmake(compilers["cmake"])
        for label, source in self.p["sources"].items():
            tree = Path(source["tree"])
            for kind, variants in (("cli", FLAGS), ("solver", ("default", "v3"))):
                for variant in variants:
                    if variant == "v3" and not self.p["topology"]["v3"]:
                        continue
                    key = f"{label}-{kind}-{variant}"
                    previous = self.p["builds"].get(key)
                    if previous and Path(previous["binary"]).exists() and digest(previous["binary"]) == previous["sha256"]:
                        continue
                    target = self.work / "targets" / key
                    env = dict(self.env, CARGO_TARGET_DIR=str(target), RUSTFLAGS=FLAGS[variant], RUSTUP_TOOLCHAIN=source["toolchain"])
                    # Rust otherwise invokes cc even when the site's C compiler is selected with CC.
                    env["CARGO_TARGET_" + source["host"].upper().replace("-", "_") + "_LINKER"] = compilers["executables"]["cc"]
                    cmd = ["cargo", "build", "--release", "--locked"] + (["-p", "cli"] if kind == "cli" else
                           ["--manifest-path", "tools/perf/Cargo.toml", "--bin", "pelagic-perf"])
                    self.state("build " + key)
                    self.run(cmd, tree, env)
                    binary = target / "release" / ("pelagic" if kind == "cli" else "pelagic-perf")
                    record = dict(ref=label, kind=kind, variant=variant, binary=str(binary), sha256=digest(binary),
                                  source_commit=source["commit"], source_dirty=False, flags=FLAGS[variant],
                                  toolchain=source["rustc"], build_command=cmd, target=str(target), compilers=compilers, linker=compilers["executables"]["cc"])
                    if kind == "cli":
                        record["no_sync"] = {f"{model}/{action}": "--no-sync" in command([str(binary), model, action, "--help"], env=self.env)
                                             for model in ("kraken", "bellhop") for action in ("run", "run-many")}
                    self.p["builds"][key] = record
                    self.persist()
        for optimization in ("O3", "O1"):
            key = "fortran-" + optimization
            previous = self.p["builds"].get(key)
            if previous and all(Path(p).exists() and digest(p) == h for p, h in previous["binaries"].items()):
                continue
            tree = self.work / key
            if tree.exists():
                shutil.rmtree(tree)
            unpack(Path(self.p["fortran"]["archive"]), tree)
            # Upstream ships stale objects: never let make reuse them across flags/architectures.
            for path in tree.rglob("*"):
                if path.is_file() and path.suffix in (".o", ".mod", ".exe", ".a"):
                    path.unlink()
            flags = "-O3 -march=native" if optimization == "O3" else "-O1"
            flags += " -ffast-math -funroll-all-loops -fomit-frame-pointer -std=gnu"
            self.state("build " + key)
            for directory, targets in (("misc", []), ("Kraken", ["kraken.exe"]), ("KrakenField", ["field.exe"]), ("Bellhop", ["bellhop.exe"])):
                self.run(["make", "-C", directory, *targets, "FC=gfortran", "FFLAGS=" + flags + (" -I../misc" if directory != "misc" else "")], tree)
            binaries = {str(tree / p): digest(tree / p) for p in ("Kraken/kraken.exe", "KrakenField/field.exe", "Bellhop/bellhop.exe")}
            self.p["builds"][key] = dict(kind="fortran", tree=str(tree), flags=flags, binaries=binaries, compilers=compilers)
            self.persist()

    def verify(self):
        if load(self.out / "machine.json")["boot_id"] != Path("/proc/sys/kernel/random/boot_id").read_text():
            raise RuntimeError("machine rebooted/changed; use a new --out")
        if topology() != self.p["topology"]:
            raise RuntimeError("CPU affinity/topology changed; use a new --out")
        files = dict(self.p["inputs"])
        for build in self.p["builds"].values():
            files.update(build.get("binaries", {build.get("binary", ""): build.get("sha256")}))
        for path, expected in files.items():
            if digest(path) != expected:
                raise RuntimeError("provenance changed: " + path)


def status(out):
    path = out / "state.json"
    if not path.exists():
        return "not started"
    state = load(path)
    text = f"{state['status']}: {state['phase']} — {state['detail']}"
    if state["status"] == "running":
        with (out / ".lock").open("a") as lock:
            try:
                fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
                text = "interrupted (no runner): " + text
            except BlockingIOError:
                pass
    return text


def main(argv=None):
    args = parse_args(argv)
    if args.action == "self-test":
        from selftest import self_test
        self_test()
        return 0
    if args.action == "status":
        print(status(args.out))
        return 0
    if platform.system() != "Linux":
        raise RuntimeError("measurements require Linux; self-test is portable")
    args.out = args.out.resolve()
    args.out.mkdir(parents=True, exist_ok=True)
    config = {k: v for k, v in vars(args).items() if k not in ("out", "deadline_minutes", "install_system", "action")}
    config["ref"] = [list(pair) for pair in args.ref.items()]
    config["kit_sha256"] = {f.name: digest(f) for f in Path(__file__).parent.glob("*.py")}
    with (args.out / ".lock").open("a") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        config_path = args.out / "config.json"
        if config_path.exists():
            if load(config_path) != config:
                raise RuntimeError("configuration/kit changed; use a new --out (only deadline/install permission may change)")
        else:
            if set(p.name for p in args.out.iterdir()) != {".lock"}:
                raise RuntimeError("--out must be empty or an existing kit run")
            save(config_path, config)
        kit = Kit(args, lock.fileno())
        def interrupted(signum, frame):
            raise KeyboardInterrupt("signal " + str(signum))
        signal.signal(signal.SIGTERM, interrupted)
        from measure import Deadline, measure, summarize
        outcome, error = "complete", None
        try:
            for phase in PHASES[:-1]:
                kit.phase = phase
                marker = args.out / (phase + ".done.json")
                if marker.exists():
                    continue
                kit.state("starting")
                if phase == "measure":
                    kit.verify()
                    measure(kit)
                else:
                    getattr(kit, phase)()
                save(marker, dict(utc=time.time()))
        except Deadline as exc:
            outcome, error = "partial", str(exc)
        except (Exception, KeyboardInterrupt) as exc:
            outcome, error = "failed", str(exc) or type(exc).__name__
        kit.phase = "summarize"
        kit.state("writing summaries")
        try:
            outcome = summarize(kit, outcome, error)
            save(args.out / "summarize.done.json", dict(status=outcome, utc=time.time()))
        except Exception as exc:
            outcome, error = "failed", "summary failed: " + str(exc)
            for name in ("results.tar.gz", "archive.json", "summarize.done.json"):
                (args.out / name).unlink(missing_ok=True)
            save(args.out / "summary.json", dict(status=outcome, error=error))
            (args.out / "summary.md").write_text("# Failed summary\n\n" + error + "\n")
        kit.state(error or "results.tar.gz", outcome)
        return {"complete": 0, "partial": 2, "failed": 1}[outcome]


if __name__ == "__main__":
    sys.dont_write_bytecode = True
    sys.exit(main())

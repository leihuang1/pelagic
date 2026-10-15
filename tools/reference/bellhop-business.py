#!/usr/bin/env python3
"""Broad, diagnostic BELLHOP business-case differential; never a release gate.

Unmodified upstream inputs, labelled small derivatives, real Fortran, real Rust,
legacy/JSON and CLI readback. Failures are retained, not waived or regenerated.
"""
import argparse
from collections import Counter
from datetime import datetime, timezone
import hashlib
import json
import math
import os
import platform
from pathlib import Path
import re
import shutil
import subprocess
import struct
import sys
import time

ROOT = Path(__file__).resolve().parents[2]
IMAGE = "pelagic-reference:v2023.5-amd64"
RESOURCES = {".env", ".ssp", ".ati", ".bty", ".brc", ".trc", ".irc", ".sbp"}
TESTS = {
    "rays": ("ray", "ray_trajectories_match_pinned_linux_reference"),
    "eigenrays": ("ray", "ray_trajectories_match_pinned_linux_reference"),
    "arrivals": ("arr", "arrivals_match_pinned_linux_reference"),
    "coherent": ("shd", "pressure_fields_match_pinned_linux_reference"),
    "semi_coherent": ("shd", "pressure_fields_match_pinned_linux_reference"),
    "incoherent": ("shd", "pressure_fields_match_pinned_linux_reference"),
}


def official_cases(tests):
    """Literal active MATLAB calls only; do not interpret MATLAB or include 3D."""
    cases = set()
    for script in sorted(tests.rglob("runtests*.m")):
        if "Bellhop3DTests" in script.parts:
            continue
        for line in script.read_text(errors="replace").splitlines():
            match = re.match(r"^\s*bellhop\b\s*(?:\(\s*)?['\"]?([\w-]+)", line)
            if match:
                cases.add(script.parent / (match[1] + ".env"))
    return sorted(cases)


def shelf(options="CG", interpolation="C", top="V", bottom="A", loss="W", extra="", irregular=False, cerveny="MS", images=1, component="P", shape=False):
    """Constructed 100-m shelf: two sources, five ranges, refracting/lossy water."""
    top_material = "0 340 0 0.0012 0 0 /\n" if top == "A" else ""
    bottom_material = "100 1700 0 1.8 0.2 0 /\n" if bottom == "A" else ""
    depths = "5\n20 35 50 65 80 /" if irregular else "3\n20 50 80 /"
    beam = options[1] if len(options) > 1 else "G"
    tail = f"'{cerveny}' 1.0 1.0\n{images} 5 '{component}'\n" if beam in "CR" else ""
    return (f"'Constructed shelf {options} {interpolation} {top}{bottom} {loss}'\n1000\n1\n"
            f"'{interpolation}{top}{loss.ljust(2)}{'*' if shape else ''}'\n{extra}{top_material}"
            "5 0 100\n0 1510 0 1 0.01 /\n25 1505 /\n50 1490 /\n75 1495 /\n100 1500 /\n"
            f"'{bottom}{'*' if shape else ''}' 0\n{bottom_material}"
            f"2\n25 65 /\n{depths}\n5\n0.2 1.0 /\n'{options}'\n101\n-70 70 /\n5 110 1.1\n{tail}")


def derived_cases(directory):
    directory.mkdir()
    cases = []

    def add(name, text, resources=None):
        # G/g and A/a must remain distinct on case-insensitive filesystems.
        path = directory / f"{len(cases):03}-{name}.env"
        path.write_text(text)
        for suffix, content in (resources or {}).items():
            path.with_suffix(suffix).write_text(content)
        cases.append(path)

    for geometry in ["R", "X"]:
        for mode in "CSI":
            for beam in "GgBSCR":
                add(f"shelf-{mode}{beam}-{geometry}", shelf(f"{mode}{beam} {geometry}"))
        for mode in "Aa":
            for beam in "GgB":
                add(f"shelf-{mode}{beam}-{geometry}", shelf(f"{mode}{beam} {geometry}"))
    for beam in "GgBS":
        add(f"shelf-E{beam}", shelf(f"E{beam}"))
    for interpolation in "NPSQ":
        resources = None
        if interpolation == "Q":
            resources = {".ssp": "3\n0 0.5 1.2\n1510 1508 1506\n1505 1502 1500\n1490 1492 1494\n1495 1498 1500\n1500 1504 1508\n"}
        for beam in "GB":
            add(f"shelf-ssp-{interpolation}-{beam}", shelf(f"C{beam}", interpolation), resources)
    for top in "VRA":
        for bottom in "VRA":
            add(f"shelf-boundary-{top}{bottom}", shelf(top=top, bottom=bottom))
    for unit in "NMFQL":
        text = shelf(loss=unit)
        if unit == "Q":
            text = text.replace("1 0.01 /", "1 1000 / ").replace("1.8 0.2 0 /", "1.8 50 0 /")
        add(f"shelf-attenuation-{unit}", text)
    add("shelf-thorp", shelf(loss="WT"))
    add("shelf-francois-garrison", shelf(loss="WF", extra="10 35 8 0.05\n"))
    add("shelf-biological", shelf(loss="WB", extra="2\n0 75 1000 2 0.01\n25 100 1500 3 0.02\n"))
    for beam in "GgBSCR":
        add(f"shelf-irregular-{beam}", shelf(f"C{beam}  I", irregular=True))
    for mode in "Aa":
        for beam in "GB":
            add(f"shelf-irregular-{mode}{beam}", shelf(f"{mode}{beam}  I", irregular=True))
    for beam in "CR":
        for width, curvature, images, component in [
            ("F", "S", 1, "P"), ("M", "D", 3, "P"), ("W", "Z", 1, "P"),
            ("M", "S", 1, "V"), ("M", "S", 1, "H"),
        ]:
            add(f"shelf-cerveny-{beam}{width}{curvature}-{images}-{component}",
                shelf(f"C{beam}", cerveny=width + curvature, images=images, component=component))
    for interpolation in "LC":
        for beam in "GBCR":
            resources = {
                ".ati": f"'{interpolation}'\n4\n-1 2\n0 0\n0.6 3\n2 0\n",
                ".bty": f"'{interpolation}'\n4\n-1 100\n0 100\n0.6 85\n2 100\n",
            }
            add(f"shelf-shaped-{interpolation}-{beam}", shelf(f"C{beam}", shape=True), resources)
    add("shelf-pattern", shelf("CG*"), {".sbp": "5\n-90 0\n-30 -3\n0 -6\n30 -3\n90 0\n"})
    return cases


def adjusted_cases(upstream, directory):
    """Alternatives are additional inputs, never replacements for failed originals."""
    directory.mkdir()
    cases = []
    for mode in "SI":
        for geometry in "RX":
            name = f"{len(cases):03}-sgb-{mode}-{geometry}-use-gaussian.env"
            path = directory / name
            path.write_text(shelf(f"{mode}B {geometry}").replace("Constructed shelf", "Adjusted SGB alternative: geometric Gaussian"))
            cases.append(path)
    for stem, beam in [("aetB", "B"), ("aetB_TL", "g")]:
        lines = (upstream / "Noise/ATOC" / f"{stem}.env").read_text().splitlines(keepends=True)
        start = next(i for i, line in enumerate(lines) if re.search(r"!\s*NSd\b", line, re.IGNORECASE))
        assert "PVWT" in "".join(lines[:start])
        lines[0] = f"'Adjusted ATOC {stem}: 50 km, 3001 beams, 100 m step, 101 x 101 receivers'\n"
        path = directory / f"{len(cases):03}-{stem}-regional.env"
        path.write_text("".join(lines[:start]) + f"1\n1.25 /\n101\n0 6000 /\n101\n0.1 50 /\n'C{beam}'\n3001\n-89 89 /\n100 6500 50.1\n")
        cases.append(path)
    return cases


def nonfinite_pressure_count(data):
    record = 4 * struct.unpack_from("<i", data)[0]
    nf, nt, nx, ny, ns, nz, nr = struct.unpack_from("<7i", data, 2 * record)
    if (nf, nt, nx, ny) != (1, 1, 1, 1):
        raise ValueError("unsupported reference SHD dimensions")
    if data[record:record + 10] == b"irregular ":
        nz = 1
    if record < 8 * nr or len(data) != record * (10 + ns * nz):
        raise ValueError("invalid reference SHD record layout")
    return sum(not math.isfinite(value[0]) for row in range(ns * nz)
               for value in struct.iter_unpack("<f", data[(10 + row) * record:(10 + row) * record + 8 * nr]))


def execute(command, log, environment, timeout, cwd=ROOT):
    with log.open("wb") as stream:
        try:
            result = subprocess.run(command, cwd=cwd, env=environment, stdout=stream,
                                    stderr=subprocess.STDOUT, timeout=timeout)
            return result.returncode == 0
        except subprocess.TimeoutExpired:
            stream.write(b"\nTIMEOUT: this stage was not accepted.\n")
            return False


def test_binary(package, target, environment, log, timeout):
    command = ["cargo", "test", "--locked", "--release", "-p", package,
               "--test", target, "--no-run", "--message-format=json"]
    if not execute(command, log, environment, timeout):
        raise RuntimeError(f"test binary build failed; see {log}")
    artifacts = [json.loads(line) for line in log.read_text().splitlines() if line.startswith("{")]
    return next(a["executable"] for a in artifacts if a.get("executable") and a["target"]["name"] == target)


def check_test(binary, name, log, environment, timeout):
    ok = execute([binary, name, "--ignored", "--exact", "--nocapture"], log, environment, timeout)
    return ok and log.read_text().count("test result: ok. 1 passed; 0 failed; 0 ignored;") == 1


def corruption_control(artifact, differential, test, environment, directory, timeout):
    """A passing comparison must detect damage in the last source, not just source 0."""
    damaged = directory / ("damaged" + artifact.suffix)
    data = bytearray(artifact.read_bytes())
    if artifact.suffix == ".shd":
        record_bytes = 4 * struct.unpack_from("<i", data)[0]
        ranges = struct.unpack_from("<i", data, 2 * record_bytes + 24)[0]
        struct.pack_into("<f", data, len(data) - record_bytes + 8 * (ranges - 1), 999.0)
    elif data.lstrip().startswith(b"'"):
        lines = data.decode().splitlines()
        index = next(i for i in range(len(lines) - 1, 4, -1) if len(lines[i].split()) == 8)
        fields = lines[index].split()
        fields[0] = "999.0"
        lines[index] = " ".join(fields)
        data = ("\n".join(lines) + "\n").encode()
    else:
        # Locate the last 32-byte arrival, retaining both Fortran record markers.
        offset, last = 0, None
        while offset < len(data):
            length = struct.unpack_from("<i", data, offset)[0]
            if length == 32:
                last = offset + 4
            offset += length + 8
        assert last is not None
        struct.pack_into("<f", data, last, 999.0)
    damaged.write_bytes(data)
    variable = f"BELLHOP_DIFFERENTIAL_{artifact.suffix[1:].upper()}"
    log = directory / "corruption-control.log"
    accepted = check_test(differential, test, log, dict(environment, **{variable: str(damaged)}), timeout)
    message = "pressure mismatch" if artifact.suffix == ".shd" else "arrival fields exceed tolerance"
    return not accepted and message in log.read_text()


def write_report(output, metadata, results):
    (output / "results.json").write_text(json.dumps({"metadata": metadata, "results": results}, indent=2) + "\n")
    lines = ["# BELLHOP business-case diagnostic", "", "Not release acceptance. No failed case is waived.", "",
             "```json", json.dumps(metadata, indent=2), "```", "", str(dict(Counter(r["status"] for r in results))), "",
             "| Case | Kind | Status | Evidence |", "|---|---|---|---|"]
    for result in results:
        lines.append(f'| {result["case"]} | {result.get("kind", "—")} | {result["status"]} | [{result.get("stage", "complete")}]({result["directory"]}/) |')
    (output / "report.md").write_text("\n".join(lines) + "\n")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path)
    parser.add_argument("--only", default=".*", help="case-name regex for a focused diagnostic (not full coverage)")
    parser.add_argument("--timeout", type=int, default=120, help="seconds per stage; timeout is failure, not skip")
    parser.add_argument("--self-test", action="store_true")
    parser.add_argument("--native-reference", action="store_true", help="inside the fixed Linux image: execute its oracle directly")
    args = parser.parse_args()
    if args.self_test:
        self_test()
        return 0
    output = (args.output or ROOT / "target" / "bellhop-business" / datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ")).resolve()
    output.mkdir(parents=True, exist_ok=False)
    environment = {k: v for k, v in os.environ.items() if not k.startswith("BELLHOP_")}
    target = Path(os.environ.get("CARGO_TARGET_DIR", ROOT / "target" / "linux-business-build")) if args.native_reference else ROOT / "target"
    environment["CARGO_TARGET_DIR"] = str(target)
    if args.native_reference:
        assert platform.system() == "Linux" and platform.machine() == "x86_64"
    metadata = {"commit": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip(),
                "dirty": subprocess.check_output(["git", "status", "--porcelain"], cwd=ROOT, text=True),
                "only": args.only, "timeout_seconds": args.timeout,
                "rustc": subprocess.check_output(["rustc", "-Vv"], env=environment, text=True),
                "oracle_image": IMAGE, "platform": platform.platform(),
                "native_reference": args.native_reference}
    oracle_info = ["/bin/sh", "-c", "cat /opt/reference-build.txt; sha256sum /usr/local/bin/bellhop-fortran"]
    if not args.native_reference:
        oracle_info = ["docker", "run", "--rm", "--platform", "linux/amd64", "--entrypoint", "/bin/sh", IMAGE] + oracle_info[1:]
    if not execute(oracle_info, output / "oracle-build.log", environment, args.timeout):
        raise RuntimeError("build the pinned image with tools/reference/build-image.sh first")
    metadata["oracle"] = (output / "oracle-build.log").read_text()
    if not execute(["cargo", "build", "--locked", "--release", "-p", "cli"], output / "build.log", environment, args.timeout):
        raise RuntimeError("CLI build failed")
    differential = test_binary("bellhop", "differential_reference", environment, output / "build-differential.log", args.timeout)
    readback = test_binary("cli", "native_output", environment, output / "build-readback.log", args.timeout)
    cli = str(target / "release/pelagic")
    snapshot = output / "bin"
    snapshot.mkdir()
    for path in [cli, differential, readback]:
        shutil.copy2(path, snapshot / Path(path).name)
    cli, differential, readback = (str(snapshot / Path(path).name) for path in [cli, differential, readback])
    metadata["source_sha256"] = {str(path.relative_to(ROOT)): hashlib.sha256(path.read_bytes()).hexdigest()
                                 for path in sorted([*ROOT.glob("crates/*/src/**/*.rs"), ROOT / "Cargo.toml", ROOT / "Cargo.lock",
                                                     ROOT / "crates/bellhop/tests/differential_reference.rs"])}
    metadata["executable_sha256"] = {Path(path).name: hashlib.sha256(Path(path).read_bytes()).hexdigest()
                                     for path in [cli, differential, readback]}
    metadata["runner_sha256"] = hashlib.sha256(Path(__file__).read_bytes()).hexdigest()
    upstream = output / "upstream"
    upstream.mkdir()
    if args.native_reference:
        shutil.copytree("/opt/acoustics-toolbox/tests", upstream / "tests")
    else:
        subprocess.run(["docker", "run", "--rm", "--platform", "linux/amd64", "--volume", f"{upstream}:/out",
                        "--entrypoint", "/bin/sh", IMAGE, "-c", "cp -R /opt/acoustics-toolbox/tests /out/"], check=True)
    cases = [(f"official/{p.relative_to(upstream / 'tests')}", p) for p in official_cases(upstream / "tests")]
    cases += [(f"constructed/{p.name}", p) for p in derived_cases(output / "constructed")]
    cases += [(f"adjusted/{p.name}", p) for p in adjusted_cases(upstream / "tests", output / "adjusted")]
    golden = ROOT / "crates/bellhop/tests/fixtures/golden"
    cases += [(f"golden/{p.name}", p) for p in sorted(golden.glob("*.env")) if p.stem != "InternalReflection"]
    selected = [(name, path) for name, path in cases if re.search(args.only, name)]
    if not selected:
        raise ValueError("no cases selected")
    metadata["selected_cases"] = len(selected)
    metadata["excluded"] = ["Bellhop3DTests directory", "InternalReflection: reference BELLHOP has no P reflection branch; separate BOUNCE golden"]
    results = [{"case": name, "directory": f"case-{i:03}", "status": "NOT RUN"}
               for i, (name, _) in enumerate(selected)]
    write_report(output, metadata, results)
    controlled = set()
    for index, (name, source) in enumerate(selected):
        directory = output / f"case-{index:03}"
        directory.mkdir()
        result = results[index]
        result.update(status="FAIL", stage="input", hashes={})
        start = time.monotonic()
        for resource in source.parent.glob(source.stem + ".*"):
            if resource.suffix in RESOURCES:
                shutil.copyfile(resource, directory / resource.name)
                result["hashes"][resource.name] = hashlib.sha256(resource.read_bytes()).hexdigest()
        input_path = directory / source.name
        json_path = input_path.with_suffix(".json")
        try:
            result["stage"] = "input/export"
            with json_path.open("wb") as document_file, (directory / "export.log").open("wb") as log:
                exported = subprocess.run([cli, "bellhop", "export", str(input_path)], env=environment,
                                          stdout=document_file, stderr=log, timeout=args.timeout)
            if exported.returncode:
                continue
            document = json.loads(json_path.read_bytes())
            kind = document["run"]["kind"]
            result["kind"] = kind
            result["run"] = document["run"]
            result["sources"] = len(document["positions"]["source_depths_m"])
            extension, test = TESTS[kind]
            container = f"pelagic-business-{os.getpid()}-{index}"
            result["stage"] = "Fortran"
            # Fortran FileRoot is CHARACTER(80); use a short stem in the case directory.
            oracle_command = (["/usr/local/bin/bellhop-fortran", source.stem] if args.native_reference else
                              ["docker", "run", "--rm", "--name", container, "--platform", "linux/amd64",
                               "--volume", f"{directory}:/work", IMAGE, f"/work/{source.stem}"])
            oracle_ok = execute(oracle_command, directory / "fortran.log", environment, args.timeout, cwd=directory)
            if not oracle_ok:
                if not args.native_reference:
                    subprocess.run(["docker", "rm", "-f", container], capture_output=True)
                continue
            artifact = input_path.with_suffix("." + extension)
            if not artifact.exists():
                continue
            result["oracle_sha256"] = hashlib.sha256(artifact.read_bytes()).hexdigest()
            nonfinite = nonfinite_pressure_count(artifact.read_bytes()) if extension == "shd" else 0
            if nonfinite:
                result["oracle_nonfinite_components"] = nonfinite
                result["stage"] = "non-finite rejection"
                safe = True
                for label, input_file in [("legacy", input_path), ("json", json_path)]:
                    for format_name in ["both", "hdf5"]:
                        log = directory / f"{label}-{format_name}-rejection.log"
                        destination = directory / f"{label}-{format_name}-rejection"
                        accepted = execute([cli, "bellhop", "run", str(input_file), "--format", format_name,
                                            "--output", str(destination)], log, environment, args.timeout)
                        safe &= (not accepted and "BH0302" in log.read_text() and "non-finite" in log.read_text()
                                 and not list(destination.rglob("*.h5")))
                result["safe_rejection"] = safe
                result["status"] = "ORACLE_NONFINITE" if safe else "FAIL"
                continue
            for label, input_file in [("legacy", input_path), ("json", json_path)]:
                result["stage"] = f"{label}/differential"
                test_env = dict(environment, BELLHOP_DIFFERENTIAL_ENV=str(input_file),
                                **{f"BELLHOP_DIFFERENTIAL_{extension.upper()}": str(artifact)})
                if not check_test(differential, test, directory / f"{label}-differential.log", test_env, args.timeout):
                    break
                control_key = extension + ("-ascii" if extension == "arr" and artifact.read_bytes().lstrip().startswith(b"'") else "")
                if label == "legacy" and result["sources"] > 1 and extension in {"shd", "arr"} and control_key not in controlled:
                    result["stage"] = "corruption control"
                    if not corruption_control(artifact, differential, test, test_env, directory, args.timeout):
                        break
                    controlled.add(control_key)
                    result["corruption_control"] = "last source damage rejected"
                result["stage"] = f"{label}/CLI"
                cli_output = directory / f"{label}-output"
                if not execute([cli, "bellhop", "run", str(input_file), "--format", "both", "--output", str(cli_output)],
                               directory / f"{label}-cli.log", environment, args.timeout):
                    break
                result["stage"] = f"{label}/readback"
                test_env = dict(environment, BELLHOP_NATIVE_INPUT=str(input_file), BELLHOP_NATIVE_OUTPUT=str(cli_output))
                if not check_test(readback, "actual_cli_native_layouts_match_all_hdf5_values",
                                  directory / f"{label}-readback.log", test_env, args.timeout):
                    break
            else:
                result["status"] = "PASS"
                result["stage"] = "complete"
        except (ValueError, OSError, subprocess.SubprocessError, KeyError, struct.error) as error:
            result["error"] = str(error)
        finally:
            result["seconds"] = round(time.monotonic() - start, 3)
            log = directory / "legacy-differential.log"
            if log.exists():
                text = log.read_text()
                result["comparison"] = [line for line in text.splitlines() if line.startswith("compared ")]
                if "reference non-finite pressure" in text:
                    result["status"] = "ORACLE_NONFINITE"
            if name == "official/MunkRot/MunkRot.env" and "three-dimensional" in (directory / "export.log").read_text():
                result["status"] = "EXCLUDED_3D"
            write_report(output, metadata, results)
            print(f'[{index + 1}/{len(selected)}] {result["status"]} {name}: {result["stage"]}', flush=True)
    print(f"Report: {output / 'report.md'}")
    return int(any(result["status"] != "PASS" for result in results))


def self_test():
    import tempfile
    from unittest.mock import patch
    with tempfile.TemporaryDirectory() as temporary:
        root = Path(temporary)
        script = root / "runtests.m"
        script.write_text("% bellhop excluded\nbellhopM('excluded')\nbellhop('one')\nbellhop 'two'\nbellhop three\nbellhop('one')\n")
        assert [p.stem for p in official_cases(root)] == ["one", "three", "two"]
        cases = derived_cases(root / "derived")
        assert len(cases) == len({str(p).lower() for p in cases}) and len(cases) >= 100
        assert all(path.read_text().count("2\n25 65 /\n") == 1 for path in cases)
        upstream = root / "tests/Noise/ATOC"
        upstream.mkdir(parents=True)
        for stem in ["aetB", "aetB_TL"]:
            (upstream / f"{stem}.env").write_text("'original'\n'PVWT'\n1 ! NSd\nold geometry\n")
        alternatives = adjusted_cases(root / "tests", root / "adjusted")
        assert len(alternatives) == 6 and "old geometry" not in alternatives[-1].read_text()
        data = bytearray(12 * 164)
        struct.pack_into("<i", data, 0, 41)
        struct.pack_into("<7i", data, 2 * 164, 1, 1, 1, 1, 2, 1, 2)
        assert nonfinite_pressure_count(data) == 0
        struct.pack_into("<f", data, 11 * 164 + 12, float("nan"))
        assert nonfinite_pressure_count(data) == 1
        try:
            nonfinite_pressure_count(data[:-1])
        except ValueError:
            pass
        else:
            raise AssertionError("truncated reference SHD was accepted")
        write_report(root, {}, [{"case": "failed", "status": "FAIL", "directory": "case-000"},
                                {"case": "pending", "status": "NOT RUN", "directory": "case-001"}])
        assert "FAIL" in (root / "report.md").read_text() and "NOT RUN" in (root / "report.md").read_text()
        # Tooling controls only: these do not mock or claim numerical evidence.
        stub = root / "libtest.py"
        for passed, code in [(1, 0), (0, 0), (2, 0), (1, 7)]:
            stub.write_text(f"print('test result: ok. {passed} passed; 0 failed; 0 ignored;')\nraise SystemExit({code})\n")
            assert check_test(sys.executable, str(stub), root / "stub.log", os.environ, 5) == (passed == 1 and code == 0)
        # Replace only the Cargo command; exercise real process exit, timeout and log I/O.
        cargo_stub = root / "cargo.py"
        real_run = subprocess.run
        artifact = json.dumps({"reason": "compiler-artifact", "target": {"name": "probe"},
                               "executable": str(root / "probe")})
        for code, delay in [(7, 0), (0, 10), (0, 0)]:
            cargo_stub.write_text("import sys, time\n"
                                  f"print({artifact!r}, flush=True)\n"
                                  "print('compiler diagnostic', file=sys.stderr, flush=True)\n"
                                  f"time.sleep({delay})\nraise SystemExit({code})\n")
            log = root / f"build-{code}-{delay}.log"
            with patch.object(subprocess, "run", side_effect=lambda command, **kwargs:
                              real_run([sys.executable, str(cargo_stub)], **kwargs)):
                try:
                    binary = test_binary("bellhop", "probe", os.environ, log, 2)
                except RuntimeError as error:
                    assert code or delay
                    assert str(log) in str(error)
                else:
                    assert code == delay == 0, "failed or timed-out build was accepted"
                    assert binary == str(root / "probe")
            text = log.read_text()
            assert artifact in text and "compiler diagnostic" in text
            assert ("TIMEOUT:" in text) == (delay > 0)
    print("business runner self-check passed")


if __name__ == "__main__":
    sys.exit(main())

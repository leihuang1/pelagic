#!/usr/bin/env python3
"""KRAKEN business diagnostic in the pinned Linux image; never release acceptance.

Retains original inputs, unsupported combinations, reference failures and full
ENV/JSON/API/CLI comparisons. Uses existing comparators and process guards.
"""
import argparse
from collections import Counter
import hashlib
import importlib.util
import json
import math
import os
from pathlib import Path
import platform
import re
import shutil
import struct
import subprocess
import sys
import time

ROOT = Path(__file__).resolve().parents[2]
sys.dont_write_bytecode = True
spec = importlib.util.spec_from_file_location("business", Path(__file__).with_name("bellhop-business.py"))
business = importlib.util.module_from_spec(spec)
spec.loader.exec_module(business)


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def official_cases(tests):
    paths = set()
    for script in tests.rglob("runtests*.m"):
        for line in script.read_text(errors="replace").splitlines():
            match = re.match(r"\s*(krakenc?)\s*\(\s*['\"]([\w-]+)['\"]", line)
            if match:
                env = script.parent / (match[2] + ".env")
                paths.add((env, env.with_suffix(".flp"), match[1]))
    # These official scripts construct calls in loops or use shared FLP names.
    for stem in "pekeris double scholte flused elsed atten normal ice kupert kuperb kuperm".split():
        for engine in ("kraken", "krakenc"):
            paths.add((tests / f"TLslices/{stem}.env", tests / "TLslices/fieldbat.flp", engine))
    for stem in ("Iso", "DownwdRef", "Sduct"):
        for engine, suffix in (("kraken", ""), ("krakenc", "_C")):
            env = tests / f"Noise/KupermanIngenito/{stem}{suffix}.env"
            paths.add((env, env.with_suffix(".flp"), engine))
    for stem in ("gulf_ad", "gulf_cm"):
        paths.add((tests / "Gulf/gulf_rd.env", tests / f"Gulf/{stem}.flp", "kraken"))
    return sorted(paths)


def pressures(path):
    data = path.read_bytes()
    record = 4 * struct.unpack_from("<i", data)[0]
    nf, nt, nx, ny, ns, nz, nr = struct.unpack_from("<7i", data, 2 * record)
    if (nt, nx, ny) != (1, 1, 1) or min(nf, ns, nz, nr) < 1:
        raise ValueError("unsupported SHD dimensions")
    if record < max(164, 8 * nr) or len(data) != record * (10 + nf * ns * nz):
        raise ValueError("invalid SHD length")
    return [[complex(*p) for row in range(ns * nz)
             for p in struct.iter_unpack("<ff", data[(10 + f * ns * nz + row) * record:
                                                     (10 + f * ns * nz + row) * record + 8 * nr])]
            for f in range(nf)]


def field_metrics(reference, actual):
    if len(reference) != len(actual):
        raise ValueError("pressure length changed")
    error = energy = max_error = max_tl = max_phase = 0.0
    one_sided_zero = 0
    for expected, value in zip(reference, actual):
        if not all(math.isfinite(x) for x in (expected.real, expected.imag, value.real, value.imag)):
            raise ValueError("nonfinite pressure")
        difference = abs(value - expected)
        error += difference ** 2
        energy += abs(expected) ** 2
        max_error = max(max_error, difference)
        if abs(expected) and abs(value):
            max_tl = max(max_tl, abs(20 * math.log10(abs(value) / abs(expected))))
            phase = value * expected.conjugate()
            max_phase = max(max_phase, abs(math.degrees(math.atan2(phase.imag, phase.real))))
        elif bool(abs(expected)) != bool(abs(value)):
            one_sided_zero += 1
    return {"pressures": len(reference), "complex_max": max_error,
            "relative_l2": math.sqrt(error / energy) if energy else None,
            "max_tl_db_nonzero": max_tl, "max_phase_degrees_nonzero": max_phase,
            "one_sided_zeros": one_sided_zero}


def oracle_problem(text):
    if re.search(r"(?i)(fatal error|\*\*\*\s*error)", text):
        return "ORACLE_FAILED"
    if re.search(r"(?i)(failed to converge|too many meshes needed)", text):
        return "ORACLE_UNCONVERGED"
    return None


def report(output, metadata, results):
    (output / "results.json").write_text(json.dumps({"metadata": metadata, "results": results}, indent=2) + "\n")
    lines = ["# KRAKEN business diagnostic", "", "Not release acceptance; statuses are not waived.", "",
             str(dict(Counter(row["status"] for row in results))), "",
             "| Input | Engine | Status | Stage |", "|---|---|---|---|"]
    lines += [f'| {r["case"]} | {r["engine"]} | {r["status"]} | [{r.get("stage", "pending")}]({r["directory"]}/) |' for r in results]
    (output / "report.md").write_text("\n".join(lines) + "\n")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=False)
    parser.add_argument("--only", default=".*")
    parser.add_argument("--timeout", type=int, default=600)
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    if args.self_test:
        self_test()
        return 0
    if not args.output:
        parser.error("--output is required")
    if platform.system() != "Linux" or platform.machine() != "x86_64":
        parser.error("run inside pelagic-business-linux:local with the Linux Rust toolchain mounted")
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    environment = {k: v for k, v in os.environ.items() if not k.startswith(("KRAKEN_", "BELLHOP_"))}
    environment.update(CARGO_PROFILE_RELEASE_DEBUG_ASSERTIONS="true", CARGO_PROFILE_RELEASE_OVERFLOW_CHECKS="true")
    target = Path(environment.get("CARGO_TARGET_DIR", ROOT / "target"))
    metadata = {"commit": subprocess.check_output(["git", "rev-parse", "HEAD"], text=True).strip(),
                "dirty": subprocess.check_output(["git", "status", "--porcelain"], text=True),
                "platform": platform.platform(), "only": args.only, "timeout_seconds": args.timeout,
                "rustc": subprocess.check_output(["rustc", "-Vv"], text=True),
                "oracle_build": Path("/opt/reference-build.txt").read_text(),
                "oracle_sha256": {name: digest(Path(f"/usr/local/bin/{name}-fortran")) for name in ("kraken", "krakenc", "field", "bounce")},
                "source_sha256": {str(p.relative_to(ROOT)): digest(p) for p in sorted(ROOT.glob("crates/**/*.rs"))},
                "runner_sha256": digest(Path(__file__))}
    if not business.execute(["cargo", "build", "--locked", "--release", "-p", "cli"], output / "build.log", environment, args.timeout):
        raise RuntimeError("CLI build failed; see build.log")
    differential = business.test_binary("kraken", "differential_reference", environment, output / "build-differential.log", args.timeout)
    readback = business.test_binary("cli", "native_output", environment, output / "build-readback.log", args.timeout)
    binaries = output / "bin"
    binaries.mkdir()
    paths = [str(target / "release/pelagic"), differential, readback]
    for p in paths:
        shutil.copy2(p, binaries / Path(p).name)
    cli, differential, readback = [str(binaries / Path(p).name) for p in paths]
    metadata["executable_sha256"] = {Path(p).name: digest(Path(p)) for p in (cli, differential, readback)}
    shutil.copy2(__file__, output / "runner.py")
    metadata["helper_sha256"] = digest(Path(business.__file__))
    metadata["build_inputs_sha256"] = {str(p.relative_to(ROOT)): digest(p) for p in [ROOT / "Cargo.lock", ROOT / "Cargo.toml", ROOT / "rust-toolchain.toml", *ROOT.glob("crates/*/Cargo.toml")]}
    upstream = output / "upstream"
    shutil.copytree("/opt/acoustics-toolbox/tests", upstream)
    table_dir = output / "generated-tables"
    table_dir.mkdir()
    shutil.copy2(upstream / "TabRefCoef/neggradB.env", table_dir / "neggradB.env")
    if not business.execute(["/usr/local/bin/bounce-fortran", "neggradB"], table_dir / "bounce.log", environment, args.timeout, table_dir):
        raise RuntimeError("official table generation failed")
    for suffix in ("brc", "irc"):
        shutil.copy2(table_dir / f"neggradB.{suffix}", upstream / f"TabRefCoef/neggradC_{suffix}.{suffix}")
    cases = [(f"official/{p.relative_to(upstream)} [{flp.name}]", p, flp, engine) for p, flp, engine in official_cases(upstream)]
    fixtures = ROOT / "crates/kraken/tests/fixtures"
    metadata["modes_only_fixtures_excluded"] = [p.name for p in sorted(fixtures.glob("*.env")) if not p.with_suffix(".flp").exists()]
    cases += [(f"fixture/{p.name}", p, p.with_suffix(".flp"), engine) for p in sorted(fixtures.glob("*.env"))
              if p.with_suffix(".flp").exists() for engine in ("kraken", "krakenc")]
    cases.append(("fixture/ProfilesAdIncoherent", fixtures / "ProfilesAd.env", fixtures / "ProfilesAdIncoherent.flp", "kraken"))
    for stem in ("BroadBand/MunkK", "sduct/sductK"):
        cases.append((f"engine-control/{stem}", upstream / (stem + ".env"), upstream / (stem + ".flp"), "krakenc"))
    cases = [c for c in cases if re.search(args.only, c[0])]
    if not cases:
        raise ValueError("no selected cases")
    results = [{"case": name, "engine": engine, "status": "NOT_RUN", "directory": f"case-{i:03d}"}
               for i, (name, _, _, engine) in enumerate(cases)]
    report(output, metadata, results)
    for result, (_, source, flp, engine) in zip(results, cases):
        directory = output / result["directory"]
        directory.mkdir()
        started = time.monotonic()
        result.update(status="FAIL", stage="input", input_sha256={})
        try:
            if not source.exists() or not flp.exists():
                result["status"] = "MISSING_ORIGINAL_INPUT"
                continue
            # Short common stem avoids Fortran FileRoot truncation; bytes are unchanged.
            for p in [source, flp, *[p for p in source.parent.glob(source.stem + ".*") if p.suffix in (".brc", ".irc", ".trc", ".sbp")]]:
                shutil.copyfile(p, directory / ("case" + p.suffix))
                result["input_sha256"][str(p)] = digest(p)
            for p in flp.parent.glob(flp.stem + ".sbp"):
                shutil.copyfile(p, directory / "case.sbp")
                result["input_sha256"][str(p)] = digest(p)
            env = directory / "case.env"
            document = directory / "case.json"
            with document.open("wb") as stdout, (directory / "export.log").open("wb") as stderr:
                exported = subprocess.run([cli, "kraken", "export", str(env), "--solver", engine], env=environment,
                                          stdout=stdout, stderr=stderr, timeout=args.timeout).returncode == 0
            result["input_accepted"] = exported
            if not exported and result["case"].startswith("fixture/"):
                result["status"] = "UNSUPPORTED_INPUT"
                continue
            result["stage"] = "Fortran"
            for program in (engine, "field"):
                if not business.execute([f"/usr/local/bin/{program}-fortran", "case"], directory / f"{program}.log", environment, args.timeout, directory):
                    break
            else:
                if env.with_suffix(".mod").is_file() and env.with_suffix(".shd").is_file():
                    prt = env.with_suffix(".prt").read_text(errors="replace")
                    field_prt = (directory / "field.prt").read_text(errors="replace")
                    result["oracle_warnings"] = [line.strip() for line in (prt + field_prt).splitlines() if "warning" in line.lower()]
                    problem = oracle_problem(prt + field_prt)
                    if problem:
                        result["status"] = problem
                        continue
                    reference = pressures(env.with_suffix(".shd"))
                    result["oracle_nonfinite_printed_rows"] = len(re.findall(r"(?im)^.*\b(?:nan|infinity)\b.*$", prt))
                    result["oracle_nonfinite_pressure_components"] = sum(not math.isfinite(x) for block in reference for p in block for x in (p.real, p.imag))
                    result["oracle_sha256"] = {suffix: digest(env.with_suffix(suffix)) for suffix in (".mod", ".shd", ".prt")}
                    if result["oracle_nonfinite_printed_rows"] or result["oracle_nonfinite_pressure_components"]:
                        result["status"] = "ORACLE_NONFINITE"
                        continue
                    if not exported:
                        result["status"] = "UNSUPPORTED_INPUT"
                        continue
                    result["stage"] = "legacy API"
                    variables = dict(environment, KRAKEN_PROFILE_ENV=str(env), KRAKEN_PROFILE_ROOT=str(env.with_suffix("")),
                                     KRAKEN_PROFILE_SOLVER=engine, KRAKEN_JSON_INPUT=str(document), KRAKEN_JSON_ENV=str(env),
                                     KRAKEN_JSON_REFERENCE_ROOT=str(env.with_suffix("")))
                    if not business.check_test(differential, "profile_fields_match_fresh_reference", directory / "legacy-api.log", variables, args.timeout):
                        continue
                    result["stage"] = "JSON API"
                    if not business.check_test(differential, "json_fields_match_fresh_reference", directory / "json-api.log", variables, args.timeout):
                        continue
                    for label, input_path in (("legacy", env), ("json", document)):
                        result["stage"] = label + " CLI"
                        destination = directory / label
                        if not business.execute([cli, "kraken", "run", str(input_path), "--format", "both", "--output", str(destination),
                                                 *(["--solver", engine] if label == "legacy" else [])], directory / f"{label}-cli.log", environment, args.timeout):
                            break
                        product_vars = dict(variables, KRAKEN_HDF5_RESULT=str(destination / "case.h5"), KRAKEN_DIFFERENTIAL_ENV=str(input_path))
                        if not business.check_test(differential, "json_fields_match_fresh_reference", directory / f"{label}-hdf5.log", product_vars, args.timeout):
                            break
                        if not business.check_test(readback, "actual_cli_native_layouts_match_all_hdf5_values", directory / f"{label}-native.log", product_vars, args.timeout):
                            break
                        actual = [pressures(destination / ("case.shd" if len(reference) == 1 else f"case.f{i:04}.shd"))[0] for i in range(len(reference))]
                        result[label + "_metrics"] = [field_metrics(a, b) for a, b in zip(reference, actual)]
                    else:
                        result.update(status="PASS", stage="complete")
                    continue
            result["status"] = "ORACLE_FAILED"
        except Exception as error:
            result["error"] = f"{type(error).__name__}: {error}"
            if result["stage"] == "Fortran":
                result["status"] = "ORACLE_FAILED"
        finally:
            result["seconds"] = round(time.monotonic() - started, 3)
            report(output, metadata, results)
            print(result["status"], result["engine"], result["case"], result["stage"], flush=True)
    return int(any(r["status"] != "PASS" for r in results))


def self_test():
    import tempfile
    with tempfile.TemporaryDirectory() as temporary:
        root = Path(temporary)
        script = root / "runtests.m"
        script.write_text("kraken('actual')\n% krakenc('comment')\nkrakenc( 'complex' )\n")
        discovered = official_cases(root)
        assert (root / "actual.env", root / "actual.flp", "kraken") in discovered
        assert (root / "complex.env", root / "complex.flp", "krakenc") in discovered
        assert all(p.name != "comment.env" for p, _, _ in discovered)
        record = 192
        data = bytearray(record * 12)
        struct.pack_into("<i", data, 0, record // 4)
        struct.pack_into("<7i", data, 2 * record, 2, 1, 1, 1, 1, 1, 1)
        struct.pack_into("<ff", data, 10 * record, 1.0, 2.0)
        struct.pack_into("<ff", data, 11 * record, 3.0, 4.0)
        path = root / "case.shd"
        path.write_bytes(data)
        assert pressures(path) == [[1 + 2j], [3 + 4j]]
        assert field_metrics([1 + 2j], [1 + 2j])["complex_max"] == 0.0
        assert field_metrics([3 + 4j], [3 + 5j])["complex_max"] == 1.0
        assert field_metrics([0j], [1j])["one_sided_zeros"] == 1
        try:
            field_metrics([1j], [complex(float("nan"), 0)])
        except ValueError:
            pass
        else:
            raise AssertionError("nonfinite pressure accepted")
    assert oracle_problem(" *** FATAL ERROR ***\nNo modes") == "ORACLE_FAILED"
    assert oracle_problem("Warning: Inverse iteration failed to converge") == "ORACLE_UNCONVERGED"
    assert oracle_problem("Warning: Too many meshes needed: check convergence") == "ORACLE_UNCONVERGED"
    assert oracle_problem("Warning: roughness exceeds the region of validity") is None
    print("KRAKEN business self-test passed")


if __name__ == "__main__":
    raise SystemExit(main())

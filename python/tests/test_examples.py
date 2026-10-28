"""Keep examples/ working: every case is current and runs, and every script runs."""
import json
import os
import shutil
import subprocess
import sys
import warnings
from pathlib import Path

import pytest

import pelagic as pg

ROOT = Path(__file__).resolve().parents[2]
EXAMPLES = ROOT / "examples"
CASES = EXAMPLES / "cases"
CATALOG = json.loads((CASES / "catalog.json").read_text(encoding="utf-8"))["cases"]
SCRIPTS = sorted((EXAMPLES / "python").glob("*.py"))


def load(case, key):
    if case["model"] == "bellhop":
        return pg.bellhop.load(CASES / case[key])
    if key == "json":
        return pg.kraken.load(CASES / case["json"])
    return pg.kraken.load(CASES / case["env"], flp=CASES / case["flp"], solver=case["solver"])


@pytest.mark.parametrize("case", CATALOG, ids=lambda case: f"{case['model']}/{case['name']}")
def test_case_json_matches_its_legacy_files_and_runs(case):
    with warnings.catch_warnings():
        warnings.simplefilter("ignore", pg.PelagicWarning)
        document = load(case, "json")
        legacy = load(case, "env")
    assert document.to_dict() == legacy.to_dict(), "regenerate with: python3 tools/export-examples.py"
    result = document.run()
    if case["model"] == "kraken":
        assert all(block.profiles[0].modes.wavenumbers.size for block in result.frequencies)
    else:
        assert result.field is not None or result.rays or result.eigenrays or result.arrivals is not None


def test_catalog_lists_every_case_once_and_the_readme_names_them():
    folders = {path.relative_to(CASES).as_posix() for path in CASES.glob("*/*") if path.is_dir()}
    assert folders == {Path(case["env"]).parent.as_posix() for case in CATALOG}
    documents = {path.relative_to(CASES).as_posix() for path in CASES.glob("*/*/*.json")}
    assert documents == {case["json"] for case in CATALOG}
    assert len({(case["model"], case["name"]) for case in CATALOG}) == len(CATALOG)
    readme = (EXAMPLES / "README.md").read_text(encoding="utf-8")
    missing = [case["name"] for case in CATALOG if f"`{case['name']}`" not in readme]
    assert not missing, f"examples/README.md does not describe {missing}"
    missing = [script.name for script in SCRIPTS if script.name not in readme]
    assert not missing, f"examples/README.md does not list {missing}"


@pytest.mark.parametrize("script", SCRIPTS, ids=lambda script: script.stem)
def test_python_example_runs(script, tmp_path):
    pytest.importorskip("matplotlib")
    if script.stem == "arlpy_backend":
        pytest.importorskip("arlpy")
    environment = dict(os.environ, MPLBACKEND="Agg")
    completed = subprocess.run([sys.executable, str(script)], cwd=tmp_path, env=environment,
                               capture_output=True, text=True, timeout=600)
    assert completed.returncode == 0, completed.stdout[-2000:] + completed.stderr[-4000:]
    if "savefig" in script.read_text(encoding="utf-8"):
        assert list((tmp_path / "results").glob("*.png"))


@pytest.mark.skipif(not os.environ.get("PELAGIC_CLI"), reason="set PELAGIC_CLI to run the CLI examples")
def test_cli_examples_run(tmp_path):
    if sys.platform == "win32":
        shell = shutil.which("pwsh") or shutil.which("powershell")
        command = [shell, "-NoProfile", "-ExecutionPolicy", "Bypass", "-File",
                   str(EXAMPLES / "cli" / "run-all.ps1"), "-Output", "results/cli"]
        subprocess.run(command, cwd=tmp_path, check=True, timeout=900)
    else:
        for script in ("run-all.sh", "pipeline.sh"):
            subprocess.run(["bash", str(EXAMPLES / "cli" / script)], cwd=tmp_path, check=True, timeout=900)
    produced = {path.parent.name for path in (tmp_path / "results/cli").glob("*/*/*.prt")}
    assert produced >= {case["name"] for case in CATALOG}
    pytest.importorskip("h5py")
    pytest.importorskip("matplotlib")
    results = tmp_path / "results/cli"
    subprocess.run([sys.executable, str(EXAMPLES / "cli" / "read_hdf5.py"),
                    str(results / "bellhop/munk-tl/munk-tl.h5"), str(results / "kraken/munk-modes/munk-modes.h5")],
                   cwd=tmp_path, env=dict(os.environ, MPLBACKEND="Agg"), check=True, timeout=300)


def sweep_status(folder, *listing):
    return subprocess.run([sys.executable, str(EXAMPLES / "cli" / "sweep-status.py"), "results", *listing],
                          cwd=folder, capture_output=True, text=True, timeout=60)


@pytest.mark.skipif(not os.environ.get("PELAGIC_CLI"), reason="set PELAGIC_CLI to run the CLI examples")
@pytest.mark.skipif(sys.platform == "win32", reason="a bash job script")
def test_slurm_sweep_runs_shard_by_shard_and_resumes(tmp_path):
    # The job array's tasks, run in sequence as Slurm would run them side by side.
    cases = sorted(str(CASES / case["json"]) for case in CATALOG if case["model"] == "kraken")
    (tmp_path / "cases.txt").write_text("\n".join(cases) + "\n", encoding="utf-8")

    def task(index):
        environment = dict(os.environ, SWEEP_SHARDS="3", SLURM_ARRAY_TASK_ID=str(index))
        subprocess.run(["bash", str(EXAMPLES / "cli" / "slurm-sweep.sh"), "kraken", "cases.txt", "results"],
                       cwd=tmp_path, env=environment, check=True, timeout=900, stdout=subprocess.DEVNULL)

    def records(index):
        path = tmp_path / f"results/status-{index}-of-3.ndjson"
        return [json.loads(line) for line in path.read_text(encoding="utf-8").splitlines()]

    task(0)
    waiting = sweep_status(tmp_path, "cases.txt")
    assert waiting.returncode == 1 and f"{len(cases) - len(records(0))} of {len(cases)} listed" in waiting.stdout
    task(1)
    task(2)
    assert sorted(record["case"] for index in range(3) for record in records(index)) == cases
    assert all(record["outcome"] == "ok" for index in range(3) for record in records(index))
    assert all((tmp_path / "results" / Path(case).stem / "pelagic-manifest.json").is_file() for case in cases)
    done = sweep_status(tmp_path, "cases.txt")
    assert done.returncode == 0, done.stdout
    assert done.stdout.startswith(f"{len(cases)} cases recorded: {len(cases)} done, 0 failed")
    # A resubmitted task skips the cases it finished.
    before = len(records(1))
    task(1)
    assert [record["outcome"] for record in records(1)[before:]] == ["skipped"] * before


@pytest.mark.skipif(not os.environ.get("PELAGIC_CLI"), reason="set PELAGIC_CLI to run the CLI examples")
def test_sweep_status_reports_a_case_whose_status_files_disagree(tmp_path):
    # A one-machine run continued as a sharded run: lines carry no time, so
    # neither file's line can be taken as the newer.
    case = tmp_path / "cases" / "pekeris.json"
    case.parent.mkdir()
    shutil.copy(CASES / "kraken/pekeris/pekeris.json", case)

    def run_many(*shard):
        command = [os.environ["PELAGIC_CLI"], "kraken", "run-many", str(case), "--output", "results", *shard]
        return subprocess.run([*command, "--resume"], cwd=tmp_path, capture_output=True, timeout=300).returncode

    assert run_many() == 0
    assert run_many("--shard", "0/1") == 0  # skipped, so done in both files
    agreed = sweep_status(tmp_path)
    assert agreed.returncode == 0 and agreed.stdout == "1 cases recorded: 1 done, 0 failed\n"
    case.write_text('{"schema_version": 1}', encoding="utf-8")
    assert run_many("--shard", "0/1") != 0
    disagreed = sweep_status(tmp_path)
    assert disagreed.returncode == 1
    assert disagreed.stdout.splitlines() == [
        "1 cases recorded: 0 done, 0 failed, 1 unclear",
        f"unclear {case}: failed in status-0-of-1.ndjson, done in status.ndjson",
    ]


@pytest.mark.skipif(not os.environ.get("PELAGIC_CLI"), reason="set PELAGIC_CLI to run the CLI examples")
def test_sweep_status_ignores_a_line_cut_inside_a_character(tmp_path):
    case = CASES / "kraken/pekeris/pekeris.json"
    subprocess.run([os.environ["PELAGIC_CLI"], "kraken", "run-many", str(case), "--output", "results"],
                   cwd=tmp_path, capture_output=True, check=True, timeout=300)
    # A task killed while writing a case path with an é, after its first byte.
    with open(tmp_path / "results/status.ndjson", "ab") as status:
        status.write('{"status_version":1,"case":"/cases/é.json"'.encode("utf-8")[:-7])
    completed = sweep_status(tmp_path)
    assert completed.returncode == 0 and completed.stdout == "1 cases recorded: 1 done, 0 failed\n", completed.stderr

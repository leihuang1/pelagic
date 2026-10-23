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
CATALOG = json.loads((CASES / "catalog.json").read_text())["cases"]
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
    readme = (EXAMPLES / "README.md").read_text()
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
    if "savefig" in script.read_text():
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

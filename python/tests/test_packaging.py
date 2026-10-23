import hashlib
import importlib.util
import pytest
from test_bindings import ROOT


def test_distribution_versions_and_schema_identity_match():
    import json
    import pelagic as pg
    from test_bindings import cli
    assert (ROOT / "matlab/VERSION").read_text().strip() == pg.__version__
    assert f'version: "{pg.__version__}"' in (ROOT / "CITATION.cff").read_text()
    version = json.loads(cli("version", "--json").stdout)
    assert version["version"] == pg.__version__
    assert version["case_schema_versions"] == pg.case_schema_versions


def test_bundled_dependency_license_texts_are_present():
    import json
    from importlib.resources import files
    for path in [files("pelagic").joinpath("THIRD-PARTY-LICENSES.json"), ROOT / "packaging/cli-licenses.json"]:
        packages = json.loads(path.read_text(encoding="utf-8"))["third_party_libraries"]
        assert len(packages) > 20
        for package in packages:
            assert package["licenses"], package["package_name"]
            for license in package["licenses"]:
                assert len(license["text"]) > 100 and "NOT FOUND" not in license["text"]


def test_matlab_staging_checks_every_platform_before_copying(tmp_path):
    spec = importlib.util.spec_from_file_location("stage", ROOT / "tools/stage-matlab.py")
    stage = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(stage)
    for platform in stage.PLATFORMS:
        source = tmp_path / ("cli-" + platform)
        source.mkdir()
        names = ["pelagic.exe" if platform == "windows-x64" else "pelagic", "LICENSE", "NOTICE", "THIRD-PARTY-LICENSES.json"]
        for name in names:
            (source / name).write_bytes(b"fixture")
        digest = hashlib.sha256(b"fixture").hexdigest()
        (source / "SHA256SUMS").write_text("".join(f"{digest}  {name}\n" for name in names))
    target = tmp_path / "staged"
    stage.stage(tmp_path, target)
    assert len(list(target.glob("*/pelagic*"))) == 5
    (tmp_path / "cli-windows-x64/pelagic.exe").write_bytes(b"changed")
    with pytest.raises(ValueError, match="checksum mismatch"):
        stage.stage(tmp_path, tmp_path / "failed")
    assert not (tmp_path / "failed").exists()


def test_matlab_array_fields_match_the_json_contract():
    """MATLAB encodes these members as arrays even when a user assigns a scalar."""
    import json
    import re
    from test_bindings import cli
    listed = set(re.findall(r"'([a-z0-9_]+)'", (ROOT / "matlab/+pelagic/+internal/array_fields.m").read_text()))
    # Authoritative: flat numeric vectors in the serde types behind each JSON v1 schema.
    declared = set()
    for source in ("crates/bellhop/src/input/json.rs", "crates/kraken/src/input/json.rs", "crates/kraken/src/model.rs"):
        declared |= set(re.findall(r"pub ([a-z0-9_]+): Vec<f64>,", (ROOT / source).read_text()))
    assert listed == declared
    # Observed: every exported curated document agrees, and no listed name is ever a scalar.
    arrays, scalars = set(), set()
    def walk(value, key=None):
        if isinstance(value, dict):
            for name, item in value.items():
                walk(item, name)
        elif isinstance(value, list):
            if value and all(isinstance(x, (int, float)) and not isinstance(x, bool) for x in value):
                arrays.add(key)
            for item in value:
                walk(item)
        elif isinstance(value, (int, float)) and not isinstance(value, bool):
            scalars.add(key)
    exported = 0
    for model, folder, solvers in (("bellhop", "crates/bellhop/tests/fixtures", [[]]),
                                   ("kraken", "crates/kraken/tests/fixtures", [["--solver", "kraken"], ["--solver", "krakenc"]])):
        for env in sorted((ROOT / folder).rglob("*.env")):
            for solver in solvers:
                run = cli(model, "export", env, *solver)
                if run.returncode == 0:
                    walk(json.loads(run.stdout))
                    exported += 1
    assert exported > 300
    assert arrays - {None} <= listed
    assert not listed & scalars

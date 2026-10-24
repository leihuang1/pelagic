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

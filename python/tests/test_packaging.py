from test_bindings import ROOT


def test_distribution_versions_and_schema_identity_match():
    import json
    import pelagic as pg
    from test_bindings import cli
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


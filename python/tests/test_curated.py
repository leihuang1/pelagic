"""Every committed ENV in the curated input trees: either matching rejection or full readback.

The numerical reference assertions remain in Rust; these tests detect adapter changes.
"""
import json

import h5py
import numpy as np
import pytest
import pelagic as pg
from test_bindings import ROOT, cli, h5_pressure

BH = sorted((ROOT / "crates/bellhop/tests/fixtures/golden").glob("*.env"))
KR = sorted((ROOT / "crates/kraken/tests/fixtures").glob("*.env"))
CASES = [("bellhop", p, None) for p in BH] + [("kraken", p, s) for p in KR for s in ("kraken", "krakenc")]
assert len(BH) >= 38 and len(KR) >= 100


def compare_field(field, group, bellhop=False):
    expected = h5_pressure(group)
    if bellhop:
        if field.geometry == "irregular":
            expected = expected.reshape(field.pressure.shape)
        else:
            s, d, r = field.pressure.shape
            expected = expected.reshape(s, r, d).transpose(0, 2, 1)
    np.testing.assert_array_equal(field.pressure, expected)
    np.testing.assert_array_equal(field.source_depths_m, group["source_depth_m"][:])


def compare_bellhop(result, file):
    if result.field is not None:
        compare_field(result.field, file["field"], True)
    for name, rays in [("rays", result.rays), ("eigenrays", result.eigenrays)]:
        if not rays:
            continue
        group = file[name]
        columns = ["range_m", "depth_m", "travel_time_s", "attenuation_time_s", "amplitude", "phase_radians"]
        actual = np.concatenate([ray.points for ray in rays])
        for i, column in enumerate(columns):
            np.testing.assert_array_equal(actual[:, i], group[column][:])
        np.testing.assert_array_equal(np.cumsum([0] + [len(r.points) for r in rays]), group["point_offset"][:])
        for column in ("launch_angle_degrees", "top_bounces", "bottom_bounces"):
            np.testing.assert_array_equal([getattr(r, column) for r in rays], group[column][:])
    if "arrivals" in file:
        for name in ("amplitude", "phase_radians", "travel_time_s", "attenuation_time_s", "source_angle_degrees", "receiver_angle_degrees", "top_bounces", "bottom_bounces", "receiver_offset", "arrival_offset"):
            np.testing.assert_array_equal(getattr(result.arrivals, name), file["arrivals"][name][:])


def compare_kraken(result, file):
    np.testing.assert_array_equal([f.frequency_hz for f in result.frequencies], file["frequency_hz"][:])
    for i, frequency in enumerate(result.frequencies):
        group = file[f"frequencies/{i}"]
        compare_field(frequency.field, group["field"])
        for j, profile in enumerate(frequency.profiles):
            modes = profile.modes
            m = group["modes"] if j == 0 else group[f"profiles/{j}/modes"]
            np.testing.assert_array_equal(modes.depths_m, m["sample_depth_m"][:])
            np.testing.assert_array_equal(modes.wavenumbers, m["horizontal_wavenumber_real"][:] + 1j * m["horizontal_wavenumber_imaginary"][:])
            np.testing.assert_array_equal(modes.eigenfunctions, (m["eigenfunction_real"][:] + 1j * m["eigenfunction_imaginary"][:]).T)
            for name in ("phase_speed_mps", "group_speed_mps", "attenuation_nepers_per_m"):
                np.testing.assert_array_equal(getattr(modes, name), m[name][:])


@pytest.mark.parametrize("model,path,solver", CASES, ids=[f"{m}-{p.stem}-{s or ''}" for m, p, s in CASES])
def test_curated_case(model, path, solver, tmp_path):
    module = getattr(pg, model)
    args = [] if solver is None else ["--solver", solver]
    exported = cli(model, "export", path, *args)
    try:
        case = module.load(path, **({} if solver is None else {"solver": solver}))
    except pg.ValidationError as error:
        assert exported.returncode == 2
        reports = [json.loads(line) for line in exported.stderr.splitlines()]
        assert any(error.code == d["code"] for report in reports for d in report["diagnostics"])
        return
    assert exported.returncode == 0, exported.stderr
    assert case.to_dict() == module.Case.from_dict(json.loads(exported.stdout)).to_dict()
    text = json.dumps(case.to_dict(), separators=(",", ":"))
    ran = cli(model, "run", "-", "--format", "both", "--output", tmp_path / "cli", input=text)
    try:
        result = case.run()
    except pg.SimulationError as error:
        assert ran.returncode == 3, ran.stderr
        assert error.code in ran.stderr
        return
    assert ran.returncode == 0, ran.stderr
    with h5py.File(tmp_path / "cli/stdin.h5") as file:
        (compare_bellhop if model == "bellhop" else compare_kraken)(result, file)
    result.write_native(tmp_path / "native")
    for file in (tmp_path / "cli").iterdir():
        if file.suffix in (".ray", ".arr", ".shd", ".mod"):
            other = tmp_path / "native" / file.name.replace("stdin", "case", 1)
            assert file.read_bytes() == other.read_bytes(), file.name

import json
import os
from pathlib import Path
import subprocess

import h5py
import numpy as np
import pytest
import pelagic as pg

ROOT = Path(__file__).resolve().parents[2]
CLI = Path(os.environ.get("PELAGIC_CLI", ROOT / "target/debug/pelagic"))


def cli(*args, input=None):
    return subprocess.run([str(CLI), "--diagnostics", "json", *map(str, args)],
                          input=input, capture_output=True, text=True)


def h5_pressure(group):
    return group["pressure_real"][:] + 1j * group["pressure_imaginary"][:]


def test_bellhop_in_process_matches_cli_and_is_immutable(tmp_path):
    case = pg.bellhop.load(ROOT / "examples/cases/bellhop/field-g/field-g.json")
    document = case.to_dict()
    # Exercise all three axes, not a convenient singleton-only reshape.
    document["source"]["depth"] = [20., 30.]
    document["receivers"].update(depth=[10., 40., 60.], range=[100., 300., 600., 900.])
    case = pg.bellhop.Case.from_dict(document)
    result = case.run()
    assert result.field.pressure.shape == (2, 3, 4)
    assert result.field.pressure.dtype == np.complex64
    with pytest.raises(ValueError):
        result.field.pressure.setflags(write=True)
    with pytest.raises(ValueError):
        result.field.pressure[0, 0, 0] = 0
    run = cli("bellhop", "run", "-", "--format", "hdf5", "--output", tmp_path / "cli", input=json.dumps(document))
    assert run.returncode == 0, run.stderr
    with h5py.File(tmp_path / "cli/stdin.h5") as f:
        expected = h5_pressure(f["field"]).reshape(2, 4, 3).transpose(0, 2, 1)
        np.testing.assert_array_equal(result.field.pressure, expected)
    result.write_native(tmp_path / "native")
    assert (tmp_path / "native/case.shd").exists()
    assert json.loads((tmp_path / "native/case.json").read_text()) == case.to_dict()
    assert pg.bellhop.Case.from_dict(case.to_dict()).to_dict() == case.to_dict()
    with np.errstate(divide="ignore"):
        np.testing.assert_array_equal(result.field.transmission_loss(), -20 * np.log10(abs(result.field.pressure)))


def test_kraken_keeps_frequency_order_duplicates_and_each_profiles_modes(tmp_path):
    document = pg.kraken.load(ROOT / "examples/cases/kraken/pekeris/pekeris.json").to_dict()
    document["frequency"] *= 2
    case = pg.kraken.Case.from_dict(document)
    result = case.run()
    assert len(result.frequencies) == 2
    run = cli("kraken", "run", "-", "--format", "hdf5", "--output", tmp_path / "cli", input=json.dumps(document))
    assert run.returncode == 0, run.stderr
    with h5py.File(tmp_path / "cli/stdin.h5") as f:
        for i, frequency in enumerate(result.frequencies):
            g = f[f"frequencies/{i}"]
            np.testing.assert_array_equal(frequency.field.pressure, h5_pressure(g["field"]))
            m = g["modes"]
            modes = frequency.profiles[0].modes
            np.testing.assert_array_equal(modes.wavenumbers, m["horizontal_wavenumber_real"][:] + 1j * m["horizontal_wavenumber_imaginary"][:])
            np.testing.assert_array_equal(modes.eigenfunctions, (m["eigenfunction_real"][:] + 1j * m["eigenfunction_imaginary"][:]).T)
    result.write_native(tmp_path / "native")
    assert len(list((tmp_path / "native").glob("*.mod"))) == 2


def test_validation_is_structured_and_matches_cli():
    document = pg.bellhop.load(ROOT / "examples/cases/bellhop/field-g/field-g.json").to_dict()
    document["frequency"] = -1
    with pytest.raises(pg.ValidationError) as caught:
        pg.bellhop.Case.from_dict(document)
    run = cli("bellhop", "validate", "-", input=json.dumps(document))
    expected = json.loads(run.stderr)["diagnostics"]
    assert [(d.code, d.severity, d.field) for d in caught.value.diagnostics] == [(d["code"], d["severity"], d["field"]) for d in expected]
    document["frequency"] = "not a number"
    with pytest.raises(pg.ValidationError) as caught:
        pg.bellhop.Case.from_dict(document)
    assert caught.value.code == "BH0103"
    assert caught.value.field == "frequency"


def test_kraken_case_is_rejected_when_its_dictionary_could_not_reload():
    # Read within the 1 MiB limit, but to_dict() writes `1500.0` for `1500` and outgrows it.
    document = {"pelagic": 2, "model": "kraken", "frequency": 50, "interpolation": "c_linear",
                "media": [{"to": 65000, "ssp": [[depth, 1500] for depth in range(65001)]}],
                "surface": "vacuum", "bottom": {"type": "halfspace", "cp": 1700, "rho": 1.5},
                "source": {"depth": 25}, "receivers": {"depth": 50, "range": 1000},
                "kraken": {"solver": "kraken", "phase_speed": [1400, 1700], "max_range": 1000}}
    assert len(json.dumps(document, separators=(",", ":"))) < 1 << 20
    with pytest.raises(pg.ValidationError) as caught:
        pg.kraken.Case.from_dict(document)
    assert (caught.value.code, caught.value.field) == ("KR0101", "json")
    document["media"][0]["ssp"] = [[0, 1500], [65000, 1500]]
    case = pg.kraken.Case.from_dict(document)
    assert pg.kraken.Case.from_dict(case.to_dict()).to_dict() == case.to_dict()


def test_irregular_field_uses_paired_receivers():
    document = pg.bellhop.load(ROOT / "examples/cases/bellhop/field-g/field-g.json").to_dict()
    document["receivers"].update(grid="irregular", range=[100., 300., 900.], depth=[10., 40., 60.])
    result = pg.bellhop.Case.from_dict(document).run()
    assert result.field.pressure.shape == (len(document["source"]["depth"]), 3)
    assert result.to_xarray().dims == ("source_depth", "receiver")


def test_run_many_is_ordered_and_releases_threads():
    from threading import enumerate as threads
    case = pg.bellhop.load(ROOT / "examples/cases/bellhop/field-g/field-g.json")
    with pg.bellhop.run_many([case] * 3, workers=2) as results:
        first = next(results)
        np.testing.assert_array_equal(first.field.pressure, next(results).field.pressure)
    assert not any(t.name.startswith("pelagic") for t in threads())


def test_native_publication_protects_modified_outputs(tmp_path):
    result = pg.bellhop.load(ROOT / "examples/cases/bellhop/field-g/field-g.json").run()
    result.write_native(tmp_path / "native")
    target = tmp_path / "native/case.shd"
    target.write_bytes(b"user edit")
    with pytest.raises(pg.OutputError):
        result.write_native(tmp_path / "native")
    assert target.read_bytes() == b"user edit"


def test_run_many_reports_failed_input_and_joins_all_workers():
    from threading import enumerate as threads
    case = pg.bellhop.load(ROOT / "examples/cases/bellhop/field-g/field-g.json")
    document = case.to_dict()
    document["bellhop"]["beam"] = "geometric_gaussian_ray_centered"
    unsupported = pg.bellhop.Case.from_dict(document)
    with pytest.raises(pg.SimulationError) as caught:
        list(pg.bellhop.run_many([case, unsupported, case], workers=2))
    assert caught.value.case_index == 1
    assert caught.value.code == "BH0301"
    assert not any(t.name.startswith("pelagic") for t in threads())


@pytest.mark.skipif(os.name == "nt", reason="POSIX SIGINT integration test")
def test_ctrl_c_cancels_running_native_work_without_detached_threads():
    import signal
    import sys
    script = '''
import signal, threading
import numpy as np
import pelagic as pg
# Background shell test runners inherit SIG_IGN; exercise a normal interactive handler.
signal.signal(signal.SIGINT, signal.default_int_handler)
c = pg.bellhop.load("examples/cases/bellhop/field-g/field-g.json").to_dict()
c["bellhop"]["angles"] = np.linspace(-15, 15, 10001)
c["receivers"]["range"] = np.linspace(50, 1000, 10000)
c["receivers"]["depth"] = np.linspace(10, 90, 100)
c = pg.bellhop.Case.from_dict(c)
class Observed:
    def _run_native(self, token):
        print("ready", flush=True)
        return c._run_native(token)
try:
    with pg.bellhop.run_many([Observed()], workers=1) as results:
        next(results)
except KeyboardInterrupt:
    assert not any(t.name.startswith("pelagic") for t in threading.enumerate())
    assert signal.getsignal(signal.SIGINT) is signal.default_int_handler
    print("cancelled", flush=True)
else:
    raise AssertionError("long solve was not interrupted")
'''
    child = subprocess.Popen([sys.executable, "-c", script], cwd=ROOT,
                             stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
    try:
        assert child.stdout.readline().strip() == "ready"
        # The worker, not the loader/main thread, announces the start of native work.
        child.send_signal(signal.SIGINT)
        out, err = child.communicate(timeout=15)
        assert child.returncode == 0, err
        assert "cancelled" in out
    finally:
        if child.poll() is None:
            child.kill()
            child.wait()


def test_interrupt_during_successful_cleanup_is_not_swallowed(monkeypatch):
    from concurrent.futures import ThreadPoolExecutor
    original = ThreadPoolExecutor.shutdown
    interrupted = []
    def shutdown(executor, *args, **kwargs):
        original(executor, *args, **kwargs)
        if not interrupted:
            interrupted.append(True)
            raise KeyboardInterrupt
    monkeypatch.setattr(ThreadPoolExecutor, "shutdown", shutdown)
    with pytest.raises(KeyboardInterrupt):
        pg.bellhop.load(ROOT / "examples/cases/bellhop/field-g/field-g.json").run()


def test_tagged_json_error_field_does_not_depend_on_object_key_order():
    from collections import OrderedDict
    case = pg.kraken.load(ROOT / "examples/cases/kraken/pekeris/pekeris.json").to_dict()
    errors = []
    for pairs in [[("type", "halfspace"), ("extra", 1)],
                  [("extra", 1), ("type", "halfspace")]]:
        case["bottom"] = OrderedDict(pairs)
        with pytest.raises(pg.ValidationError) as caught:
            pg.kraken.Case.from_dict(case)
        errors.append((caught.value.code, caught.value.field))
    assert errors[0] == errors[1] == ("KR0103", "bottom")


def test_cancelled_call_has_stable_code_and_restores_thread_scope():
    from pelagic import _core
    from pelagic.errors import call
    case = pg.bellhop.load(ROOT / "examples/cases/bellhop/field-g/field-g.json")
    token = _core.Cancellation()
    token.cancel()
    with pytest.raises(pg.CancelledError) as caught:
        call(case._native.run, token)
    assert caught.value.code == "BH0304"
    assert case.run().field is not None


def test_failure_index_does_not_depend_on_done_callback_timing(monkeypatch):
    # wait() may return before done-callbacks run; the main loop must still attribute the failure.
    from concurrent.futures import Future
    monkeypatch.setattr(Future, "add_done_callback", lambda self, fn: None)
    case = pg.bellhop.load(ROOT / "examples/cases/bellhop/field-g/field-g.json")
    document = case.to_dict()
    document["bellhop"]["beam"] = "geometric_gaussian_ray_centered"
    unsupported = pg.bellhop.Case.from_dict(document)
    with pytest.raises(pg.SimulationError) as caught:
        list(pg.bellhop.run_many([unsupported, case], workers=1))
    assert caught.value.case_index == 0


def test_load_warnings_point_at_the_caller():
    path = ROOT / "crates/bellhop/tests/fixtures/golden/ShadedField.env"
    with pytest.warns(pg.PelagicWarning) as record:
        case = pg.bellhop.load(path)
    assert {w.filename for w in record} == {__file__}
    with pytest.warns(pg.PelagicWarning) as record:
        pg.bellhop.Case.from_dict(case.to_dict())
    assert {w.filename for w in record} == {__file__}

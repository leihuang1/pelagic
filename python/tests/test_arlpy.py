import numpy as np
import pytest


def test_explicit_arlpy_backend_does_not_spawn_bellhop(monkeypatch):
    pm = pytest.importorskip("arlpy.uwapm")
    from pelagic.arlpy import register
    register()
    register()
    def no_binary(*args, **kwargs):
        raise AssertionError("external Bellhop invoked")
    monkeypatch.setattr(pm._Bellhop, "_bellhop", no_binary)
    env = pm.create_env2d(frequency=500, nbeams=101, rx_depth=[5, 10], rx_range=[100, 200, 300])
    field = pm.compute_transmission_loss(env, model="pelagic")
    assert field.shape == (2, 3)
    assert np.iscomplexobj(field.values)
    assert list(field.index) == [5, 10]
    assert list(field.columns) == [100, 200, 300]
    rays = pm.compute_rays(env, model="pelagic")
    assert len(rays) == 101
    arrivals = pm.compute_arrivals(env, model="pelagic")
    assert len(arrivals) > 0
    assert "arrival_amplitude" in arrivals

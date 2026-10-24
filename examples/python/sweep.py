"""Parameter sweeps in parallel with run_many: how source depth changes the field.

    python examples/python/sweep.py

Builds one case per source depth, runs them on four worker threads and collects
the results in input order. With xarray installed, the results are also stacked
into one labelled DataArray. Writes results/sweep.png.
"""
from pathlib import Path
import time

import matplotlib.pyplot as plt
import numpy as np

import pelagic as pg

CASES = Path(__file__).resolve().parent.parent / "cases"
OUT = Path("results")
OUT.mkdir(exist_ok=True)

base = pg.bellhop.load(CASES / "bellhop/munk-tl/munk-tl.json").to_dict()
depths = [100.0, 500.0, 1000.0, 2000.0]


def with_source_depth(depth):
    spec = dict(base, title=f"Munk, source at {depth:g} m")
    spec["source"] = dict(base["source"], depth=depth)
    return pg.bellhop.Case.from_dict(spec)


cases = [with_source_depth(depth) for depth in depths]
start = time.perf_counter()
fields = []
with pg.bellhop.run_many(cases, workers=4) as results:  # yields results in input order
    for depth, result in zip(depths, results):
        fields.append(result.field)
        print(f"source {depth:6.0f} m: mean TL {np.mean(np.minimum(result.field.transmission_loss(), 150)):.1f} dB")
print(f"{len(cases)} cases in {time.perf_counter() - start:.2f} s")

fig, axes = plt.subplots(2, 2, figsize=(11, 6), sharex=True, sharey=True)
for ax, depth, field in zip(axes.flat, depths, fields):
    image = ax.pcolormesh(field.ranges_m / 1e3, field.depths_m, np.minimum(field.transmission_loss()[0], 120),
                          cmap="jet_r", vmin=60, vmax=110, shading="auto")
    ax.set(title=f"source at {depth:g} m")
axes[0, 0].invert_yaxis()
for ax in axes[1]:
    ax.set_xlabel("Range (km)")
for ax in axes[:, 0]:
    ax.set_ylabel("Depth (m)")
fig.colorbar(image, ax=axes, label="TL (dB)")
fig.savefig(OUT / "sweep.png", dpi=130)

try:
    import xarray as xr

    stacked = xr.concat([field.to_xarray() for field in fields], dim="source_depth")
    print(stacked.sizes)
except ImportError:
    print("install xarray to stack the results into one labelled array")
plt.show()

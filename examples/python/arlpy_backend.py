"""Use Pelagic as the solver behind arlpy's uwapm module.

    pip install "pelagic[arlpy]"
    python examples/python/arlpy_backend.py

Existing arlpy scripts keep their environments and post-processing; only the
model name changes, and no BELLHOP executable is needed. Writes
results/arlpy_backend.png.
"""
from pathlib import Path

import arlpy.uwapm as pm
import matplotlib.pyplot as plt
import numpy as np

from pelagic.arlpy import register

OUT = Path("results")
OUT.mkdir(exist_ok=True)

register()  # adds the "pelagic" model to arlpy; safe to call more than once
print("models:", pm.models())

env = pm.create_env2d(depth=100, soundspeed=1500, bottom_soundspeed=1650, bottom_density=1900,
                      frequency=500, tx_depth=20, rx_depth=50, rx_range=2000, nbeams=501)
arrivals = pm.compute_arrivals(env, model="pelagic")
print(arrivals[["time_of_arrival", "arrival_amplitude", "surface_bounces", "bottom_bounces"]].head())

env["rx_depth"] = np.arange(1, 100, 1.0)
env["rx_range"] = np.arange(10, 5001, 10.0)
pressure = pm.compute_transmission_loss(env, model="pelagic")  # complex DataFrame [depth, range]
loss = -20 * np.log10(np.abs(pressure.values).clip(1e-12))

fig, ax = plt.subplots(figsize=(8, 3.5))
image = ax.pcolormesh(pressure.columns / 1e3, pressure.index, loss, cmap="jet_r", vmin=40, vmax=90, shading="auto")
ax.invert_yaxis()
ax.set(xlabel="Range (km)", ylabel="Depth (m)", title="arlpy environment solved by Pelagic")
fig.colorbar(image, label="TL (dB)")
fig.tight_layout()
fig.savefig(OUT / "arlpy_backend.png", dpi=150)
plt.show()

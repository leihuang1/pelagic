"""Range-dependent modes in the Gulf of Mexico: adiabatic versus coupled.

    python examples/python/range_dependent.py

The environment has eight sound-speed profiles along the track. Adiabatic modes
keep their energy as the environment changes; coupled modes exchange it. The two
FIELD files (gulf.flp, gulf-coupled.flp) choose the method. Writes
results/range_dependent.png.
"""
from pathlib import Path

import matplotlib.pyplot as plt
import numpy as np

import pelagic as pg

CASES = Path(__file__).resolve().parent.parent / "cases"
OUT = Path("results")
OUT.mkdir(exist_ok=True)

env = CASES / "kraken/gulf/gulf.env"
runs = {
    "adiabatic": pg.kraken.load(env).run(),  # gulf.flp, the same-stem default
    "coupled": pg.kraken.load(env, flp=CASES / "kraken/gulf/gulf-coupled.flp").run(),
}

fig, axes = plt.subplots(3, 1, figsize=(8, 10))
losses = {}
for ax, (method, result) in zip(axes, runs.items()):
    block = result.frequencies[0]
    print(f"{method}: {len(block.profiles)} profiles at ranges "
          f"{[round(p.range_m / 1e3, 1) for p in block.profiles]} km")
    field = block.field
    losses[method] = np.minimum(field.transmission_loss()[0], 120)
    image = ax.pcolormesh(field.ranges_m / 1e3, field.depths_m, losses[method], cmap="jet_r",
                          vmin=60, vmax=110, shading="auto")
    ax.invert_yaxis()
    ax.set(xlabel="Range (km)", ylabel="Depth (m)", title=f"{method} modes")
    fig.colorbar(image, ax=ax, label="TL (dB)")

row = np.argmin(abs(field.depths_m - 300.0))
for method, loss in losses.items():
    axes[2].plot(field.ranges_m / 1e3, loss[row], lw=0.8, label=method)
axes[2].invert_yaxis()
axes[2].set(xlabel="Range (km)", ylabel="TL (dB)", title="At 300 m depth")
axes[2].legend()

fig.tight_layout()
fig.savefig(OUT / "range_dependent.png", dpi=130)
plt.show()

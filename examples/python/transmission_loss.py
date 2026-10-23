"""Transmission loss in six BELLHOP environments, side by side.

    python examples/python/transmission_loss.py

Shows how bathymetry (.bty), an elastic bottom, a tabulated reflection coefficient
(.brc) and a directional source (.sbp) change the field. Writes
results/transmission_loss.png.
"""
from pathlib import Path

import matplotlib.pyplot as plt
import numpy as np

import pelagic as pg

CASES = Path(__file__).resolve().parent.parent / "cases"
OUT = Path("results")
OUT.mkdir(exist_ok=True)

NAMES = ["munk-tl", "seamount", "bottom-transition", "elastic-bottom", "reflection-table", "beam-pattern"]

fig, axes = plt.subplots(3, 2, figsize=(11, 10))
for ax, name in zip(axes.flat, NAMES):
    case = pg.bellhop.load(CASES / f"bellhop/{name}/{name}.env")
    result = case.run()
    field = result.field
    loss = np.minimum(field.transmission_loss()[0], 150)  # zero pressure -> clip
    top = np.percentile(loss, 5)  # scale each panel to its own field
    image = ax.pcolormesh(field.ranges_m / 1e3, field.depths_m, loss, cmap="jet_r",
                          vmin=top, vmax=top + 50, shading="auto")
    shape = case.to_dict()["bottom_boundary"].get("shape")
    if shape:  # bathymetry from the .bty file
        ax.plot([p["range_m"] / 1e3 for p in shape["points"]], [p["depth_m"] for p in shape["points"]], "k", lw=2)
        ax.set_xlim(field.ranges_m[0] / 1e3, field.ranges_m[-1] / 1e3)
    ax.invert_yaxis()
    ax.set(title=f"{name}: {result.frequency_hz:g} Hz", xlabel="Range (km)", ylabel="Depth (m)")
    fig.colorbar(image, ax=ax, label="TL (dB)")

fig.tight_layout()
fig.savefig(OUT / "transmission_loss.png", dpi=130)
plt.show()

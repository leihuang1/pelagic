"""Your first Pelagic run: transmission loss in the deep-water Munk sound channel.

    python examples/python/quickstart.py

Writes results/quickstart.png. Needs matplotlib (pip install "pelagic[examples]").
"""
from pathlib import Path

import matplotlib.pyplot as plt
import numpy as np

import pelagic as pg

CASES = Path(__file__).resolve().parent.parent / "cases"
OUT = Path("results")
OUT.mkdir(exist_ok=True)

# Load a case. A legacy .env file works the same way as this JSON file.
case = pg.bellhop.load(CASES / "bellhop/munk-tl/munk-tl.json")
result = case.run()

# The field's pressure is complex64 with axes [source, receiver depth, receiver range].
field = result.field
loss = field.transmission_loss()[0]  # first (only) source: [depth, range], dB
print(f"{result.title}: {loss.shape[0]} depths x {loss.shape[1]} ranges")
print(f"loss at 1000 m depth, 50 km range: {loss[40, 250]:.1f} dB")

fig, ax = plt.subplots(figsize=(8, 4))
# Shadow zones have zero pressure, so infinite loss: clip them to the colour scale.
image = ax.pcolormesh(field.ranges_m / 1e3, field.depths_m, np.minimum(loss, 120),
                      cmap="jet_r", vmin=60, vmax=110, shading="auto")
ax.invert_yaxis()
ax.set(xlabel="Range (km)", ylabel="Depth (m)", title=result.title)
fig.colorbar(image, label="Transmission loss (dB)")
fig.tight_layout()
fig.savefig(OUT / "quickstart.png", dpi=150)
plt.show()

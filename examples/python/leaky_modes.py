"""KRAKENC complex modes: trapped and leaky modes over a lossy bottom.

    python examples/python/leaky_modes.py

KRAKENC finds complex wavenumbers, so it includes modes that leak energy into the
bottom. The imaginary part of each wavenumber is its attenuation. Writes
results/leaky_modes.png.
"""
from pathlib import Path

import matplotlib.pyplot as plt
import numpy as np

import pelagic as pg

CASES = Path(__file__).resolve().parent.parent / "cases"
OUT = Path("results")
OUT.mkdir(exist_ok=True)

# The JSON names its solver; for legacy files pass solver="krakenc".
case = pg.kraken.load(CASES / "kraken/leaky-modes/leaky-modes.env", solver="krakenc")
block = case.run().frequencies[0]
modes = block.profiles[0].modes
k = modes.wavenumbers
loss_db_per_km = modes.attenuation_nepers_per_m * 8.686 * 1e3
print(f"{len(k)} complex modes; attenuation from {loss_db_per_km.min():.3f} to {loss_db_per_km.max():.1f} dB/km")

fig, (plane, decay, image_ax) = plt.subplots(3, 1, figsize=(8, 10))
plane.plot(k.real, k.imag, "o")
plane.set(xlabel="Re k (rad/m)", ylabel="Im k (rad/m)", title="Complex wavenumbers")

decay.semilogy(np.arange(1, len(k) + 1), loss_db_per_km, "o")
decay.set(xlabel="Mode number", ylabel="Attenuation (dB/km)", title="Higher modes leak into the bottom")

field = block.field
loss = np.minimum(field.transmission_loss()[0], 120)
image = image_ax.pcolormesh(field.ranges_m / 1e3, field.depths_m, loss, cmap="jet_r", vmin=50, vmax=100, shading="auto")
image_ax.invert_yaxis()
image_ax.set(xlabel="Range (km)", ylabel="Depth (m)", title="Transmission loss")
fig.colorbar(image, ax=image_ax, label="TL (dB)")

fig.tight_layout()
fig.savefig(OUT / "leaky_modes.png", dpi=130)
plt.show()

"""Normal modes with KRAKEN: mode shapes, dispersion and the field they build.

    python examples/python/modes.py

Uses the Munk sound channel (102 modes at 50 Hz). Writes results/modes.png.
"""
from pathlib import Path

import matplotlib.pyplot as plt
import numpy as np

import pelagic as pg

CASES = Path(__file__).resolve().parent.parent / "cases"
OUT = Path("results")
OUT.mkdir(exist_ok=True)

result = pg.kraken.load(CASES / "kraken/munk-modes/munk-modes.env").run()  # reads munk-modes.flp
block = result.frequencies[0]  # one entry per frequency
modes = block.profiles[0].modes  # one entry per range profile
print(f"{len(modes.wavenumbers)} modes at {block.frequency_hz:g} Hz")
print("mode  phase speed  group speed")
for index in range(5):
    print(f"{index + 1:4d}  {modes.phase_speed_mps[index]:9.2f}  {modes.group_speed_mps[index]:9.2f} m/s")

fig = plt.figure(figsize=(11, 7))
shapes = fig.add_subplot(2, 2, 1)
for index in (0, 1, 2, 9, 29):
    shapes.plot(modes.eigenfunctions[:, index].real, modes.depths_m, label=f"mode {index + 1}")
shapes.invert_yaxis()
shapes.set(xlabel="Mode shape", ylabel="Depth (m)", title="Mode shapes")
shapes.legend(fontsize=8)

speeds = fig.add_subplot(2, 2, 2)
number = np.arange(1, len(modes.wavenumbers) + 1)
speeds.plot(number, modes.phase_speed_mps, ".", label="phase speed")
speeds.plot(number, modes.group_speed_mps, ".", label="group speed")
speeds.set(xlabel="Mode number", ylabel="Speed (m/s)", title="Dispersion")
speeds.legend()

field = block.field
loss = np.minimum(field.transmission_loss()[0], 120)
ax = fig.add_subplot(2, 1, 2)
image = ax.pcolormesh(field.ranges_m / 1e3, field.depths_m, loss, cmap="jet_r", vmin=60, vmax=110, shading="auto")
ax.invert_yaxis()
ax.set(xlabel="Range (km)", ylabel="Depth (m)", title="Transmission loss from all modes")
fig.colorbar(image, ax=ax, label="TL (dB)")

fig.tight_layout()
fig.savefig(OUT / "modes.png", dpi=130)
plt.show()

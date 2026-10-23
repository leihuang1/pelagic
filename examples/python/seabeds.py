"""How boundaries shape normal modes: an interface wave, rough-surface scattering, a layered seabed.

    python examples/python/seabeds.py

1. Over an elastic sediment, KRAKEN finds an interface (Scholte) wave: mode 1
   travels slower than sound in water and is trapped at the seabed.
2. A 5 m RMS rough surface scatters energy out of the modes. Editing the case to
   remove the roughness gives the smooth reference.
3. A layered seabed: the field over two lossy sediment layers.
Writes results/seabeds.png.
"""
from pathlib import Path

import matplotlib.pyplot as plt
import numpy as np

import pelagic as pg

CASES = Path(__file__).resolve().parent.parent / "cases"
OUT = Path("results")
OUT.mkdir(exist_ok=True)

fig = plt.figure(figsize=(11, 8))

# 1. Interface wave over an elastic sediment
modes = pg.kraken.load(CASES / "kraken/elastic-sediment/elastic-sediment.json").run().frequencies[0].profiles[0].modes
print(f"elastic sediment: mode 1 at {modes.phase_speed_mps[0]:.0f} m/s, mode 2 at {modes.phase_speed_mps[1]:.0f} m/s")
shapes = fig.add_subplot(2, 2, 1)
for index in (0, 1):
    shapes.plot(modes.eigenfunctions[:, index].real, modes.depths_m,
                label=f"mode {index + 1}: {modes.phase_speed_mps[index]:.0f} m/s")
shapes.axhline(5000, color="saddlebrown", lw=1)
shapes.set(ylim=(5000, 3500), xlabel="Mode shape", ylabel="Depth (m)", title="Interface wave at an elastic seabed")
shapes.legend(fontsize=8)

# 2. Rough versus smooth surface
rough = pg.kraken.load(CASES / "kraken/rough-surface/rough-surface.json")
spec = rough.to_dict()
for profile in spec["frequencies"][0]["profiles"]:
    profile["interface_roughness_m"] = [0.0] * len(profile["interface_roughness_m"])
smooth = pg.kraken.Case.from_dict(spec)
average = fig.add_subplot(2, 2, 2)
for label, case in (("smooth surface", smooth), ("5 m RMS rough surface", rough)):
    field = case.run().frequencies[0].field
    depth_averaged = -10 * np.log10(np.mean(abs(field.pressure[0]) ** 2, axis=0))  # over all depths
    average.plot(field.ranges_m / 1e3, depth_averaged, label=label)
    print(f"{label}: depth-averaged TL at 200 km {depth_averaged[-1]:.1f} dB")
average.invert_yaxis()
average.set(xlabel="Range (km)", ylabel="Depth-averaged TL (dB)", title="Scattering loss, 10 Hz")
average.legend()

# 3. Layered seabed
block = pg.kraken.load(CASES / "kraken/layered-seabed/layered-seabed.json").run().frequencies[0]
field = block.field
ax = fig.add_subplot(2, 1, 2)
image = ax.pcolormesh(field.ranges_m / 1e3, field.depths_m, np.minimum(field.transmission_loss()[0], 120),
                      cmap="jet_r", vmin=40, vmax=90, shading="auto")
for interface in (70.0, 105.0):  # water/sediment and sediment/sediment interfaces
    ax.axhline(interface, color="white", lw=0.8, ls="--")
ax.invert_yaxis()
ax.set(xlabel="Range (km)", ylabel="Depth (m)",
       title=f"Layered seabed, {block.frequency_hz:g} Hz ({len(block.profiles[0].modes.wavenumbers)} modes)")
fig.colorbar(image, ax=ax, label="TL (dB)")

fig.tight_layout()
fig.savefig(OUT / "seabeds.png", dpi=130)
plt.show()

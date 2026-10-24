"""Build a BELLHOP case from scratch, without any Acoustics Toolbox files.

    python examples/python/build_case.py

A shallow-water summer profile over a sand bottom, written as a plain dictionary
in the JSON format (docs/bellhop/json.md). It then samples the field along a
glider's saw-tooth track with an irregular receiver grid. Writes
results/build_case.png and results/shallow-water.json.
"""
import json
from pathlib import Path

import matplotlib.pyplot as plt
import numpy as np

import pelagic as pg

OUT = Path("results")
OUT.mkdir(exist_ok=True)

spec = {
    "pelagic": 2,
    "model": "bellhop",
    "title": "Shallow water: summer thermocline over sand",
    "frequency": 1000.0,
    "interpolation": "c_linear",
    "attenuation": {"volume": "thorp"},  # sea-water absorption; losses in dB/wavelength
    "media": [{"to": 120.0, "ssp": [[0, 1530], [20, 1528], [40, 1505], [120, 1500]]}],  # [depth, cp]
    "surface": "vacuum",
    "bottom": {"type": "halfspace", "cp": 1650.0, "rho": 1.9, "ap": 0.8},  # sand; rho in g/cm3
    "source": {"depth": 30.0},
    "receivers": {"depth": {"from": 0, "to": 120, "count": 241},
                  "range": {"from": 0, "to": 10, "count": 501, "unit": "km"}},
    "bellhop": {"run": "coherent", "beam": "geometric_gaussian_cartesian",
                "angles": {"from": -60, "to": 60, "count": 1201},
                "box": {"depth": 121.0, "range": 10.1e3}},
}

case = pg.bellhop.Case.from_dict(spec)
(OUT / "shallow-water.json").write_text(json.dumps(case.to_dict(), indent=2))
field = case.run().field

# The same environment, sampled only along a glider track: paired ranges and depths.
track_range = np.linspace(200, 10e3, 400)
track_depth = 10 + 100 * np.abs((track_range / 2000) % 2 - 1)  # saw-tooth between 10 and 110 m
glider = dict(spec, receivers={"grid": "irregular", "range": track_range, "depth": track_depth})
along_track = pg.bellhop.Case.from_dict(glider).run().field
print(f"glider field: geometry={along_track.geometry}, pressure shape {along_track.pressure.shape}")

fig, (image_ax, track_ax) = plt.subplots(2, 1, figsize=(8, 7))
image = image_ax.pcolormesh(field.ranges_m / 1e3, field.depths_m, np.minimum(field.transmission_loss()[0], 120),
                            cmap="jet_r", vmin=40, vmax=90, shading="auto")
image_ax.plot(track_range / 1e3, track_depth, "w--", lw=1, label="glider track")
image_ax.invert_yaxis()
image_ax.set(xlabel="Range (km)", ylabel="Depth (m)", title=spec["title"])
image_ax.legend(loc="lower right")
fig.colorbar(image, ax=image_ax, label="TL (dB)")
track_ax.plot(along_track.ranges_m / 1e3, along_track.transmission_loss()[0])
track_ax.invert_yaxis()
track_ax.set(xlabel="Range (km)", ylabel="TL (dB)", title="Loss received along the glider track")

fig.tight_layout()
fig.savefig(OUT / "build_case.png", dpi=150)
plt.show()

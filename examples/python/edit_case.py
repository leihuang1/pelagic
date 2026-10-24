"""Edit a case as data: turn the Gulf of Mexico ray trace into a transmission-loss run.

    python examples/python/edit_case.py

A case converts to a plain dictionary in the JSON format (docs/bellhop/json.md).
Change it, and build a new case: Pelagic validates it again. The edited case is
saved as results/gulf-tl.json, which the CLI, MATLAB and Python can all run.
Writes results/edit_case.png.
"""
import json
from pathlib import Path

import matplotlib.pyplot as plt
import numpy as np

import pelagic as pg

CASES = Path(__file__).resolve().parent.parent / "cases"
OUT = Path("results")
OUT.mkdir(exist_ok=True)

spec = pg.bellhop.load(CASES / "bellhop/gulf/gulf.env").to_dict()  # reads .ssp and .bty too
print("original run:", spec["bellhop"]["run"], "with launch angles", spec["bellhop"]["angles"])

spec["title"] = "Gulf of Mexico: coherent transmission loss"
spec["bellhop"]["run"] = "coherent"
spec["bellhop"]["beam"] = "geometric_gaussian_cartesian"  # ray runs have no beam family
spec["bellhop"]["angles"] = {"from": -20, "to": 20, "count": 801}  # a grid, like the .env
spec["receivers"]["depth"] = np.linspace(0, 5000, 201)  # NumPy arrays are accepted too
spec["receivers"]["range"] = {"from": 0, "to": 120, "count": 601, "unit": "km"}

case = pg.bellhop.Case.from_dict(spec)
(OUT / "gulf-tl.json").write_text(json.dumps(case.to_dict(), indent=2))
result = case.run()

field = result.field
fig, ax = plt.subplots(figsize=(8, 4))
image = ax.pcolormesh(field.ranges_m / 1e3, field.depths_m, np.minimum(field.transmission_loss()[0], 120),
                      cmap="jet_r", vmin=60, vmax=110, shading="auto")
ax.invert_yaxis()
ax.set(xlabel="Range (km)", ylabel="Depth (m)", title=result.title)
fig.colorbar(image, label="Transmission loss (dB)")
fig.tight_layout()
fig.savefig(OUT / "edit_case.png", dpi=150)
print(f"wrote {OUT / 'gulf-tl.json'}")
plt.show()

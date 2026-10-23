"""Ray traces in three environments, coloured by how each ray interacts with the boundaries.

    python examples/python/rays.py

Cases: the Munk sound channel, a curved surface over a parabolic bottom (.ati and
.bty files), and the Gulf of Mexico with a range-dependent sound speed (.ssp) and
bathymetry (.bty). Writes results/rays.png.
"""
from pathlib import Path

import matplotlib.pyplot as plt

import pelagic as pg

CASES = Path(__file__).resolve().parent.parent / "cases"
OUT = Path("results")
OUT.mkdir(exist_ok=True)


def colour(ray):
    """Black: refracted only. Blue: surface bounces only. Red: hits the bottom."""
    if ray.bottom_bounces:
        return "tab:red"
    return "tab:blue" if ray.top_bounces else "black"


fig, axes = plt.subplots(3, 1, figsize=(8, 10))
for ax, name in zip(axes, ["munk-rays", "curved-boundaries", "gulf"]):
    case = pg.bellhop.load(CASES / f"bellhop/{name}/{name}.env")
    result = case.run()
    for ray in result.rays:
        # points columns: range (m), depth (m), travel time, attenuation time, amplitude, phase
        ax.plot(ray.points[:, 0] / 1e3, ray.points[:, 1], color=colour(ray), lw=0.6)

    # Boundary shapes from .ati/.bty files are part of the case as plain data.
    spec = case.to_dict()
    for boundary in ("top_boundary", "bottom_boundary"):
        shape = spec[boundary].get("shape")
        if shape:
            points = shape["points"]
            ax.plot([p["range_m"] / 1e3 for p in points], [p["depth_m"] for p in points],
                    color="saddlebrown", lw=2)
    ax.invert_yaxis()
    ax.set(xlabel="Range (km)", ylabel="Depth (m)", title=f"{result.title} ({len(result.rays)} rays)")

fig.tight_layout()
fig.savefig(OUT / "rays.png", dpi=150)
plt.show()

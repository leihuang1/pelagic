"""Eigenrays: the rays that connect a source to one receiver, and their travel times.

    python examples/python/eigenrays.py

Writes results/eigenrays.png and prints one line per eigenray.
"""
from pathlib import Path

import matplotlib.pyplot as plt

import pelagic as pg

CASES = Path(__file__).resolve().parent.parent / "cases"
OUT = Path("results")
OUT.mkdir(exist_ok=True)

result = pg.bellhop.load(CASES / "bellhop/munk-eigenrays/munk-eigenrays.env").run()
print(f"{len(result.eigenrays)} eigenrays to the receiver at "
      f"{result.eigenrays[0].range_m / 1e3:.0f} km, {result.eigenrays[0].depth_m:.0f} m")
print("launch angle   travel time   surface  bottom")

fig, ax = plt.subplots(figsize=(8, 4))
for ray in sorted(result.eigenrays, key=lambda ray: ray.points[-1, 2]):
    travel_time = ray.points[-1, 2]  # column 2 is travel time (s)
    print(f"{ray.launch_angle_degrees:10.2f}°  {travel_time:10.4f} s  {ray.top_bounces:7d}  {ray.bottom_bounces:6d}")
    ax.plot(ray.points[:, 0] / 1e3, ray.points[:, 1], lw=0.8)

ax.invert_yaxis()
ax.set(xlabel="Range (km)", ylabel="Depth (m)", title=result.title)
fig.tight_layout()
fig.savefig(OUT / "eigenrays.png", dpi=150)
plt.show()

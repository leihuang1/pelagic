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


def water(depth, speed):
    return {"depth_m": depth, "compressional_speed_mps": speed, "shear_speed_mps": 0.0,
            "density_kg_m3": 1000.0, "compressional_attenuation": 0.0, "shear_attenuation": 0.0}


spec = {
    "schema_version": 1,
    "title": "Shallow water: summer thermocline over sand",
    "frequency_hz": 1000.0,
    "sound_speed": {
        "interpolation": "c_linear",
        "attenuation_unit": "db_per_wavelength",
        "volume_attenuation": {"type": "thorp"},  # sea-water absorption
        "nominal_point_count": 0,
        "top_depth_m": 0.0,
        "bottom_depth_m": 120.0,
        "points": [water(0.0, 1530.0), water(20.0, 1528.0), water(40.0, 1505.0), water(120.0, 1500.0)],
    },
    "top_boundary": {"roughness_m": 0.0, "condition": {"type": "vacuum"}},
    "bottom_boundary": {
        "roughness_m": 0.0,
        "condition": {"type": "acousto_elastic", "half_space": {
            "depth_m": 120.0, "compressional_speed_mps": 1650.0, "shear_speed_mps": 0.0,
            "density_kg_m3": 1900.0, "compressional_attenuation": 0.8, "shear_attenuation": 0.0}},
    },
    "positions": {
        "source_depths_m": [30.0],
        "receiver_depths_m": np.linspace(0, 120, 241),
        "receiver_ranges_m": np.linspace(0, 10e3, 501),
    },
    "run": {"kind": "coherent", "beam_family": "geometric_gaussian_cartesian",
            "source_geometry": "point", "receiver_grid": "rectilinear", "beam_shift": False},
    "trace": {"launch_angles_degrees": np.linspace(-60, 60, 1201), "step_m": 0.0,
              "max_depth_m": 121.0, "max_range_m": 10.1e3},
}

case = pg.bellhop.Case.from_dict(spec)
(OUT / "shallow-water.json").write_text(json.dumps(case.to_dict(), indent=2))
field = case.run().field

# The same environment, sampled only along a glider track: paired ranges and depths.
track_range = np.linspace(200, 10e3, 400)
track_depth = 10 + 100 * np.abs((track_range / 2000) % 2 - 1)  # saw-tooth between 10 and 110 m
glider = dict(spec, run=dict(spec["run"], receiver_grid="irregular"),
              positions=dict(spec["positions"], receiver_ranges_m=track_range, receiver_depths_m=track_depth))
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

"""Read the command line's HDF5 results with h5py.

    pelagic bellhop run examples/cases/bellhop/munk-tl/munk-tl.json --format hdf5 --output results/cli-munk
    pelagic kraken run examples/cases/kraken/munk-modes/munk-modes.json --format hdf5 --output results/cli-modes
    python examples/cli/read_hdf5.py results/cli-munk/munk-tl.h5 results/cli-modes/munk-modes.h5

Needs h5py and matplotlib. The layouts are documented in docs/guide/bellhop-hdf5.md
and docs/guide/kraken-hdf5.md. Writes results/read_hdf5.png.
"""
import sys
from pathlib import Path

import h5py
import matplotlib.pyplot as plt
import numpy as np


def bellhop_field(path):
    """BELLHOP schema v3 stores one row per receiver: depth varies fastest, then range, then source."""
    with h5py.File(path, "r") as h5:
        print(f"{path}: {h5.attrs['title']} (schema {h5.attrs['schema_version']}, {h5.attrs['implementation']})")
        field = h5["field"]
        pressure = field["pressure_real"][:] + 1j * field["pressure_imaginary"][:]
        depth, range_ = field["receiver_depth_m"][:], field["receiver_range_m"][:]
        sources = len(field["source_depth_m"])
    depths = np.unique(depth)
    ranges = np.unique(range_)
    pressure = pressure.reshape(sources, len(ranges), len(depths)).transpose(0, 2, 1)
    return depths, ranges, pressure  # pressure: [source, depth, range]


def kraken_field(path, frequency=0):
    """KRAKEN schema v1 stores each frequency's field as [source, depth, range]."""
    with h5py.File(path, "r") as h5:
        print(f"{path}: {h5.attrs['title']} ({h5.attrs['mode_count']} modes, solver {h5.attrs['solver']})")
        block = h5[f"frequencies/{frequency}/field"]
        pressure = block["pressure_real"][:] + 1j * block["pressure_imaginary"][:]
        return block["receiver_depth_m"][:], block["receiver_range_m"][:], pressure


def main(paths):
    fig, axes = plt.subplots(len(paths), 1, figsize=(8, 3.5 * len(paths)), squeeze=False)
    for ax, path in zip(axes[:, 0], paths):
        with h5py.File(path, "r") as h5:
            reader = kraken_field if h5.attrs.get("schema_name") == "kraken" else bellhop_field
        depths, ranges, pressure = reader(path)
        with np.errstate(divide="ignore"):
            loss = np.minimum(-20 * np.log10(abs(pressure[0])), 120)
        image = ax.pcolormesh(ranges / 1e3, depths, loss, cmap="jet_r", vmin=60, vmax=110, shading="auto")
        ax.invert_yaxis()
        ax.set(xlabel="Range (km)", ylabel="Depth (m)", title=Path(path).name)
        fig.colorbar(image, ax=ax, label="TL (dB)")
    fig.tight_layout()
    Path("results").mkdir(exist_ok=True)
    fig.savefig("results/read_hdf5.png", dpi=130)
    plt.show()


if __name__ == "__main__":
    if len(sys.argv) < 2:
        sys.exit(__doc__)
    main(sys.argv[1:])
